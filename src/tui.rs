use std::io;
use std::path::PathBuf;
use std::time::Duration;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use tokio::sync::mpsc::{self, Receiver};

use crate::config::{Config, Account, AccountType, MicrosoftAuth};
use crate::api::{ApiClient, VersionBrief, VersionDetails, VersionManifest};
use crate::downloader::{Downloader, ProgressUpdate};
use crate::launcher::Launcher;
use crate::instance::{Instance, InstanceMod};

#[derive(Copy, Clone, Debug, PartialEq)]
enum Tab {
    Dashboard,
    Instances,
    Accounts,
    Settings,
}

enum MicrosoftAuthUpdate {
    Code { user_code: String, verification_uri: String },
    Success(Account),
    Error(String),
}

enum AppState {
    Normal,
    AddOfflineAccount,
    AddMicrosoftAccount {
        user_code: String,
        verification_uri: String,
        rx: Receiver<MicrosoftAuthUpdate>,
    },
    EditingSetting {
        field_idx: usize,
        input_value: String,
    },
    SelectInstanceFieldToEdit {
        instance_idx: usize,
    },
    EditingInstanceSetting {
        instance_idx: usize,
        field_idx: usize,
        input_value: String,
    },
    GameRunning {
        instance_id: String,
        instance_name: String,
        logs: Vec<String>,
        rx_logs: Receiver<String>,
        rx_status: Receiver<Result<(), String>>,
        status: Option<Result<(), String>>,
        crash_analysis: Option<crate::crash_analyzer::CrashAnalysis>,
        scroll_offset: usize,
        auto_scroll: bool,
    },
    Downloading {
        completed: usize,
        total: usize,
        current_file: String,
        message: String,
        logs: Vec<String>,
        rx: Receiver<ProgressUpdate>,
        version_details: VersionDetails,
    },
    CreatingInstanceName,
    CreatingInstanceVersion {
        id: String,
        name: String,
    },
    SyncingInstanceMods {
        completed: usize,
        total: usize,
        current_file: String,
        message: String,
        logs: Vec<String>,
        rx: Receiver<ProgressUpdate>,
    },
    BackupsMenu {
        instance_idx: usize,
        backups: Vec<String>,
        backups_list_state: ListState,
    },
    ChoosingModLoader {
        instance_id: String,
        instance_name: String,
        game_version: String,
        loader_options: Vec<String>,
        loader_list_state: ListState,
    },
    ModManager {
        instance_idx: usize,
        mods: Vec<InstanceMod>,
        mod_list_state: ListState,
    },
    ImportingModpackPath,
    ImportingModpackId {
        path: std::path::PathBuf,
    },
    ImportingModpackProgress {
        completed: usize,
        total: usize,
        current_file: String,
        message: String,
        logs: Vec<String>,
        rx: Receiver<ProgressUpdate>,
    },
}

pub struct App {
    config: Config,
    api_client: ApiClient,
    active_tab: Tab,
    state: AppState,
    version_manifest: Option<VersionManifest>,
    local_versions: Vec<String>,
    
    // UI selection states
    version_list_state: ListState,
    filtered_version_briefs: Vec<VersionBrief>,
    version_search_query: String,
    filter_releases: bool,
    filter_snapshots: bool,
    
    account_list_state: ListState,
    settings_list_state: ListState,
    
    // User message display
    status_message: Option<(String, bool)>, // (message, is_error)

    // Instances
    instances: Vec<Instance>,
    instances_list_state: ListState,
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        let launcher = Launcher::new(config.clone());
        let local_versions = launcher.get_available_local_versions();
        let instances = Instance::load_all(&config.game_dir);
        
        let mut app = Self {
            config,
            api_client: ApiClient::new(),
            active_tab: Tab::Dashboard,
            state: AppState::Normal,
            version_manifest: None,
            local_versions,
            version_list_state: ListState::default(),
            filtered_version_briefs: Vec::new(),
            version_search_query: String::new(),
            filter_releases: true,
            filter_snapshots: true,
            account_list_state: ListState::default(),
            settings_list_state: ListState::default(),
            status_message: None,
            instances,
            instances_list_state: ListState::default(),
        };

