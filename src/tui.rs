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
use uuid::Uuid;

use crate::config::{Config, Account, AccountType, MicrosoftAuth};
use crate::api::{ApiClient, VersionBrief, VersionDetails, VersionManifest};
use crate::downloader::{Downloader, ProgressUpdate};
use crate::launcher::Launcher;

#[derive(Copy, Clone, Debug, PartialEq)]
enum Tab {
    Dashboard,
    Versions,
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
    Downloading {
        completed: usize,
        total: usize,
        current_file: String,
        message: String,
        logs: Vec<String>,
        rx: Receiver<ProgressUpdate>,
        version_details: VersionDetails,
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
}

impl App {
    pub fn new() -> Self {
        let config = Config::load();
        let launcher = Launcher::new(config.clone());
        let local_versions = launcher.get_available_local_versions();
        
        Self {
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
        }
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

    fn get_selected_version_id(&self) -> Option<String> {
        self.config.selected_version.clone()
    }

    fn start_offline_account_flow(&mut self) {
        self.state = AppState::AddOfflineAccount;
        self.status_message = None;
    }

    fn start_microsoft_account_flow(&mut self) {
        let (tx, rx) = mpsc::channel::<MicrosoftAuthUpdate>(10);
        let api = ApiClient::new();

        // Spawn Microsoft Device Code polling
        tokio::spawn(async move {
            match api.request_device_code().await {
                Ok(device_res) => {
                    let _ = tx.send(MicrosoftAuthUpdate::Code {
                        user_code: device_res.user_code,
                        verification_uri: device_res.verification_uri,
                    }).await;
                    
                    // Poll Microsoft for authorization
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
                                // Exchange for Xbox and Minecraft Token
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
                                // Still pending
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

        // Set state to loading while we wait for the first response
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
                        let version_id = version_details.id.clone();
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
                    // Refresh local version list
                    let launcher = Launcher::new(self.config.clone());
                    self.local_versions = launcher.get_available_local_versions();
                    
                    self.status_message = Some((format!("Successfully downloaded and verified version {}!", version_id), false));
                    self.config.selected_version = Some(version_id);
                    let _ = self.config.save();
                }
                Err(e) => {
                    self.status_message = Some((format!("Download failed: {}", e), true));
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

                // Save details JSON
                let details_json_path = game_dir
                    .join("versions")
                    .join(&details.id)
                    .join(format!("{}.json", details.id));
                if let Some(parent) = details_json_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(content) = serde_json::to_string_pretty(&details) {
                    let _ = std::fs::write(&details_json_path, content);
                }

                tokio::spawn(async move {
                    if let Err(_e) = downloader.download_version(&game_dir, &details_clone).await {
                        // Download failed
                    }
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
        let version_id = match self.get_selected_version_id() {
            Some(v) => v,
            None => {
                self.status_message = Some(("No version selected. Please select a version in the 'Versions' tab.".to_string(), true));
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

        // Suspend TUI
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = crossterm::execute!(stdout, LeaveAlternateScreen, crossterm::event::DisableMouseCapture);
        println!("==================================================");
        println!("Launching Minecraft {} as {}...", version_id, account.username);
        println!("Game outputs will print below. Please wait...");
        println!("==================================================");

        let launcher = Launcher::new(self.config.clone());
        let launch_res = launcher.launch(&version_id, &account).await;

        println!("\n==================================================");
        if let Err(e) = launch_res {
            println!("Game launch failed: {}", e);
            println!("Press enter to return to the launcher.");
            let mut temp = String::new();
            let _ = io::stdin().read_line(&mut temp);
        } else {
            println!("Minecraft game closed successfully.");
            println!("Press enter to return to the launcher.");
            let mut temp = String::new();
            let _ = io::stdin().read_line(&mut temp);
        }

        // Restore TUI
        let _ = enable_raw_mode();
        let _ = crossterm::execute!(io::stdout(), EnterAlternateScreen, crossterm::event::EnableMouseCapture);
        self.status_message = None;
    }

    fn draw(&mut self, f: &mut ratatui::Frame) {
        let size = f.size();

        // Theme colors
        let bg_color = Color::Rgb(15, 17, 26); // Deep blue-gray dark mode
        let border_color = Color::Rgb(86, 73, 150); // Muted violet
        let text_color = Color::Rgb(220, 222, 235); // Ice white
        let active_color = Color::Rgb(46, 204, 113); // Emerald green for play
        let select_color = Color::Rgb(142, 68, 173); // Purple accent

        let main_block = Block::default()
            .bg(bg_color)
            .style(Style::default().fg(text_color));
        f.render_widget(main_block, size);

        // Core Layout: Title, Main content (Sidebar + Body), Help bar
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Title Banner
                Constraint::Min(5),    // Main content area
                Constraint::Length(3), // Footer Help Bar
            ])
            .split(size);

        // 1. Draw Title Banner
        let logo = format!(" ✦ MineCLI Terminal Launcher ✦ ");
        let logo_p = Paragraph::new(logo)
            .style(Style::default().fg(Color::Rgb(155, 89, 182)).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(border_color)));
        f.render_widget(logo_p, chunks[0]);

        // Draw Status Message if any
        if let Some((ref msg, is_error)) = self.status_message {
            let color = if is_error { Color::Red } else { Color::Cyan };
            let status_rect = Rect {
                x: chunks[0].x + chunks[0].width - 45,
                y: chunks[0].y + 1,
                width: 42,
                height: 1,
            };
            let status_p = Paragraph::new(format!("● {}", msg))
                .style(Style::default().fg(color).add_modifier(Modifier::ITALIC))
                .wrap(Wrap { trim: true });
            f.render_widget(status_p, status_rect);
        }

        // 2. Draw Main content area (Sidebar + Body)
        let main_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22), // Sidebar width
                Constraint::Min(20),    // Body width
            ])
            .split(chunks[1]);

        // Draw Sidebar Menu
        self.draw_sidebar(f, main_layout[0], border_color, select_color);

        // Draw active view body
        match self.active_tab {
            Tab::Dashboard => self.draw_dashboard(f, main_layout[1], border_color, active_color),
            Tab::Versions => self.draw_versions(f, main_layout[1], border_color, select_color),
            Tab::Accounts => self.draw_accounts(f, main_layout[1], border_color, select_color),
            Tab::Settings => self.draw_settings(f, main_layout[1], border_color, select_color),
        }

        // 3. Draw Footer Help Bar
        self.draw_footer(f, chunks[2], border_color);

        // Render modal overlay states (like text inputs, device codes)
        self.draw_overlays(f, size, select_color);
    }

    fn draw_sidebar(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
        let menu_items = vec![
            "[D] Dashboard",
            "[V] Versions",
            "[A] Accounts",
            "[S] Settings",
        ];

        let list_items: Vec<ListItem> = menu_items.iter().enumerate().map(|(idx, item)| {
            let tab_match = match idx {
                0 => self.active_tab == Tab::Dashboard,
                1 => self.active_tab == Tab::Versions,
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
                Constraint::Length(5), // Launch Game Panel
                Constraint::Length(6), // Selected Settings Profile
                Constraint::Min(4),    // Instructions / Welcome
            ])
            .split(inner);

        // A. Launch Game Panel
        let version_str = self.get_selected_version_id().unwrap_or_else(|| "None selected".to_string());
        let account_str = self.active_account_desc();
        
        let launch_btn_text = if self.config.selected_version.is_some() && self.config.active_account_uuid.is_some() {
            format!(" ► LAUNCH MINECRAFT {} (Press Enter or 'L') ◄ ", version_str)
        } else {
            " ⚠️ Setup Required (Select version & account) ⚠️ ".to_string()
        };

        let launch_btn_style = if self.config.selected_version.is_some() && self.config.active_account_uuid.is_some() {
            Style::default().fg(active_color).add_modifier(Modifier::BOLD).bg(Color::Rgb(20, 40, 25))
        } else {
            Style::default().fg(Color::Rgb(230, 126, 34)).add_modifier(Modifier::BOLD)
        };

        let launch_btn = Paragraph::new(format!("\n{}", launch_btn_text))
            .alignment(ratatui::layout::Alignment::Center)
            .style(launch_btn_style)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(active_color)));
        
        f.render_widget(launch_btn, chunks[0]);

        // B. Selected Settings Profile
        let details_text = vec![
            Line::from(vec![Span::raw("Active Player:  "), Span::styled(account_str, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]),
            Line::from(vec![Span::raw("Game Version:   "), Span::styled(version_str, Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))]),
            Line::from(vec![Span::raw("Game Directory: "), Span::styled(self.config.game_dir.to_string_lossy().to_string(), Style::default().fg(Color::Yellow))]),
            Line::from(vec![Span::raw("JVM Memory:     "), Span::styled(self.config.jvm_args.join(" "), Style::default().fg(Color::Yellow))]),
        ];
        let details = Paragraph::new(details_text)
            .block(Block::default().title(" Active Session Details ").borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        
        f.render_widget(details, chunks[1]);

        // C. Welcome Banner
        let ascii_art = r#"
  __  __ _            _____ _      _____ 
 |  \/  (_)          / ____| |    |_   _|
 | \  / |_ _ __   ___| |    | |      | |  
 | |\/| | | '_ \ / _ \ |    | |      | |  
 | |  | | | | | |  __/ |____| |____ _| |_ 
 |_|  |_|_|_| |_|\___|\_____|______|_____|
        "#;
        let welcome_text = format!("{}\n\nWelcome to MineCLI. Select a version and account from the side tabs to get started.\nPress 'q' at any time to quit.", ascii_art);
        let welcome = Paragraph::new(welcome_text)
            .alignment(ratatui::layout::Alignment::Center)
            .style(Style::default().fg(Color::Rgb(130, 130, 150)))
            .wrap(Wrap { trim: false });
        f.render_widget(welcome, chunks[2]);
    }

    fn draw_versions(&self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
        let main_block = Block::default()
            .title(" Minecraft Version Manager ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        
        let inner = main_block.inner(rect);
        f.render_widget(main_block, rect);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search bar
                Constraint::Min(4),    // Version list
            ])
            .split(inner);

        // A. Search Bar
        let release_status = if self.filter_releases { "● Releases (F1)" } else { "○ Releases (F1)" };
        let snapshot_status = if self.filter_snapshots { "● Snapshots (F2)" } else { "○ Snapshots (F2)" };
        
        let search_text = format!(" Search: {:<30} | {} | {}", self.version_search_query, release_status, snapshot_status);
        let search_p = Paragraph::new(search_text)
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        f.render_widget(search_p, chunks[0]);

        // B. Version List
        let list_items: Vec<ListItem> = self.filtered_version_briefs.iter().map(|v| {
            let is_local = self.local_versions.contains(&v.id);
            let is_selected = self.config.selected_version.as_deref() == Some(&v.id);

            let status_span = if is_local {
                Span::styled(" [LOCAL] ", Style::default().fg(Color::Green))
            } else {
                Span::styled(" [CLOUD] ", Style::default().fg(Color::Yellow))
            };

            let select_span = if is_selected {
                Span::styled(" ★ ACTIVE ★ ", Style::default().fg(Color::Rgb(155, 89, 182)).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            };

            let item_line = Line::from(vec![
                Span::styled(format!(" {:<18}", v.id), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::styled(format!(" ({:<10})", v.r#type), Style::default().fg(Color::Rgb(160, 160, 170))),
                status_span,
                select_span,
            ]);

            ListItem::new(item_line)
        }).collect();

        // Render List with State
        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title(" Available Versions ").border_style(Style::default().fg(border_color)))
            .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

        // Use standard state rendering
        let mut state = self.version_list_state.clone();
        f.render_stateful_widget(list, chunks[1], &mut state);
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
                Constraint::Min(4),    // Account list
                Constraint::Length(5), // Quick instructions / actions
            ])
            .split(inner);

        // A. Account List
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

        // B. Quick Actions Help Panel
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
                Constraint::Min(4),    // List of settings fields
                Constraint::Length(4), // Edit help instructions
            ])
            .split(inner);

        // A. List of Settings Fields
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

        // B. Instructions Help
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

    fn draw_overlays(&self, f: &mut ratatui::Frame, size: Rect, select_color: Color) {
        match self.state {
            AppState::Normal => {}
            
            AppState::AddOfflineAccount => {
                let area = self.get_centered_rect(50, 20, size);
                f.render_widget(Clear, area); // Clear underlying pixels
                
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
                        Constraint::Length(3), // Input box
                        Constraint::Length(2), // Help text
                    ])
                    .split(area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 }));

                f.render_widget(Paragraph::new("Enter desired username:"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) // Reuse query string as buffer or temporary value
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Create, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
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
                        Constraint::Length(2), // Intro
                        Constraint::Length(3), // URL
                        Constraint::Length(4), // Big Code Box
                        Constraint::Length(3), // Polling indicator
                        Constraint::Length(2), // Escape help
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

            AppState::Downloading { completed, total, ref current_file, ref message, ref logs, .. } => {
                f.render_widget(Clear, size); // Take over entire window during download
                
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
                        Constraint::Length(3), // Progress Info Title
                        Constraint::Length(3), // Visual Progress Bar
                        Constraint::Min(4),    // Log list
                    ])
                    .split(inner);

                // A. Progress Info Title
                let pct = if total > 0 { (completed * 100) / total } else { 0 };
                let title_text = format!("{} Progress: {}% ({}/{})", message, pct, completed, total);
                let current_p = Paragraph::new(format!("{}\nFile: {}", title_text, current_file))
                    .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
                f.render_widget(current_p, chunks[0]);

                // B. Visual Progress Bar
                let inner_width = chunks[1].width as usize - 2; // borders
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

                // C. Log List
                let log_items: Vec<ListItem> = logs.iter().rev().take(15).map(|log| {
                    ListItem::new(log.as_str()).style(Style::default().fg(Color::Rgb(150, 150, 160)))
                }).collect();

                let log_list = List::new(log_items)
                    .block(Block::default().borders(Borders::ALL).title(" Download Event Log "));
                f.render_widget(log_list, chunks[2]);
            }
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
        // Mode specific handling
        match self.state {
            AppState::Normal => {
                // Main key bindings
                match key.code {
                    KeyCode::Char('q') => return true,
                    KeyCode::Tab => {
                        self.active_tab = match self.active_tab {
                            Tab::Dashboard => Tab::Versions,
                            Tab::Versions => Tab::Accounts,
                            Tab::Accounts => Tab::Settings,
                            Tab::Settings => Tab::Dashboard,
                        };
                        self.status_message = None;
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Dashboard;
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Versions;
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Accounts;
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') if self.active_tab == Tab::Dashboard => {
                        self.active_tab = Tab::Settings;
                    }

                    // Launch triggers
                    KeyCode::Enter | KeyCode::Char('l') | KeyCode::Char('L') if self.active_tab == Tab::Dashboard => {
                        self.run_minecraft().await;
                    }

                    // Tab specific inputs
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
                            // Generate offline UUID
                            let offline_uuid = Uuid::new_v4().simple().to_string();
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

            AppState::AddMicrosoftAccount { .. } => {
                if key.code == KeyCode::Esc {
                    // Cancel MS login
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

            AppState::Downloading { .. } => {
                // Intercept keys during downloading
            }
        }
        false
    }

    async fn handle_tab_keys(&mut self, key: KeyEvent) {
        match self.active_tab {
            Tab::Dashboard => {}
            
            Tab::Versions => {
                match key.code {
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
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        // Force download selected version
                        if let Some(idx) = self.version_list_state.selected() {
                            if let Some(brief) = self.filtered_version_briefs.get(idx).cloned() {
                                self.start_download_flow(brief).await;
                            }
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
                        // Select version as active if local, else download it!
                        if let Some(idx) = self.version_list_state.selected() {
                            if let Some(brief) = self.filtered_version_briefs.get(idx).cloned() {
                                if self.local_versions.contains(&brief.id) {
                                    self.config.selected_version = Some(brief.id);
                                    let _ = self.config.save();
                                } else {
                                    self.start_download_flow(brief).await;
                                }
                            }
                        }
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
                        // Set selected active
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
                                // Adjust selection state
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
    
    // Fetch manifests in background thread
    let version_manifest_present = app.version_manifest.is_some();
    if !version_manifest_present {
        app.fetch_manifest().await;
    }

    // Set default selections
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

    // Clean up
    disable_raw_mode().map_err(|e| format!("Failed to disable raw mode: {}", e))?;
    crossterm::execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        event::DisableMouseCapture
    ).map_err(|e| format!("Failed to leave alternate screen: {}", e))?;
    
    terminal.show_cursor().map_err(|e| format!("Failed to show cursor: {}", e))?;
    
    Ok(())
}