        app.select_active_instance_in_list();
        app
    }

    fn select_active_instance_in_list(&mut self) {
        if let Some(ref active_id) = self.config.active_instance {
            if let Some(idx) = self.instances.iter().position(|inst| &inst.id == active_id) {
                self.instances_list_state.select(Some(idx));
            } else if !self.instances.is_empty() {
                self.instances_list_state.select(Some(0));
            }
        } else if !self.instances.is_empty() {
            self.instances_list_state.select(Some(0));
        }
    }

    fn refresh_instances(&mut self) {
        self.instances = Instance::load_all(&self.config.game_dir);
        self.select_active_instance_in_list();
    }

    async fn fetch_manifest(&mut self) {
        self.status_message = Some(("Fetching version manifest...".to_string(), false));
        match self.api_client.fetch_version_manifest().await {
            Ok(manifest) => {
                self.version_manifest = Some(manifest);
                self.filter_versions();
                self.status_message = None;
            }
            Err(e) => {
                self.status_message = Some((format!("Failed to fetch manifest: {}", e), true));
            }
        }
    }

    fn filter_versions(&mut self) {
        if let Some(ref manifest) = self.version_manifest {
            let query = self.version_search_query.to_lowercase();
            self.filtered_version_briefs = manifest.versions.iter()
                .filter(|v| {
                    let matches_search = v.id.to_lowercase().contains(&query) || v.r#type.to_lowercase().contains(&query);
                    let matches_type = (self.filter_releases && v.r#type == "release") || (self.filter_snapshots && v.r#type == "snapshot");
                    matches_search && matches_type
                })
                .cloned()
                .collect();
            
            // Keep selected index valid
            let len = self.filtered_version_briefs.len();
            if len == 0 {
                self.version_list_state.select(None);
            } else {
                let selected = self.version_list_state.selected().unwrap_or(0);
                self.version_list_state.select(Some(selected.min(len - 1)));
            }
        }
    }

    fn active_account_desc(&self) -> String {
        if let Some(acc) = self.config.get_active_account() {
            let auth_type = match acc.account_type {
                AccountType::Offline => "Offline",
                AccountType::Microsoft => "Microsoft",
            };
            format!("{} ({})", acc.username, auth_type)
        } else {
            "None (Please add an account)".to_string()
        }
    }

    fn start_offline_account_flow(&mut self) {
        self.state = AppState::AddOfflineAccount;
        self.status_message = None;
    }

    fn start_microsoft_account_flow(&mut self) {
        let (tx, rx) = mpsc::channel::<MicrosoftAuthUpdate>(10);
        let api = ApiClient::new();

        tokio::spawn(async move {
            match api.request_device_code().await {
                Ok(device_res) => {
                    let _ = tx.send(MicrosoftAuthUpdate::Code {
                        user_code: device_res.user_code,
                        verification_uri: device_res.verification_uri,
                    }).await;
                    
                    let poll_interval = Duration::from_secs(device_res.interval.max(1));
                    let mut expires_in = device_res.expires_in;
                    
                    loop {
                        tokio::time::sleep(poll_interval).await;
                        if expires_in < poll_interval.as_secs() {
                            let _ = tx.send(MicrosoftAuthUpdate::Error("Authentication timed out. Please try again.".to_string())).await;
                            break;
                        }
                        expires_in -= poll_interval.as_secs();

                        match api.poll_token(&device_res.device_code).await {
                            Ok(Some(token_res)) => {
                                match api.login_with_microsoft(&token_res.access_token).await {
                                    Ok(mc_res) => {
                                        match api.fetch_profile(&mc_res.access_token).await {
                                            Ok(profile) => {
                                                let account = Account {
                                                    uuid: profile.id,
                                                    username: profile.name,
                                                    account_type: AccountType::Microsoft,
                                                    microsoft_auth: Some(MicrosoftAuth {
                                                        access_token: mc_res.access_token,
                                                        refresh_token: token_res.refresh_token,
                                                        expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(mc_res.expires_in as i64)),
                                                    }),
                                                };
                                                let _ = tx.send(MicrosoftAuthUpdate::Success(account)).await;
                                                break;
                                            }
                                            Err(e) => {
                                                let _ = tx.send(MicrosoftAuthUpdate::Error(format!("Profile fetch failed: {}", e))).await;
                                                break;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(MicrosoftAuthUpdate::Error(format!("Minecraft login failed: {}", e))).await;
                                        break;
                                    }
                                }
                            }
                            Ok(None) => {
                                continue;
                            }
                            Err(e) => {
                                let _ = tx.send(MicrosoftAuthUpdate::Error(format!("Auth polling error: {}", e))).await;
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(MicrosoftAuthUpdate::Error(format!("Failed to request Microsoft device code: {}", e))).await;
                }
            }
        });

        self.state = AppState::AddMicrosoftAccount {
            user_code: "LOADING...".to_string(),
            verification_uri: "...".to_string(),
            rx,
        };
        self.status_message = Some(("Requesting login code from Microsoft...".to_string(), false));
    }

    async fn handle_background_tasks(&mut self) {
        // 1. Process Microsoft Auth state changes
        let mut auth_update_state = None;
        if let AppState::AddMicrosoftAccount { ref mut user_code, ref mut verification_uri, ref mut rx } = self.state {
            while let Ok(update) = rx.try_recv() {
                match update {
                    MicrosoftAuthUpdate::Code { user_code: code, verification_uri: uri } => {
                        *user_code = code;
                        *verification_uri = uri;
                        self.status_message = None;
                    }
                    MicrosoftAuthUpdate::Success(account) => {
                        self.config.add_account(account);
                        auth_update_state = Some(Ok(()));
                    }
                    MicrosoftAuthUpdate::Error(e) => {
                        auth_update_state = Some(Err(e));
                    }
                }
            }
        }

        if let Some(res) = auth_update_state {
            self.state = AppState::Normal;
            match res {
                Ok(_) => {
                    self.status_message = Some(("Successfully logged in with Microsoft!".to_string(), false));
                    self.active_tab = Tab::Accounts;
                }
                Err(e) => {
                    self.status_message = Some((e, true));
                }
            }
        }

        // 2. Process Downloader progress state changes
        let mut download_finished_state = None;
        if let AppState::Downloading { ref mut completed, ref mut total, ref mut current_file, ref mut message, ref mut logs, ref mut rx, ref version_details } = self.state {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Started { total: t, message: msg } => {
                        *total = t;
                        *completed = 0;
                        *message = msg.clone();
                        logs.push(format!("Task: {}", msg));
                    }
                    ProgressUpdate::Progress { completed: c, total: t, current_file: f } => {
                        *completed = c;
                        *total = t;
                        *current_file = f.clone();
                        if c % 10 == 0 || c == t {
                            logs.push(format!("[{}/{}] Downloaded {}", c, t, f));
                        }
                    }
                    ProgressUpdate::Message(msg) => {
                        *message = msg.clone();
                        logs.push(msg);
                    }
                     ProgressUpdate::Finished => {
                         let version_id = version_details.id();
                         download_finished_state = Some(Ok(version_id));
                    }
                    ProgressUpdate::Error(e) => {
                        download_finished_state = Some(Err(e));
                    }
                }
            }
        }

        if let Some(res) = download_finished_state {
            self.state = AppState::Normal;
            match res {
                Ok(version_id) => {
                    let launcher = Launcher::new(self.config.clone());
                    self.local_versions = launcher.get_available_local_versions();
                    self.status_message = Some((format!("Successfully downloaded Minecraft version {}!", version_id), false));
                }
                Err(e) => {
                    self.status_message = Some((format!("Download failed: {}", e), true));
                }
            }
        }

        // 3. Process Mod Sync state changes
        let mut sync_finished_state = None;
        if let AppState::SyncingInstanceMods { ref mut completed, ref mut total, ref mut current_file, ref mut message, ref mut logs, ref mut rx } = self.state {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Started { total: t, message: msg } => {
                        *total = t;
                        *completed = 0;
                        *message = msg.clone();
                        logs.push(format!("Sync: {}", msg));
                    }
                    ProgressUpdate::Progress { completed: c, total: t, current_file: f } => {
                        *completed = c;
                        *total = t;
                        *current_file = f.clone();
                        if c % 5 == 0 || c == t {
                            logs.push(format!("[{}/{}] Synced {}", c, t, f));
                        }
                    }
                    ProgressUpdate::Message(msg) => {
                        *message = msg.clone();
                        logs.push(msg);
                    }
                    ProgressUpdate::Finished => {
                        sync_finished_state = Some(Ok(()));
                    }
                    ProgressUpdate::Error(e) => {
                        sync_finished_state = Some(Err(e));
                    }
                }
            }
        }

        if let Some(res) = sync_finished_state {
            self.state = AppState::Normal;
            match res {
                Ok(_) => {
                    self.status_message = Some(("Mods synchronized successfully!".to_string(), false));
                }
                Err(e) => {
                    self.status_message = Some((format!("Mod sync failed: {}", e), true));
                }
            }
            self.refresh_instances();
        }

        // 3b. Process Modpack Import state changes
        let mut import_finished_state = None;
        if let AppState::ImportingModpackProgress { ref mut completed, ref mut total, ref mut current_file, ref mut message, ref mut logs, ref mut rx } = self.state {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Started { total: t, message: msg } => {
                        *total = t;
                        *completed = 0;
                        *message = msg.clone();
                        logs.push(format!("Import: {}", msg));
                    }
                    ProgressUpdate::Progress { completed: c, total: t, current_file: f } => {
                        *completed = c;
                        *total = t;
                        *current_file = f.clone();
                        if c % 5 == 0 || c == t {
                            logs.push(format!("[{}/{}] Imported {}", c, t, f));
                        }
                    }
                    ProgressUpdate::Message(msg) => {
                        *message = msg.clone();
                        logs.push(msg);
                    }
                    ProgressUpdate::Finished => {
                        import_finished_state = Some(Ok(()));
                    }
                    ProgressUpdate::Error(e) => {
                        import_finished_state = Some(Err(e));
                    }
                }
            }
        }

        if let Some(res) = import_finished_state {
            self.state = AppState::Normal;
            match res {
                Ok(_) => {
                    self.status_message = Some(("Modpack imported successfully!".to_string(), false));
                }
                Err(e) => {
                    self.status_message = Some((format!("Import failed: {}", e), true));
                }
            }
            self.refresh_instances();
        }

        // 4. Process Game Log streams and termination status
        if let AppState::GameRunning { ref mut logs, ref mut rx_logs, ref mut rx_status, ref mut status, ref mut crash_analysis, ref mut scroll_offset, auto_scroll, ref instance_id, .. } = self.state {
            while let Ok(line) = rx_logs.try_recv() {
                logs.push(line);
                if auto_scroll {
                    *scroll_offset = logs.len();
                }
            }
            while let Ok(res) = rx_status.try_recv() {
                let res_val: Result<(), String> = res;
                *status = Some(res_val.clone());
                if let Err(_) = res_val {
                    let instance_path = self.config.game_dir.join("instances").join(instance_id);
                    let latest_log_content = logs.join("\n");
                    *crash_analysis = crate::crash_analyzer::analyze_crash(&instance_path, &latest_log_content);
                }
            }
        }
    }

    async fn start_download_flow(&mut self, brief: VersionBrief) {
        self.status_message = Some(("Initializing downloader...".to_string(), false));
        match self.api_client.fetch_version_details(&brief.url).await {
            Ok(details) => {
                let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                let downloader = Downloader::new(tx);
                let game_dir = self.config.game_dir.clone();
                let details_clone = details.clone();

                let details_id = details.id();
                let details_json_path = game_dir
                    .join("versions")
                    .join(&details_id)
                    .join(format!("{}.json", details_id));
                if let Some(parent) = details_json_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(content) = serde_json::to_string_pretty(&details) {
                    let _ = std::fs::write(&details_json_path, content);
                }

                tokio::spawn(async move {
                    let _ = downloader.download_version(&game_dir, &details_clone).await;
                });

                self.state = AppState::Downloading {
                    completed: 0,
                    total: 100,
                    current_file: String::new(),
                    message: "Starting download...".to_string(),
                    logs: Vec::new(),
                    rx,
                    version_details: details,
                };
                self.status_message = None;
            }
            Err(e) => {
                self.status_message = Some((format!("Failed to retrieve version details: {}", e), true));
            }
        }
    }

    async fn run_minecraft(&mut self) {
        let active_id = match &self.config.active_instance {
            Some(id) => id,
            None => {
                self.status_message = Some(("No active instance selected. Please select an instance in the 'Instances' tab.".to_string(), true));
                return;
            }
        };

        let instance = match Instance::load(active_id, self.config.game_dir.join("instances").join(active_id)) {
            Ok(inst) => inst,
            Err(e) => {
                self.status_message = Some((format!("Failed to load active instance: {}", e), true));
                return;
            }
        };

        let account = match self.config.get_active_account() {
            Some(acc) => acc.clone(),
            None => {
                self.status_message = Some(("No active account found. Please configure an account in the 'Accounts' tab.".to_string(), true));
                return;
            }
        };

        let (tx_logs, rx_logs) = mpsc::channel::<String>(1000);
        let (tx_status, rx_status) = mpsc::channel::<Result<(), String>>(1);

        let launcher = Launcher::new(self.config.clone());
        let inst_clone = instance.clone();
        let acc_clone = account.clone();

        tokio::spawn(async move {
            let res = launcher.launch_with_logs(&inst_clone, &acc_clone, Some(tx_logs)).await;
            let _ = tx_status.send(res).await;
        });

        self.state = AppState::GameRunning {
            instance_id: instance.id.clone(),
            instance_name: instance.config.name.clone(),
            logs: Vec::new(),
            rx_logs,
            rx_status,
            status: None,
            crash_analysis: None,
            scroll_offset: 0,
            auto_scroll: true,
        };
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let size = f.size();

        let bg_color = Color::Rgb(15, 17, 26); 
        let border_color = Color::Rgb(86, 73, 150); 
        let select_color = Color::Rgb(142, 68, 173); 

        if let AppState::GameRunning { .. } = self.state {
            self.draw_game_logs(f, size, select_color, border_color);
            return;
        }
        let text_color = Color::Rgb(220, 222, 235); 
        let active_color = Color::Rgb(46, 204, 113); 

        let main_block = Block::default()
            .bg(bg_color)
            .style(Style::default().fg(text_color));
        f.render_widget(main_block, size);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), 
                Constraint::Min(5),    
                Constraint::Length(3), 
            ])
            .split(size);

        let logo = format!(" ✦ MineCLI Terminal Launcher ✦ ");
        let logo_p = Paragraph::new(logo)
            .style(Style::default().fg(Color::Rgb(155, 89, 182)).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(border_color)));
        f.render_widget(logo_p, chunks[0]);

        if let Some((ref msg, is_error)) = self.status_message {
            let color = if is_error { Color::Red } else { Color::Cyan };
            let status_rect = Rect {
                x: chunks[0].x + chunks[0].width.saturating_sub(45),
                y: chunks[0].y + 1,
                width: 42,
                height: 1,
            };
            let status_p = Paragraph::new(format!("● {}", msg))
                .style(Style::default().fg(color).add_modifier(Modifier::ITALIC))
                .wrap(Wrap { trim: true });
            f.render_widget(status_p, status_rect);
        }

        let main_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22), 
                Constraint::Min(20),    
            ])
            .split(chunks[1]);

        self.draw_sidebar(f, main_layout[0], border_color, select_color);

        match self.active_tab {
            Tab::Dashboard => self.draw_dashboard(f, main_layout[1], border_color, active_color),
            Tab::Instances => self.draw_instances(f, main_layout[1], border_color, select_color),
            Tab::Accounts => self.draw_accounts(f, main_layout[1], border_color, select_color),
            Tab::Settings => self.draw_settings(f, main_layout[1], border_color, select_color),
        }

        self.draw_footer(f, chunks[2], border_color);
        self.draw_overlays(f, size, select_color, border_color);
    }

    fn draw_sidebar(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
        let menu_items = vec![
            "[D] Dashboard",
            "[I] Instances",
            "[A] Accounts",
            "[S] Settings",
        ];

        let list_items: Vec<ListItem> = menu_items.iter().enumerate().map(|(idx, item)| {
            let tab_match = match idx {
                0 => self.active_tab == Tab::Dashboard,
                1 => self.active_tab == Tab::Instances,
                2 => self.active_tab == Tab::Accounts,
                3 => self.active_tab == Tab::Settings,
                _ => false,
            };

            let style = if tab_match {
                Style::default().fg(select_color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(150, 150, 160))
            };

            ListItem::new(item.to_string()).style(style)
        }).collect();

        let menu_list = List::new(list_items)
            .block(Block::default()
                .title(" Menu ")
                .borders(Borders::RIGHT)
                .border_style(Style::default().fg(border_color)));
        
        f.render_widget(menu_list, rect);
    }

    fn draw_game_logs(&self, f: &mut ratatui::Frame, size: Rect, _select_color: Color, border_color: Color) {
        if let AppState::GameRunning { ref instance_name, ref logs, ref status, ref crash_analysis, scroll_offset, auto_scroll, .. } = self.state {
            f.render_widget(Clear, size);
            let bg_block = Block::default().bg(Color::Rgb(15, 17, 26));
            f.render_widget(bg_block, size);

            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Title
                    Constraint::Min(5),    // Log content & crash card
                    Constraint::Length(3), // Footer
                ])
                .split(size);

            // Title
            let status_text = match status {
                None => Span::styled(" RUNNING ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Some(Ok(_)) => Span::styled(" EXITED SUCCESSFULLY ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Some(Err(_)) => Span::styled(" CRASHED ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            };
            let title_line = Line::from(vec![
                Span::raw(" Game Session: "),
                Span::styled(instance_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw(" | Status: "),
                status_text,
            ]);
            let title_p = Paragraph::new(title_line)
                .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(border_color)))
                .style(Style::default().fg(Color::Rgb(200, 200, 220)));
            f.render_widget(title_p, main_chunks[0]);

            // Layout center area
            let (logs_area, crash_area) = if crash_analysis.is_some() {
                let center_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Percentage(55),
                        Constraint::Percentage(45),
                    ])
                    .split(main_chunks[1]);
                (center_chunks[0], Some(center_chunks[1]))
            } else {
                (main_chunks[1], None)
            };

            // Render log viewer
            let logs_block = Block::default()
                .title(format!(" Game Logs (Auto-scroll: {}) ", if auto_scroll { "ON" } else { "OFF (Press [End] to lock)" }))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if status.is_none() { Color::Green } else { border_color }));
            
            // Format log lines to show in paragraph
            let display_height = logs_area.height.saturating_sub(2) as usize;
            
            // Slice logs based on scroll offset
            let scroll_offset = if auto_scroll {
                logs.len().saturating_sub(display_height)
            } else {
                scroll_offset.min(logs.len().saturating_sub(1))
            };

            let start = scroll_offset;
            let end = (start + display_height).min(logs.len());
            let sliced_logs = if start < logs.len() {
                &logs[start..end]
            } else {
                &[]
            };

            let log_lines: Vec<Line> = sliced_logs.iter().map(|line| {
                let fg = if line.contains("[ERROR]") || line.contains("[stderr]") || line.contains("Error") || line.contains("Exception") {
                    Color::Red
                } else if line.contains("[WARN]") || line.contains("Warning") {
                    Color::Yellow
                } else if line.contains("[INFO]") {
                    Color::Rgb(180, 180, 200)
                } else {
                    Color::Rgb(140, 140, 150)
                };
                Line::from(Span::styled(line, Style::default().fg(fg)))
            }).collect();

            let logs_p = Paragraph::new(log_lines).block(logs_block);
            f.render_widget(logs_p, logs_area);

            // Render crash diagnostics card if present
            if let Some(crash) = crash_analysis {
                if let Some(area) = crash_area {
                    let card_block = Block::default()
                        .title(" Crash Diagnostics ")
                        .borders(Borders::ALL)
                        .border_type(BorderType::Double)
                        .border_style(Style::default().fg(Color::Red));

                    let mut card_text = vec![
                        Line::from(Span::styled(&crash.title, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
                        Line::from(""),
                        Line::from(Span::styled("Description:", Style::default().fg(Color::Rgb(200, 200, 210)).add_modifier(Modifier::BOLD))),
                    ];
                    
                    card_text.push(Line::from(Span::raw(&crash.description)));
                    card_text.push(Line::from(""));
                    card_text.push(Line::from(Span::styled("Suggested Solutions:", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))));
                    
                    for sol in &crash.possible_solutions {
                        card_text.push(Line::from(Span::styled(format!("• {}", sol), Style::default().fg(Color::White))));
                    }

                    let card_p = Paragraph::new(card_text)
                        .block(card_block)
                        .wrap(Wrap { trim: true });
                    f.render_widget(card_p, area);
                }
            }

            // Render footer
            let footer_text = match status {
                None => {
                    "● Game is running. Close Minecraft to return to launcher.  |  [Up/Down]: Scroll Logs"
                }
                Some(Ok(_)) => {
                    "✔ Game closed successfully. Press [Esc] to return to menu.  |  [Up/Down]: Scroll Logs"
                }
                Some(Err(_)) => {
                    "❌ Game crashed! See diagnostics on the right. Press [Esc] to return to menu.  |  [Up/Down]: Scroll Logs"
                }
            };
            let footer_p = Paragraph::new(footer_text)
                .style(Style::default().fg(Color::Rgb(150, 150, 160)))
                .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(border_color)))
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(footer_p, main_chunks[2]);
        }
    }

    fn draw_dashboard(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, active_color: Color) {
        let main_block = Block::default()
            .title(" Launcher Dashboard ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        
        let inner = main_block.inner(rect);
        f.render_widget(main_block, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), 
                Constraint::Length(6), 
                Constraint::Min(4),    
            ])
            .split(inner);

        let active_instance_str = self.config.active_instance.clone().unwrap_or_else(|| "None".to_string());
        let account_str = self.active_account_desc();
        
        let launch_btn_text = if self.config.active_instance.is_some() && self.config.active_account_uuid.is_some() {
            format!(" ► LAUNCH INSTANCE '{}' (Press Enter or 'L') ◄ ", active_instance_str)
        } else {
            " ⚠️ Setup Required (Select instance & account) ⚠️ ".to_string()
        };

        let launch_btn_style = if self.config.active_instance.is_some() && self.config.active_account_uuid.is_some() {
            Style::default().fg(active_color).add_modifier(Modifier::BOLD).bg(Color::Rgb(20, 40, 25))
        } else {
            Style::default().fg(Color::Rgb(230, 126, 34)).add_modifier(Modifier::BOLD)
        };

        let launch_btn = Paragraph::new(format!("\n{}", launch_btn_text))
            .alignment(ratatui::layout::Alignment::Center)
            .style(launch_btn_style)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(active_color)));
        
        f.render_widget(launch_btn, chunks[0]);

        let details_text = vec![
            Line::from(vec![Span::raw("Active Player:   "), Span::styled(account_str, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]),
            Line::from(vec![Span::raw("Active Instance: "), Span::styled(active_instance_str, Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))]),
            Line::from(vec![Span::raw("Game Directory:  "), Span::styled(self.config.game_dir.to_string_lossy().to_string(), Style::default().fg(Color::Yellow))]),
            Line::from(vec![Span::raw("JVM Memory:      "), Span::styled(self.config.jvm_args.join(" "), Style::default().fg(Color::Yellow))]),
        ];
        let details = Paragraph::new(details_text)
            .block(Block::default().title(" Active Session Details ").borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        
        f.render_widget(details, chunks[1]);

        let ascii_art = r#"
  __  __ _            _____ _      _____ 
 |  \/  (_)          / ____| |    |_   _|
 | \  / |_ _ __   ___| |    | |      | |  
 | |\/| | | '_ \ / _ \ |    | |      | |  
 | |  | | | | | |  __/ |____| |____ _| |_ 
 |_|  |_|_|_| |_|\___|\_____|______|_____|
        "#;
        let welcome_text = format!("{}\n\nWelcome to MineCLI. Select an instance and account from the tabs to get started.\nPress 'q' at any time to quit.", ascii_art);
        let welcome = Paragraph::new(welcome_text)
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Rgb(130, 130, 150)))
            .wrap(Wrap { trim: false });
        f.render_widget(welcome, chunks[2]);
    }

    fn draw_instances(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
        let main_block = Block::default()
            .title(" Minecraft Instance Manager ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        
        let inner = main_block.inner(rect);
        f.render_widget(main_block, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    
                Constraint::Length(5), 
            ])
            .split(inner);

        let list_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(chunks[0]);

        let list_items: Vec<ListItem> = self.instances.iter().map(|inst| {
            let is_active = self.config.active_instance.as_ref() == Some(&inst.id);
            
            let status_span = if is_active {
                Span::styled(" [ACTIVE] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };

            let item_line = Line::from(vec![
                Span::styled(format!(" {:<18}", inst.config.name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" ID: {:<12}", inst.id), Style::default().fg(Color::Rgb(160, 160, 170))),
                Span::styled(format!(" Version: {:<10}", inst.config.version), Style::default().fg(Color::Cyan)),
                status_span,
            ]);

            ListItem::new(item_line)
        }).collect();

        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title(" Available Instances ").border_style(Style::default().fg(border_color)))
            .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

        let mut state = self.instances_list_state.clone();
        f.render_stateful_widget(list, list_chunks[0], &mut state);

        // Highlighted instance detail panel on the right
        let selected_idx = self.instances_list_state.selected().unwrap_or(0);
        let highlighted_instance = self.instances.get(selected_idx);

        if let Some(inst) = highlighted_instance {
            let mods_count = inst.config.mods.as_ref().map(|m| m.len()).unwrap_or(0);
            let hooks_status = match (&inst.config.pre_launch, &inst.config.post_exit) {
                (Some(_), Some(_)) => "Pre & Post Hooks active",
                (Some(_), None) => "Pre Hook active",
                (None, Some(_)) => "Post Hook active",
                (None, None) => "None",
            };
            let jre_path_str = inst.config.java_path.as_deref().unwrap_or("None (Default)");
            let jre_ver_str = match inst.config.java_version {
                Some(v) => format!("Java {} (Forced)", v),
                None => "Auto-detect (Recommended)".to_string(),
            };

            let details_text = vec![
                Line::from(vec![
                    Span::styled("Name:              ", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(&inst.config.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("ID:                ", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(&inst.id, Style::default().fg(Color::White)),
                ]),
                Line::from(vec![
                    Span::styled("Minecraft Version: ", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(&inst.config.version, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("Java Runtime Path: ", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(jre_path_str, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::styled("Forced JRE Version:", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(jre_ver_str, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::styled("Declared Mods:     ", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(mods_count.to_string(), Style::default().fg(Color::Green)),
                ]),
                Line::from(vec![
                    Span::styled("Launch Hooks:      ", Style::default().fg(Color::Rgb(150, 150, 160))),
                    Span::styled(hooks_status, Style::default().fg(Color::Magenta)),
                ]),
            ];

            let details_p = Paragraph::new(details_text)
                .block(Block::default().title(" Selected Instance Config ").borders(Borders::ALL).border_style(Style::default().fg(border_color)));
            f.render_widget(details_p, list_chunks[1]);
        } else {
            let details_p = Paragraph::new("No instance highlighted")
                .block(Block::default().title(" Selected Instance Config ").borders(Borders::ALL).border_style(Style::default().fg(border_color)));
            f.render_widget(details_p, list_chunks[1]);
        }

        let help_text = vec![
            Line::from(vec![
                Span::styled(" [N]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), 
                Span::raw(" New Instance       "), 
                Span::styled("[D]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)), 
                Span::raw(" Delete Instance   "),
                Span::styled("[S]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)), 
                Span::raw(" Sync Mods"),
            ]),
            Line::from(vec![
                Span::styled(" [Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)), 
                Span::raw(" Set Active         "), 
                Span::styled("[B]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)), 
                Span::raw(" Backups & Snapshots"),
                Span::styled("   [E]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), 
                Span::raw(" Edit Settings"),
            ]),
            Line::from(vec![
                Span::styled(" [P]", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)), 
                Span::raw(" Import Modpack (.mrpack)"),
            ]),
        ];
        
        let help_p = Paragraph::new(help_text)
            .block(Block::default().title(" Instance Actions ").borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        f.render_widget(help_p, chunks[1]);
    }

    fn draw_accounts(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
        let main_block = Block::default()
            .title(" Account Profile Manager ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        
        let inner = main_block.inner(rect);
        f.render_widget(main_block, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    
                Constraint::Length(5), 
            ])
            .split(inner);

        let list_items: Vec<ListItem> = self.config.accounts.iter().map(|acc| {
            let is_active = self.config.active_account_uuid.as_deref() == Some(&acc.uuid);
            
            let status_span = if is_active {
                Span::styled(" [ACTIVE] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };

            let type_span = match acc.account_type {
                AccountType::Offline => Span::styled(" (Offline) ", Style::default().fg(Color::Yellow)),
                AccountType::Microsoft => Span::styled(" (Microsoft) ", Style::default().fg(Color::Cyan)),
            };

            let item_line = Line::from(vec![
                Span::styled(format!(" {:<20}", acc.username), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                type_span,
                Span::styled(format!(" UUID: {}", acc.uuid), Style::default().fg(Color::Rgb(120, 120, 130))),
                status_span,
            ]);

            ListItem::new(item_line)
        }).collect();

        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title(" Configured Accounts ").border_style(Style::default().fg(border_color)))
            .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

        let mut state = self.account_list_state.clone();
        f.render_stateful_widget(list, chunks[0], &mut state);

        let help_text = vec![
            Line::from(vec![Span::styled(" [A]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Add Account               "), Span::styled("[O]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)), Span::raw(" Add Offline Profile")]),
            Line::from(vec![Span::styled(" [Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)), Span::raw(" Select Profile Active     "), Span::styled("[X]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)), Span::raw(" Delete Profile")]),
        ];
        
        let help_p = Paragraph::new(help_text)
            .block(Block::default().title(" Profile Actions ").borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        f.render_widget(help_p, chunks[1]);
    }

    fn draw_settings(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
        let main_block = Block::default()
            .title(" Launcher Preferences ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        
        let inner = main_block.inner(rect);
        f.render_widget(main_block, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),    
                Constraint::Length(4), 
            ])
            .split(inner);

        let settings_list = vec![
            ("Game Directory", self.config.game_dir.to_string_lossy().to_string()),
            ("Java Executable Path", self.config.java_path.to_string_lossy().to_string()),
            ("JVM Extra Arguments", self.config.jvm_args.join(" ")),
        ];

        let list_items: Vec<ListItem> = settings_list.into_iter().map(|(label, val)| {
            ListItem::new(vec![
                Line::from(Span::styled(format!(" {}:", label), Style::default().fg(Color::Rgb(160, 160, 170)))),
                Line::from(Span::styled(format!("   {}", val), Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
            ])
        }).collect();

        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title(" Launcher Settings ").border_style(Style::default().fg(border_color)))
            .highlight_style(Style::default().bg(select_color).fg(Color::White));

        let mut state = self.settings_list_state.clone();
        f.render_stateful_widget(list, chunks[0], &mut state);

        let help_text = "Press [Enter] on a selected preference to edit its value.\nChanges are automatically saved to your configuration file.";
        let help_p = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        f.render_widget(help_p, chunks[1]);
    }

    fn draw_footer(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color) {
        let help_spans = vec![
            Span::styled(" [Tab] ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw("Switch Tab  "),
            Span::styled(" [L] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("Launch Minecraft  "),
            Span::styled(" [q] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw("Quit Launcher"),
        ];

        let footer_p = Paragraph::new(Line::from(help_spans))
            .alignment(ratatui::layout::Alignment::Center)
            .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(border_color)));
        
        f.render_widget(footer_p, rect);
    }

    fn draw_overlays(&self, f: &mut ratatui::Frame, size: Rect, select_color: Color, border_color: Color) {
        match self.state {
            AppState::Normal => {}
            
            AppState::AddOfflineAccount => {
                let area = self.get_centered_rect(50, 20, size);
                f.render_widget(Clear, area); 
                
                let block = Block::default()
                    .title(" Add Offline Account Profile ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(3), 
                        Constraint::Length(2), 
                    ])
                    .split(area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 }));

                f.render_widget(Paragraph::new("Enter desired username:"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Create, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::CreatingInstanceName => {
                let area = self.get_centered_rect(50, 20, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Create New Instance ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(3), 
                        Constraint::Length(2), 
                    ])
                    .split(area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 }));

                f.render_widget(Paragraph::new("Enter instance name/ID (e.g. survival-1.20):"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Next, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::ImportingModpackPath => {
                let area = self.get_centered_rect(60, 20, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Import Modpack (.mrpack) ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(3), 
                        Constraint::Length(2), 
                    ])
                    .split(area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 }));

                f.render_widget(Paragraph::new("Enter absolute path to .mrpack file:"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Next, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::ImportingModpackId { path: _ } => {
                let area = self.get_centered_rect(50, 20, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Confirm Instance ID ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner_layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(3), 
                        Constraint::Length(2), 
                    ])
                    .split(area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 }));

                f.render_widget(Paragraph::new("Enter desired instance ID/folder name:"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Import, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::CreatingInstanceVersion { ref id, ref name } => {
                let area = self.get_centered_rect(80, 80, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(format!(" Choose Version for '{}' (ID: {}) ", name, id))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3), 
                        Constraint::Min(4),    
                    ])
                    .split(inner);

                let release_status = if self.filter_releases { "● Releases (F1)" } else { "○ Releases (F1)" };
                let snapshot_status = if self.filter_snapshots { "● Snapshots (F2)" } else { "○ Snapshots (F2)" };
                
                let search_text = format!(" Search: {:<30} | {} | {}", self.version_search_query, release_status, snapshot_status);
                let search_p = Paragraph::new(search_text)
                    .style(Style::default().fg(Color::White))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
                f.render_widget(search_p, chunks[0]);

                let list_items: Vec<ListItem> = self.filtered_version_briefs.iter().map(|v| {
                    let is_local = self.local_versions.contains(&v.id);
                    let status_span = if is_local {
                        Span::styled(" [LOCAL] ", Style::default().fg(Color::Green))
                    } else {
                        Span::styled(" [CLOUD] ", Style::default().fg(Color::Yellow))
                    };

                    let item_line = Line::from(vec![
                        Span::styled(format!(" {:<18}", v.id), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" ({:<10})", v.r#type), Style::default().fg(Color::Rgb(160, 160, 170))),
                        status_span,
                    ]);

                    ListItem::new(item_line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Select Minecraft Version (Enter to Select, Esc to Cancel) ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                let mut state = self.version_list_state.clone();
                f.render_stateful_widget(list, chunks[1], &mut state);
            }

            AppState::AddMicrosoftAccount { ref user_code, ref verification_uri, .. } => {
                let area = self.get_centered_rect(65, 35, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(" Add Account ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 3, vertical: 2 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2), 
                        Constraint::Length(3), 
                        Constraint::Length(4), 
                        Constraint::Length(3), 
                        Constraint::Length(2), 
                    ])
                    .split(inner);

                f.render_widget(Paragraph::new("To log in, open a web browser on any device and navigate to the following page:").wrap(Wrap { trim: true }), chunks[0]);

                let url_p = Paragraph::new(verification_uri.as_str())
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED));
                f.render_widget(url_p, chunks[1]);

                let code_text = format!("\n  CODE: {}  ", user_code);
                let code_p = Paragraph::new(code_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Green).bg(Color::Rgb(20, 25, 20)).add_modifier(Modifier::BOLD))
                    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
                f.render_widget(code_p, chunks[2]);

                let poll_text = "Waiting for you to enter the code and authorize the login...\nThis window will close automatically upon successful authorization.";
                let poll_p = Paragraph::new(poll_text)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)))
                    .wrap(Wrap { trim: true });
                f.render_widget(poll_p, chunks[3]);

                let cancel_p = Paragraph::new("Press [Esc] to Cancel Login Flow")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Red));
                f.render_widget(cancel_p, chunks[4]);
            }

            AppState::EditingSetting { ref input_value, .. } => {
                let area = self.get_centered_rect(60, 20, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(" Edit Preference Value ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Length(3),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                f.render_widget(Paragraph::new("Type new preference value below and hit enter:"), chunks[0]);

                let input_p = Paragraph::new(input_value.as_str())
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, chunks[1]);

                let help = Paragraph::new("Press [Enter] to Save, [Esc] to Cancel")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)));
                f.render_widget(help, chunks[2]);
            }

            AppState::SelectInstanceFieldToEdit { instance_idx: _ } => {
                let area = self.get_centered_rect(55, 12, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(" Select Instance Setting to Edit ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                f.render_widget(Paragraph::new("Select a field to configure:"), chunks[0]);

                let opt1 = Line::from(vec![
                    Span::styled(" [1]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(" Java Executable Path (Override JRE path)"),
                ]);
                let opt2 = Line::from(vec![
                    Span::styled(" [2]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    Span::raw(" Java Version (Force specific major JRE version)"),
                ]);

                f.render_widget(Paragraph::new(opt1), chunks[2]);
                f.render_widget(Paragraph::new(opt2), chunks[3]);

                let help = Paragraph::new("Press [1] or [2] to select, [Esc] to Cancel")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)));
                f.render_widget(help, chunks[4]);
            }

            AppState::EditingInstanceSetting { field_idx, ref input_value, .. } => {
                let area = self.get_centered_rect(65, 12, size);
                f.render_widget(Clear, area);

                let field_name = match field_idx {
                    1 => "Java Executable Path",
                    2 => "Java JRE Version",
                    _ => "Instance Setting",
                };

                let block = Block::default()
                    .title(format!(" Edit {} ", field_name))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(2),
                        Constraint::Length(3),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                let desc = match field_idx {
                    1 => "Enter absolute path to 'java' binary (leave empty/type 'clear' to reset):",
                    2 => "Enter Java version (e.g. 8, 17, 21 or enter '0' to auto-detect):",
                    _ => "Enter setting value:",
                };
                f.render_widget(Paragraph::new(desc), chunks[0]);

                let input_p = Paragraph::new(input_value.as_str())
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, chunks[1]);

                let help = Paragraph::new("Press [Enter] to Save, [Esc] to Cancel")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)));
                f.render_widget(help, chunks[2]);
            }

            AppState::Downloading { completed, total, ref current_file, ref message, ref logs, .. } => {
                f.render_widget(Clear, size); 
                
                let block = Block::default()
                    .title(" Downloading Game Data Files ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, size);

                let inner = size.inner(&ratatui::layout::Margin { horizontal: 3, vertical: 2 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3), 
                        Constraint::Length(3), 
                        Constraint::Min(4),    
                    ])
                    .split(inner);

                let pct = if total > 0 { (completed * 100) / total } else { 0 };
                let title_text = format!("{} Progress: {}% ({}/{})", message, pct, completed, total);
                let current_p = Paragraph::new(format!("{}\nFile: {}", title_text, current_file))
                    .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
                f.render_widget(current_p, chunks[0]);

                let inner_width = chunks[1].width as usize - 2; 
                let filled_chars = if total > 0 { (completed * inner_width) / total } else { 0 };
                let mut bar = String::new();
                for _ in 0..filled_chars {
                    bar.push('█');
                }
                for _ in filled_chars..inner_width {
                    bar.push('░');
                }
                let bar_p = Paragraph::new(bar)
                    .style(Style::default().fg(Color::Cyan))
                    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
                f.render_widget(bar_p, chunks[1]);

                let log_items: Vec<ListItem> = logs.iter().rev().take(15).map(|log| {
                    ListItem::new(log.as_str()).style(Style::default().fg(Color::Rgb(150, 150, 160)))
                }).collect();

                let log_list = List::new(log_items)
                    .block(Block::default().borders(Borders::ALL).title(" Download Event Log "));
                f.render_widget(log_list, chunks[2]);
            }

            AppState::SyncingInstanceMods { completed, total, ref current_file, ref message, ref logs, .. } => {
                f.render_widget(Clear, size);
                
                let block = Block::default()
                    .title(" Syncing Mod Files ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, size);

                let inner = size.inner(&ratatui::layout::Margin { horizontal: 3, vertical: 2 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3), 
                        Constraint::Length(3), 
                        Constraint::Min(4),    
                    ])
                    .split(inner);

                let pct = if total > 0 { (completed * 100) / total } else { 0 };
                let title_text = format!("{} Progress: {}% ({}/{})", message, pct, completed, total);
                let current_p = Paragraph::new(format!("{}\nMod: {}", title_text, current_file))
                    .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
                f.render_widget(current_p, chunks[0]);

                let inner_width = chunks[1].width as usize - 2;
                let filled_chars = if total > 0 { (completed * inner_width) / total } else { 0 };
                let mut bar = String::new();
                for _ in 0..filled_chars { bar.push('█'); }
                for _ in filled_chars..inner_width { bar.push('░'); }
                let bar_p = Paragraph::new(bar)
                    .style(Style::default().fg(Color::Cyan))
                    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
                f.render_widget(bar_p, chunks[1]);

                let log_items: Vec<ListItem> = logs.iter().rev().take(15).map(|log| {
                    ListItem::new(log.as_str()).style(Style::default().fg(Color::Rgb(150, 150, 160)))
                }).collect();

                let log_list = List::new(log_items)
                    .block(Block::default().borders(Borders::ALL).title(" Mod Sync Log "));
                f.render_widget(log_list, chunks[2]);
            }

            AppState::ImportingModpackProgress { completed, total, ref current_file, ref message, ref logs, .. } => {
                f.render_widget(Clear, size);
                
                let block = Block::default()
                    .title(" Importing Modpack ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, size);

                let inner = size.inner(&ratatui::layout::Margin { horizontal: 3, vertical: 2 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3), 
                        Constraint::Length(3), 
                        Constraint::Min(4),    
                    ])
                    .split(inner);

                let pct = if total > 0 { (completed * 100) / total } else { 0 };
                let title_text = format!("{} Progress: {}% ({}/{})", message, pct, completed, total);
                let current_p = Paragraph::new(format!("{}\nFile: {}", title_text, current_file))
                    .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
                f.render_widget(current_p, chunks[0]);

                let inner_width = chunks[1].width as usize - 2;
                let filled_chars = if total > 0 { (completed * inner_width) / total } else { 0 };
                let mut bar = String::new();
                for _ in 0..filled_chars { bar.push('█'); }
                for _ in filled_chars..inner_width { bar.push('░'); }
                let bar_p = Paragraph::new(bar)
                    .style(Style::default().fg(Color::Cyan))
                    .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
                f.render_widget(bar_p, chunks[1]);

                let log_items: Vec<ListItem> = logs.iter().rev().take(15).map(|log| {
                    ListItem::new(log.as_str()).style(Style::default().fg(Color::Rgb(150, 150, 160)))
                }).collect();

                let log_list = List::new(log_items)
                    .block(Block::default().borders(Borders::ALL).title(" Modpack Import Log "));
                f.render_widget(log_list, chunks[2]);
            }

            AppState::BackupsMenu { instance_idx, ref backups, ref backups_list_state } => {
                let area = self.get_centered_rect(70, 70, size);
                f.render_widget(Clear, area);

                let inst = &self.instances[instance_idx];
                let block = Block::default()
                    .title(format!(" Backups for '{}' ", inst.config.name))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(4),    
                        Constraint::Length(4), 
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = backups.iter().map(|b| {
                    ListItem::new(format!("  {}", b)).style(Style::default().fg(Color::White))
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Created Snapshots ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                let mut state = backups_list_state.clone();
                f.render_stateful_widget(list, chunks[0], &mut state);

                let help_p = Paragraph::new("Press [B] to Create New Backup\nPress [Enter] to Restore Selected Backup\nPress [Esc] to Close Menu")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);
            }
            AppState::ChoosingModLoader { instance_id: _, ref instance_name, ref game_version, ref loader_options, ref loader_list_state } => {
                let area = self.get_centered_rect(60, 60, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(format!(" Mod Loader for '{}' (MC {}) ", instance_name, game_version))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(4),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = loader_options.iter().map(|opt| {
                    ListItem::new(format!("  {}", opt)).style(Style::default().fg(Color::White))
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Select a Loader ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                let mut state = loader_list_state.clone();
                f.render_stateful_widget(list, chunks[0], &mut state);

                let help_p = Paragraph::new("Press [Enter] to Select, [Esc] to Skip (Vanilla)")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);
            }

            AppState::ModManager { instance_idx, ref mods, ref mod_list_state } => {
                let area = self.get_centered_rect(80, 80, size);
                f.render_widget(Clear, area);

                let inst_name = self.instances.get(instance_idx)
                    .map(|i| i.config.name.clone())
                    .unwrap_or_else(|| "?".to_string());

                let block = Block::default()
                    .title(format!(" Mod Manager — {} ({} mods) ", inst_name, mods.len()))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(4),
                        Constraint::Length(4),
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = mods.iter().map(|m| {
                    let status = if m.enabled { "✓" } else { "✗" };
                    let status_color = if m.enabled { Color::Green } else { Color::Red };
                    let line = Line::from(vec![
                        Span::styled(format!(" [{}] ", status), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                        Span::styled(&m.metadata.name, Style::default().fg(Color::White)),
                        Span::styled(format!("  v{}", m.metadata.version), Style::default().fg(Color::Rgb(140, 140, 160))),
                    ]);
                    ListItem::new(line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Installed Mods ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                let mut state = mod_list_state.clone();
                f.render_stateful_widget(list, chunks[0], &mut state);

                // Show description of selected mod
                let desc_text = if let Some(idx) = mod_list_state.selected() {
                    if let Some(m) = mods.get(idx) {
                        format!("{}\n{}", m.metadata.name, m.metadata.description.as_deref().unwrap_or("No description available."))
                    } else {
                        String::new()
                    }
                } else {
                    "No mod selected.".to_string()
                };

                let desc_p = Paragraph::new(desc_text)
                    .style(Style::default().fg(Color::Rgb(180, 180, 200)))
                    .wrap(Wrap { trim: true })
                    .block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(border_color)));
                f.render_widget(desc_p, chunks[1]);
            }

            AppState::GameRunning { .. } => {}
        }
    }

    fn get_centered_rect(&self, percent_x: u16, percent_y: u16, r: Rect) -> Rect {
        let popup_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage((100 - percent_y) / 2),
                Constraint::Percentage(percent_y),
                Constraint::Percentage((100 - percent_y) / 2),
            ])
            .split(r);

        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(popup_layout[1])[1]
    }

    async fn handle_key(&mut self, key: KeyEvent) -> bool {
        match self.state {
            AppState::Normal => {
                match key.code {
                    KeyCode::Char('q') => return true,
                    KeyCode::Tab => {
                        self.active_tab = match self.active_tab {
                            Tab::Dashboard => Tab::Instances,
                            Tab::Instances => Tab::Accounts,
                            Tab::Accounts => Tab::Settings,
                            Tab::Settings => Tab::Dashboard,
                        };
                        self.status_message = None;
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Dashboard;
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Instances;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Accounts;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Settings;
                    }
                    KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('L') if self.active_tab == Tab::Dashboard => {
                        self.run_minecraft().await;
                    }
                    _ => self.handle_tab_keys(key).await,
                }
            }

            AppState::AddOfflineAccount => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let username = self.version_search_query.trim().to_string();
                        if !username.is_empty() {
                            let offline_uuid = uuid::Uuid::new_v4().simple().to_string();
                            let account = Account {
                                uuid: offline_uuid,
                                username,
                                account_type: AccountType::Offline,
                                microsoft_auth: None,
                            };
                            self.config.add_account(account);
                            self.state = AppState::Normal;
                            self.status_message = Some(("Created Offline Profile!".to_string(), false));
                            self.active_tab = Tab::Accounts;
                        }
                        self.version_search_query.clear();
                    }
                    KeyCode::Char(c) => {
                        self.version_search_query.push(c);
                    }
                    KeyCode::Backspace => {
                        self.version_search_query.pop();
                    }
                    _ => {}
                }
            }

            AppState::ImportingModpackPath => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let path_str = self.version_search_query.trim().to_string();
                        let path = std::path::PathBuf::from(&path_str);
                        if path.exists() && path.is_file() {
                            self.version_search_query.clear();
                            // Derive default ID from file stem
                            let default_id = path.file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("imported-pack")
                                .to_string();
                            self.version_search_query = default_id;
                            self.state = AppState::ImportingModpackId { path };
                        } else {
                            self.status_message = Some((format!("File does not exist: {}", path_str), true));
                        }
                    }
                    KeyCode::Char(c) => {
                        self.version_search_query.push(c);
                    }
                    KeyCode::Backspace => {
                        self.version_search_query.pop();
                    }
                    _ => {}
                }
            }

            AppState::ImportingModpackId { ref path } => {
                let path = path.clone();
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let custom_id = self.version_search_query.trim().to_string();
                        if !custom_id.is_empty() {
                            self.version_search_query.clear();
                            
                            let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                            let game_dir = self.config.game_dir.clone();
                            let path_clone = path.clone();
                            let id_clone = custom_id.clone();
                            
                            tokio::spawn(async move {
                                let _ = Instance::import_mrpack(&game_dir, &path_clone, &id_clone, &tx).await;
                            });

                            self.state = AppState::ImportingModpackProgress {
                                completed: 0,
                                total: 100,
                                current_file: String::new(),
                                message: "Starting modpack import...".to_string(),
                                logs: Vec::new(),
                                rx,
                            };
                        }
                    }
                    KeyCode::Char(c) => {
                        self.version_search_query.push(c);
                    }
                    KeyCode::Backspace => {
                        self.version_search_query.pop();
                    }
                    _ => {}
                }
            }

            AppState::CreatingInstanceName => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let id = self.version_search_query.trim().to_string();
                        if !id.is_empty() {
                            self.version_search_query.clear();
                            let name = format!("{} Profile", id);
                            if self.version_manifest.is_none() {
                                self.fetch_manifest().await;
                            }
                            self.state = AppState::CreatingInstanceVersion { id, name };
                            self.version_list_state.select(Some(0));
                        }
                    }
                    KeyCode::Char(c) => {
                        self.version_search_query.push(c);
                    }
                    KeyCode::Backspace => {
                        self.version_search_query.pop();
                    }
                    _ => {}
                }
            }

            AppState::CreatingInstanceVersion { ref id, ref name } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::F(1) => {
                        self.filter_releases = !self.filter_releases;
                        self.filter_versions();
                    }
                    KeyCode::F(2) => {
                        self.filter_snapshots = !self.filter_snapshots;
                        self.filter_versions();
                    }
                    KeyCode::Up => {
                        let selected = self.version_list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            self.version_list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down => {
                        let selected = self.version_list_state.selected().unwrap_or(0);
                        if selected + 1 < self.filtered_version_briefs.len() {
                            self.version_list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Char(c) => {
                        self.version_search_query.push(c);
                        self.filter_versions();
                    }
                    KeyCode::Backspace => {
                        self.version_search_query.pop();
                        self.filter_versions();
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = self.version_list_state.selected() {
                            if let Some(brief) = self.filtered_version_briefs.get(idx).cloned() {
                                let id_clone = id.clone();
                                let name_clone = name.clone();
                                let game_version = brief.id.clone();

                                // Build loader options asynchronously
                                let mut loader_options = vec!["Vanilla".to_string()];

                                // Fetch available Fabric loaders
                                if let Ok(fabric_loaders) = self.api_client.fetch_fabric_loaders(&game_version).await {
                                    for fl in fabric_loaders.iter().take(5) {
                                        let label = if fl.loader.stable {
                                            format!("Fabric {} (stable)", fl.loader.version)
                                        } else {
                                            format!("Fabric {}", fl.loader.version)
                                        };
                                        loader_options.push(label);
                                    }
                                }

                                // Fetch Forge versions for this MC version
                                if let Ok(forge_index) = self.api_client.fetch_forge_versions().await {
                                    for fv in forge_index.versions.iter().take(100) {
                                        // Check if this Forge version targets our MC version
                                        if fv.requires.iter().any(|r| r.uid == "net.minecraft" && r.equals == game_version) {
                                            let label = if fv.recommended {
                                                format!("Forge {} (recommended)", fv.version)
                                            } else {
                                                format!("Forge {}", fv.version)
                                            };
                                            loader_options.push(label);
                                        }
                                    }
                                }

                                // Fetch NeoForge versions for this MC version
                                if let Ok(neoforge_index) = self.api_client.fetch_neoforge_versions().await {
                                    for nv in neoforge_index.versions.iter().take(100) {
                                        if nv.requires.iter().any(|r| r.uid == "net.minecraft" && r.equals == game_version) {
                                            let label = if nv.recommended {
                                                format!("NeoForge {} (recommended)", nv.version)
                                            } else {
                                                format!("NeoForge {}", nv.version)
                                            };
                                            loader_options.push(label);
                                        }
                                    }
                                }

                                let mut loader_list_state = ListState::default();
                                loader_list_state.select(Some(0));

                                self.state = AppState::ChoosingModLoader {
                                    instance_id: id_clone,
                                    instance_name: name_clone,
                                    game_version,
                                    loader_options,
                                    loader_list_state,
                                };
                                self.version_search_query.clear();
                            }
                        }
                    }
                    _ => {}
                }
            }

            AppState::AddMicrosoftAccount { .. } => {
                if key.code == KeyCode::Esc {
                    self.state = AppState::Normal;
                    self.status_message = Some(("Cancelled Microsoft Login.".to_string(), false));
                }
            }

            AppState::EditingSetting { field_idx, ref mut input_value } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                    }
                    KeyCode::Enter => {
                        let val = input_value.trim().to_string();
                        match field_idx {
                            0 => self.config.game_dir = PathBuf::from(val),
                            1 => self.config.java_path = PathBuf::from(val),
                            2 => {
                                self.config.jvm_args = val.split_whitespace().map(|s| s.to_string()).collect();
                            }
                            _ => {}
                        }
                        let _ = self.config.save();
                        self.state = AppState::Normal;
                    }
                    KeyCode::Char(c) => {
                        input_value.push(c);
                    }
                    KeyCode::Backspace => {
                        input_value.pop();
                    }
                    _ => {}
                }
            }

            AppState::SelectInstanceFieldToEdit { instance_idx } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                    }
                    KeyCode::Char('1') => {
                        let initial_val = if let Some(inst) = self.instances.get(instance_idx) {
                            inst.config.java_path.clone().unwrap_or_default()
                        } else {
                            String::new()
                        };
                        self.state = AppState::EditingInstanceSetting {
                            instance_idx,
                            field_idx: 1,
                            input_value: initial_val,
                        };
                    }
                    KeyCode::Char('2') => {
                        let initial_val = if let Some(inst) = self.instances.get(instance_idx) {
                            inst.config.java_version.map(|v| v.to_string()).unwrap_or_default()
                        } else {
                            String::new()
                        };
                        self.state = AppState::EditingInstanceSetting {
                            instance_idx,
                            field_idx: 2,
                            input_value: initial_val,
                        };
                    }
                    _ => {}
                }
            }

            AppState::EditingInstanceSetting { instance_idx, field_idx, ref mut input_value } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::SelectInstanceFieldToEdit { instance_idx };
                    }
                    KeyCode::Enter => {
                        let val = input_value.trim().to_string();
                        if let Some(inst) = self.instances.get_mut(instance_idx) {
                            match field_idx {
                                1 => {
                                    if val.is_empty() || val == "clear" {
                                        inst.config.java_path = None;
                                    } else {
                                        inst.config.java_path = Some(val);
                                    }
                                }
                                2 => {
                                    if val.is_empty() || val == "0" || val == "clear" {
                                        inst.config.java_version = None;
                                    } else if let Ok(ver) = val.parse::<u32>() {
                                        inst.config.java_version = Some(ver);
                                    }
                                }
                                _ => {}
                            }
                            let _ = inst.save();
                        }
                        self.refresh_instances();
                        self.state = AppState::Normal;
                    }
                    KeyCode::Char(c) => {
                        input_value.push(c);
                    }
                    KeyCode::Backspace => {
                        input_value.pop();
                    }
                    _ => {}
                }
            }

            AppState::GameRunning { ref logs, ref status, ref mut scroll_offset, ref mut auto_scroll, .. } => {
                match key.code {
                    KeyCode::Esc => {
                        if status.is_some() {
                            self.state = AppState::Normal;
                            self.status_message = None;
                            self.refresh_instances();
                        }
                    }
                    KeyCode::Up => {
                        *auto_scroll = false;
                        if *scroll_offset > 0 {
                            *scroll_offset -= 1;
                        }
                    }
                    KeyCode::Down => {
                        *auto_scroll = false;
                        if *scroll_offset + 1 < logs.len() {
                            *scroll_offset += 1;
                        }
                    }
                    KeyCode::PageUp => {
                        *auto_scroll = false;
                        if *scroll_offset > 15 {
                            *scroll_offset -= 15;
                        } else {
                            *scroll_offset = 0;
                        }
                    }
                    KeyCode::PageDown => {
                        *auto_scroll = false;
                        if *scroll_offset + 15 < logs.len() {
                            *scroll_offset += 15;
                        } else if !logs.is_empty() {
                            *scroll_offset = logs.len() - 1;
                        }
                    }
                    KeyCode::End => {
                        *auto_scroll = true;
                        if !logs.is_empty() {
                            *scroll_offset = logs.len() - 1;
                        }
                    }
                    _ => {}
                }
            }

            AppState::BackupsMenu { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::BackupsMenu { instance_idx, backups, mut backups_list_state } = state {
                    match key.code {
                        KeyCode::Esc => {
                            // self.state is already AppState::Normal
                        }
                        KeyCode::Up => {
                            let selected = backups_list_state.selected().unwrap_or(0);
                            if selected > 0 {
                                backups_list_state.select(Some(selected - 1));
                            }
                            self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                        }
                        KeyCode::Down => {
                            let selected = backups_list_state.selected().unwrap_or(0);
                            if selected + 1 < backups.len() {
                                backups_list_state.select(Some(selected + 1));
                            }
                            self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                        }
                        KeyCode::Char('b') | KeyCode::Char('B') => {
                            let inst = &self.instances[instance_idx];
                            match inst.backup() {
                                Ok(p) => {
                                    self.status_message = Some((format!("Backup created: {}", p.file_name().unwrap().to_string_lossy()), false));
                                    let new_backups = inst.list_backups();
                                    let mut state = ListState::default();
                                    if !new_backups.is_empty() {
                                        state.select(Some(0));
                                    }
                                    self.state = AppState::BackupsMenu {
                                        instance_idx,
                                        backups: new_backups,
                                        backups_list_state: state,
                                    };
                                }
                                Err(e) => {
                                    self.status_message = Some((format!("Backup failed: {}", e), true));
                                    self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                                }
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(selected_idx) = backups_list_state.selected() {
                                if let Some(backup_name) = backups.get(selected_idx) {
                                    let inst = &self.instances[instance_idx];
                                    match inst.restore(backup_name) {
                                        Ok(_) => {
                                            self.status_message = Some((format!("Restored backup '{}' successfully!", backup_name), false));
                                            // self.state is already Normal
                                        }
                                        Err(e) => {
                                            self.status_message = Some((e, true));
                                            self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                                        }
                                    }
                                } else {
                                    self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                                }
                            } else {
                                self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                            }
                        }
                        _ => {
                            self.state = AppState::BackupsMenu { instance_idx, backups, backups_list_state };
                        }
                    }
                }
            }

            AppState::ChoosingModLoader { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, mut loader_list_state } = state {
                    match key.code {
                        KeyCode::Esc => {
                            // Skip mod loader → create vanilla instance
                            match Instance::create(&self.config.game_dir, &instance_id, &instance_name, &game_version) {
                                Ok(inst) => {
                                    self.config.active_instance = Some(inst.id.clone());
                                    let _ = self.config.save();
                                    self.refresh_instances();

                                    if !self.local_versions.contains(&game_version) {
                                        if let Some(brief) = self.filtered_version_briefs.iter().find(|b| b.id == game_version).cloned() {
                                            self.start_download_flow(brief).await;
                                        } else {
                                            self.state = AppState::Normal;
                                            self.status_message = Some((format!("Created vanilla instance '{}'!", instance_id), false));
                                        }
                                    } else {
                                        self.state = AppState::Normal;
                                        self.status_message = Some((format!("Created vanilla instance '{}'!", instance_id), false));
                                    }
                                }
                                Err(e) => {
                                    self.status_message = Some((format!("Failed: {}", e), true));
                                }
                            }
                            self.version_search_query.clear();
                        }
                        KeyCode::Up => {
                            let selected = loader_list_state.selected().unwrap_or(0);
                            if selected > 0 {
                                loader_list_state.select(Some(selected - 1));
                            }
                            self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                        }
                        KeyCode::Down => {
                            let selected = loader_list_state.selected().unwrap_or(0);
                            if selected + 1 < loader_options.len() {
                                loader_list_state.select(Some(selected + 1));
                            }
                            self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                        }
                        KeyCode::Enter => {
                            if let Some(idx) = loader_list_state.selected() {
                                if let Some(choice) = loader_options.get(idx).cloned() {
                                    // Determine version_id based on loader choice
                                    let version_id = if choice == "Vanilla" {
                                        game_version.clone()
                                    } else {
                                        let parts: Vec<&str> = choice.split_whitespace().collect();
                                        let loader_ver = parts.get(1).copied().unwrap_or("");

                                        if choice.starts_with("Fabric") {
                                            // Fetch fabric profile and install it
                                            let version_id = format!("fabric-loader-{}-{}", loader_ver, game_version);

                                            match self.api_client.fetch_fabric_profile(&game_version, loader_ver).await {
                                                Ok(profile) => {
                                                    let version_dir = self.config.game_dir
                                                        .join("versions")
                                                        .join(&version_id);
                                                    let _ = std::fs::create_dir_all(&version_dir);
                                                    let json_path = version_dir.join(format!("{}.json", version_id));
                                                    if let Ok(json_str) = serde_json::to_string_pretty(&profile) {
                                                        let _ = std::fs::write(&json_path, json_str);
                                                    }
                                                }
                                                Err(e) => {
                                                    self.status_message = Some((format!("Failed to fetch Fabric profile: {}", e), true));
                                                    self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                                                    return false;
                                                }
                                            }
                                            version_id
                                        } else {
                                            // Forge/NeoForge — use Prism meta
                                            let is_neoforge = choice.starts_with("NeoForge");
                                            let version_id = if is_neoforge {
                                                format!("neoforge-{}", loader_ver)
                                            } else {
                                                format!("forge-{}", loader_ver)
                                            };

                                            let profile_result = if is_neoforge {
                                                self.api_client.fetch_neoforge_profile(loader_ver).await
                                            } else {
                                                self.api_client.fetch_forge_profile(loader_ver).await
                                            };

                                            match profile_result {
                                                Ok(profile) => {
                                                    let version_dir = self.config.game_dir
                                                        .join("versions")
                                                        .join(&version_id);
                                                    let _ = std::fs::create_dir_all(&version_dir);
                                                    let json_path = version_dir.join(format!("{}.json", version_id));
                                                    if let Ok(json_str) = serde_json::to_string_pretty(&profile) {
                                                        let _ = std::fs::write(&json_path, json_str);
                                                    }
                                                }
                                                Err(e) => {
                                                    let loader_name = if is_neoforge { "NeoForge" } else { "Forge" };
                                                    self.status_message = Some((format!("Failed to fetch {} profile: {}", loader_name, e), true));
                                                    self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                                                    return false;
                                                }
                                            }
                                            version_id
                                        }
                                    };

                                    // Create the instance with the chosen version_id
                                    match Instance::create(&self.config.game_dir, &instance_id, &instance_name, &version_id) {
                                        Ok(inst) => {
                                            self.config.active_instance = Some(inst.id.clone());
                                            let _ = self.config.save();
                                            self.refresh_instances();

                                            // Need to download the base game + loader libraries
                                            let launcher = Launcher::new(self.config.clone());
                                            match launcher.load_version_details(&version_id) {
                                                Ok(details) => {
                                                    let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                                                    let game_dir = self.config.game_dir.clone();
                                                    tokio::spawn(async move {
                                                        let downloader = Downloader::new(tx);
                                                        let _ = downloader.download_version(&game_dir, &details).await;
                                                    });
                                                    self.state = AppState::Downloading {
                                                        completed: 0,
                                                        total: 100,
                                                        current_file: String::new(),
                                                        message: format!("Installing {}...", version_id),
                                                        logs: Vec::new(),
                                                        rx,
                                                        version_details: launcher.load_version_details(&version_id).unwrap(),
                                                    };
                                                }
                                                Err(e) => {
                                                    self.status_message = Some((format!("Failed to load version details: {}", e), true));
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            self.status_message = Some((format!("Failed to create instance: {}", e), true));
                                        }
                                    }
                                    self.version_search_query.clear();
                                } else {
                                    self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                                }
                            } else {
                                self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                            }
                        }
                        _ => {
                            self.state = AppState::ChoosingModLoader { instance_id, instance_name, game_version, loader_options, loader_list_state };
                        }
                    }
                }
            }

            AppState::ModManager { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::ModManager { instance_idx, mut mods, mut mod_list_state } = state {
                    match key.code {
                        KeyCode::Esc => {
                            // self.state already Normal
                        }
                        KeyCode::Up => {
                            let selected = mod_list_state.selected().unwrap_or(0);
                            if selected > 0 {
                                mod_list_state.select(Some(selected - 1));
                            }
                            self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                        }
                        KeyCode::Down => {
                            let selected = mod_list_state.selected().unwrap_or(0);
                            if selected + 1 < mods.len() {
                                mod_list_state.select(Some(selected + 1));
                            }
                            self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            // Toggle enabled/disabled
                            if let Some(selected) = mod_list_state.selected() {
                                if let Some(inst) = self.instances.get(instance_idx) {
                                    if let Some(m) = mods.get_mut(selected) {
                                        let mods_dir = inst.path.join("mods");
                                        let old_path = mods_dir.join(&m.filename);
                                        let new_filename = if m.enabled {
                                            format!("{}.disabled", m.filename)
                                        } else {
                                            m.filename.strip_suffix(".disabled").unwrap_or(&m.filename).to_string()
                                        };
                                        let new_path = mods_dir.join(&new_filename);
                                        if std::fs::rename(&old_path, &new_path).is_ok() {
                                            m.filename = new_filename;
                                            m.enabled = !m.enabled;
                                        }
                                    }
                                }
                            }
                            self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                        }
                        _ => {
                            self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                        }
                    }
                }
            }

            AppState::Downloading { .. } | AppState::SyncingInstanceMods { .. } | AppState::ImportingModpackProgress { .. } => {}
        }
        false
    }

    async fn handle_tab_keys(&mut self, key: KeyEvent) {
        match self.active_tab {
            Tab::Dashboard => {}
            
            Tab::Instances => {
                match key.code {
                    KeyCode::Up => {
                        let selected = self.instances_list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            self.instances_list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down => {
                        let selected = self.instances_list_state.selected().unwrap_or(0);
                        if selected + 1 < self.instances.len() {
                            self.instances_list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            if let Some(inst) = self.instances.get(idx) {
                                self.config.active_instance = Some(inst.id.clone());
                                let _ = self.config.save();
                            }
                        }
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') => {
                        self.state = AppState::CreatingInstanceName;
                        self.version_search_query.clear();
                    }
                    KeyCode::Char('e') | KeyCode::Char('E') => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            self.state = AppState::SelectInstanceFieldToEdit { instance_idx: idx };
                        }
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            if let Some(inst) = self.instances.get(idx).cloned() {
                                let _ = inst.delete();
                                if self.config.active_instance.as_ref() == Some(&inst.id) {
                                    self.config.active_instance = None;
                                    let _ = self.config.save();
                                }
                                self.refresh_instances();
                            }
                        }
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            if let Some(inst) = self.instances.get(idx).cloned() {
                                let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                                let game_dir = self.config.game_dir.clone();
                                tokio::spawn(async move {
                                    let _ = inst.sync_mods(&game_dir, tx).await;
                                });
                                self.state = AppState::SyncingInstanceMods {
                                    completed: 0,
                                    total: 100,
                                    current_file: String::new(),
                                    message: "Initializing mod sync...".to_string(),
                                    logs: Vec::new(),
                                    rx,
                                };
                            }
                        }
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            if let Some(inst) = self.instances.get(idx) {
                                let backups = inst.list_backups();
                                let mut state = ListState::default();
                                if !backups.is_empty() {
                                    state.select(Some(0));
                                }
                                self.state = AppState::BackupsMenu {
                                    instance_idx: idx,
                                    backups,
                                    backups_list_state: state,
                                };
                            }
                        }
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            if let Some(inst) = self.instances.get(idx) {
                                let mods = inst.get_mods().unwrap_or_default();
                                let mut mod_list_state = ListState::default();
                                if !mods.is_empty() {
                                    mod_list_state.select(Some(0));
                                }
                                self.state = AppState::ModManager {
                                    instance_idx: idx,
                                    mods,
                                    mod_list_state,
                                };
                            }
                        }
                    }
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        self.state = AppState::ImportingModpackPath;
                        self.version_search_query.clear();
                    }
                    _ => {}
                }
            }

            Tab::Accounts => {
                match key.code {
                    KeyCode::Up => {
                        let selected = self.account_list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            self.account_list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down => {
                        let selected = self.account_list_state.selected().unwrap_or(0);
                        if selected + 1 < self.config.accounts.len() {
                            self.account_list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = self.account_list_state.selected() {
                            if let Some(acc) = self.config.accounts.get(idx) {
                                self.config.active_account_uuid = Some(acc.uuid.clone());
                                let _ = self.config.save();
                            }
                        }
                    }
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        self.start_offline_account_flow();
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        self.start_microsoft_account_flow();
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        if let Some(idx) = self.account_list_state.selected() {
                            if let Some(acc) = self.config.accounts.get(idx).cloned() {
                                self.config.remove_account(&acc.uuid);
                                let len = self.config.accounts.len();
                                if len == 0 {
                                    self.account_list_state.select(None);
                                } else {
                                    self.account_list_state.select(Some(idx.min(len - 1)));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            Tab::Settings => {
                match key.code {
                    KeyCode::Up => {
                        let selected = self.settings_list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            self.settings_list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down => {
                        let selected = self.settings_list_state.selected().unwrap_or(0);
                        if selected < 2 {
                            self.settings_list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        let idx = self.settings_list_state.selected().unwrap_or(0);
                        let initial_val = match idx {
                            0 => self.config.game_dir.to_string_lossy().to_string(),
                            1 => self.config.java_path.to_string_lossy().to_string(),
                            2 => self.config.jvm_args.join(" "),
                            _ => String::new(),
                        };
                        self.state = AppState::EditingSetting {
                            field_idx: idx,
                            input_value: initial_val,
                        };
                    }
                    _ => {}
                }
            }
        }
    }
}

pub async fn run_tui() -> Result<(), String> {
    enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {}", e))?;
    
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, event::EnableMouseCapture)
        .map_err(|e| format!("Failed to enter alternate screen: {}", e))?;
        
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| format!("Failed to create terminal: {}", e))?;

    let mut app = App::new();
    
    app.version_list_state.select(Some(0));
    app.account_list_state.select(Some(0));
    app.settings_list_state.select(Some(0));

    loop {
        app.handle_background_tasks().await;
        
        terminal.draw(|f| app.draw(f)).map_err(|e| format!("Failed to draw TUI: {}", e))?;

        if event::poll(Duration::from_millis(50)).map_err(|e| format!("Poll failed: {}", e))? {
            if let Event::Key(key) = event::read().map_err(|e| format!("Failed to read key: {}", e))? {
                if key.kind == event::KeyEventKind::Press {
                    let should_quit = app.handle_key(key).await;
                    if should_quit {
                        break;
                    }
                }
            }
        }
    }

    disable_raw_mode().map_err(|e| format!("Failed to disable raw mode: {}", e))?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        event::DisableMouseCapture
    ).map_err(|e| format!("Failed to leave alternate screen: {}", e))?;
    
    terminal.show_cursor().map_err(|e| format!("Failed to show cursor: {}", e))?;
    
    Ok(())
}
