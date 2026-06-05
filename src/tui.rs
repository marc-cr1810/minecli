use std::io;
use std::path::PathBuf;
use std::fs::{self, File};
use std::time::Duration;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, BorderType, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetSearchType {
    Mod,
    Shader,
    ResourcePack,
    Datapack { world_name: String },
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
    ModpackMenu {
        selected_option: usize,
    },
    SearchingModpackQuery,
    SearchingModpackLoading {
        query: String,
        rx: tokio::sync::oneshot::Receiver<Result<Vec<crate::api::ModrinthSearchHit>, String>>,
    },
    SearchingModpackResults {
        query: String,
        hits: Vec<crate::api::ModrinthSearchHit>,
        list_state: ListState,
    },
    SearchingModpackVersionsLoading {
        hit: crate::api::ModrinthSearchHit,
        rx: tokio::sync::oneshot::Receiver<Result<Vec<crate::api::ModrinthVersion>, String>>,
    },
    SearchingModpackVersions {
        hit: crate::api::ModrinthSearchHit,
        versions: Vec<crate::api::ModrinthVersion>,
        list_state: ListState,
    },
    SearchingModpackConfirmId {
        hit: crate::api::ModrinthSearchHit,
        version: crate::api::ModrinthVersion,
        url: String,
        filename: String,
    },
    ExportingModpackPath {
        instance_idx: usize,
    },
    SearchingModQuery {
        instance_idx: usize,
        search_type: AssetSearchType,
    },
    SearchingModLoading {
        instance_idx: usize,
        search_type: AssetSearchType,
        query: String,
        rx: tokio::sync::oneshot::Receiver<Result<Vec<crate::api::ModrinthSearchHit>, String>>,
    },
    SearchingModResults {
        instance_idx: usize,
        search_type: AssetSearchType,
        query: String,
        hits: Vec<crate::api::ModrinthSearchHit>,
        list_state: ListState,
    },
    SearchingModVersionsLoading {
        instance_idx: usize,
        search_type: AssetSearchType,
        hit: crate::api::ModrinthSearchHit,
        rx: tokio::sync::oneshot::Receiver<Result<Vec<crate::api::ModrinthVersion>, String>>,
    },
    SearchingModVersions {
        instance_idx: usize,
        search_type: AssetSearchType,
        hit: crate::api::ModrinthSearchHit,
        versions: Vec<crate::api::ModrinthVersion>,
        list_state: ListState,
    },
    InstallingModProgress {
        instance_idx: usize,
        search_type: AssetSearchType,
        completed: usize,
        total: usize,
        current_file: String,
        message: String,
        rx: tokio::sync::mpsc::Receiver<ProgressUpdate>,
    },
    AssetManager {
        instance_idx: usize,
        selected_category: usize,
        has_shader_support: bool,
    },
    WorldManager {
        instance_idx: usize,
        worlds: Vec<crate::assets::WorldInfo>,
        list_state: ListState,
        confirm_delete: Option<String>,
    },
    PromptWorldNameForDatapack {
        instance_idx: usize,
        input_value: String,
    },
    ResourcePackManager {
        instance_idx: usize,
        packs: Vec<crate::assets::AssetInfo>,
        list_state: ListState,
    },
    ShaderPackManager {
        instance_idx: usize,
        shaders: Vec<crate::assets::AssetInfo>,
        list_state: ListState,
        has_shader_support: bool,
    },
    InstallingShaderSupport {
        instance_idx: usize,
        completed: usize,
        total: usize,
        current_file: String,
        message: String,
        logs: Vec<String>,
        rx: tokio::sync::mpsc::Receiver<ProgressUpdate>,
    },
    ScreenshotManager {
        instance_idx: usize,
        screenshots: Vec<crate::assets::ScreenshotInfo>,
        list_state: ListState,
        confirm_delete: Option<String>,
    },
    PromptRenameAsset {
        instance_idx: usize,
        asset_type: String,
        old_filename: String,
        input_value: String,
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

        // 3c. Process Modrinth Modpack search results
        let mut search_finished_state = None;
        if let AppState::SearchingModpackLoading { ref query, ref mut rx } = self.state
            && let Ok(res) = rx.try_recv() {
                search_finished_state = Some((query.clone(), res));
            }
        if let Some((query, res)) = search_finished_state {
            match res {
                Ok(hits) => {
                    let mut list_state = ListState::default();
                    if !hits.is_empty() {
                        list_state.select(Some(0));
                    }
                    self.state = AppState::SearchingModpackResults { query, hits, list_state };
                }
                Err(e) => {
                    self.status_message = Some((format!("Search failed: {}", e), true));
                    self.state = AppState::SearchingModpackQuery;
                }
            }
        }

        // 3d. Process Modrinth Modpack version results
        let mut versions_finished_state = None;
        if let AppState::SearchingModpackVersionsLoading { ref hit, ref mut rx } = self.state
            && let Ok(res) = rx.try_recv() {
                versions_finished_state = Some((hit.clone(), res));
            }
        if let Some((hit, res)) = versions_finished_state {
            match res {
                Ok(versions) => {
                    let mut list_state = ListState::default();
                    if !versions.is_empty() {
                        list_state.select(Some(0));
                    }
                    self.state = AppState::SearchingModpackVersions { hit, versions, list_state };
                }
                Err(e) => {
                    self.status_message = Some((format!("Failed to load versions: {}", e), true));
                    self.state = AppState::Normal;
                }
            }
        }

        // 3e. Process Modrinth Mod search results
        let mut mod_search_finished_state = None;
        if let AppState::SearchingModLoading { instance_idx, ref search_type, ref query, ref mut rx } = self.state
            && let Ok(res) = rx.try_recv() {
                mod_search_finished_state = Some((instance_idx, search_type.clone(), query.clone(), res));
            }
        if let Some((instance_idx, search_type, query, res)) = mod_search_finished_state {
            match res {
                Ok(hits) => {
                    let mut list_state = ListState::default();
                    if !hits.is_empty() {
                        list_state.select(Some(0));
                    }
                    self.state = AppState::SearchingModResults { instance_idx, search_type, query, hits, list_state };
                }
                Err(e) => {
                    self.status_message = Some((format!("Search failed: {}", e), true));
                    self.state = AppState::SearchingModQuery { instance_idx, search_type };
                }
            }
        }

        // 3f. Process Modrinth Mod version results
        let mut mod_versions_finished_state = None;
        if let AppState::SearchingModVersionsLoading { instance_idx, ref search_type, ref hit, ref mut rx } = self.state
            && let Ok(res) = rx.try_recv() {
                mod_versions_finished_state = Some((instance_idx, search_type.clone(), hit.clone(), res));
            }
        if let Some((instance_idx, search_type, hit, res)) = mod_versions_finished_state {
            match res {
                Ok(versions) => {
                    let mut list_state = ListState::default();
                    if let Some(inst) = self.instances.get(instance_idx) {
                        let (game_version, loader) = inst.get_game_version_and_loader(&self.config.game_dir);
                        let compatible_versions: Vec<crate::api::ModrinthVersion> = versions.into_iter().filter(|v| {
                            let matches_game = v.game_versions.contains(&game_version);
                            let matches_loader = match search_type {
                                AssetSearchType::Mod => match loader.as_deref() {
                                    Some(l) => v.loaders.iter().any(|loader_name| loader_name.to_lowercase() == l.to_lowercase()),
                                    None => true,
                                },
                                _ => true, // Category/loader-independent search for shaders, packs, datapacks
                            };
                            matches_game && matches_loader
                        }).collect();

                        if compatible_versions.is_empty() {
                            self.status_message = Some((format!("No compatible versions found for Minecraft {} ({}).", game_version, loader.as_deref().unwrap_or("vanilla")), true));
                            self.state = AppState::SearchingModQuery { instance_idx, search_type };
                        } else {
                            list_state.select(Some(0));
                            self.state = AppState::SearchingModVersions { instance_idx, search_type, hit, versions: compatible_versions, list_state };
                        }
                    } else {
                        self.state = AppState::Normal;
                    }
                }
                Err(e) => {
                    self.status_message = Some((format!("Failed to load versions: {}", e), true));
                    self.state = AppState::Normal;
                }
            }
        }

        // 3g. Process Mod Installation progress updates
        let mut mod_install_finished_state = None;
        if let AppState::InstallingModProgress { instance_idx, ref search_type, ref mut completed, ref mut total, ref mut current_file, ref mut message, ref mut rx } = self.state {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Started { total: t, message: msg } => {
                        *total = t;
                        *completed = 0;
                        *message = msg;
                    }
                    ProgressUpdate::Progress { completed: c, total: t, current_file: f } => {
                        *completed = c;
                        *total = t;
                        *current_file = f;
                    }
                    ProgressUpdate::Message(msg) => {
                        *message = msg;
                    }
                    ProgressUpdate::Finished => {
                        mod_install_finished_state = Some((instance_idx, search_type.clone(), Ok(())));
                    }
                    ProgressUpdate::Error(e) => {
                        mod_install_finished_state = Some((instance_idx, search_type.clone(), Err(e)));
                    }
                }
            }
        }
        if let Some((instance_idx, search_type, res)) = mod_install_finished_state {
            let asset_name = match search_type {
                AssetSearchType::Mod => "Mod",
                AssetSearchType::Shader => "Shader pack",
                AssetSearchType::ResourcePack => "Resource pack",
                AssetSearchType::Datapack { .. } => "Datapack",
            };
            match res {
                Ok(_) => {
                    self.status_message = Some((format!("{} installed successfully!", asset_name), false));
                }
                Err(e) => {
                    self.status_message = Some((format!("Failed to install {}: {}", asset_name.to_lowercase(), e), true));
                }
            }
            if let Some(inst) = self.instances.get(instance_idx) {
                match search_type {
                    AssetSearchType::Mod => {
                        if let Ok(mods) = inst.get_mods() {
                            let mut mod_list_state = ListState::default();
                            if !mods.is_empty() {
                                mod_list_state.select(Some(0));
                            }
                            self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                        } else {
                            self.state = AppState::Normal;
                        }
                    }
                    AssetSearchType::Shader => {
                        let shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                        let mut list_state = ListState::default();
                        if !shaders.is_empty() {
                            list_state.select(Some(0));
                        }
                        let has_shader_support = crate::assets::detect_shader_support(&inst.path);
                        self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                    }
                    AssetSearchType::ResourcePack => {
                        let packs = crate::assets::list_resourcepacks(&inst.path).unwrap_or_default();
                        let mut list_state = ListState::default();
                        if !packs.is_empty() {
                            list_state.select(Some(0));
                        }
                        self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                    }
                    AssetSearchType::Datapack { .. } => {
                        let worlds = crate::assets::list_worlds(&inst.path).unwrap_or_default();
                        let mut list_state = ListState::default();
                        if !worlds.is_empty() {
                            list_state.select(Some(0));
                        }
                        self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                    }
                }
            } else {
                self.state = AppState::Normal;
            }
        }

        // 3h. Process Shader Support Installation progress updates
        let mut shader_install_finished_state = None;
        if let AppState::InstallingShaderSupport { instance_idx, ref mut completed, ref mut total, ref mut current_file, ref mut message, ref mut logs, ref mut rx } = self.state {
            while let Ok(update) = rx.try_recv() {
                match update {
                    ProgressUpdate::Started { total: t, message: msg } => {
                        *total = t;
                        *completed = 0;
                        *message = msg.clone();
                        logs.push(msg);
                    }
                    ProgressUpdate::Progress { completed: c, total: t, current_file: f } => {
                        *completed = c;
                        *total = t;
                        *current_file = f.clone();
                        logs.push(format!("Installing: {}", f));
                    }
                    ProgressUpdate::Message(msg) => {
                        *message = msg.clone();
                        logs.push(msg);
                    }
                    ProgressUpdate::Finished => {
                        shader_install_finished_state = Some((instance_idx, Ok(())));
                    }
                    ProgressUpdate::Error(e) => {
                        shader_install_finished_state = Some((instance_idx, Err(e)));
                    }
                }
            }
        }
        if let Some((instance_idx, res)) = shader_install_finished_state {
            match res {
                Ok(_) => {
                    self.status_message = Some(("Shader support enabled successfully!".to_string(), false));
                }
                Err(e) => {
                    self.status_message = Some((format!("Failed to install shader support: {}", e), true));
                }
            }
            if let Some(inst) = self.instances.get(instance_idx) {
                let shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                let mut list_state = ListState::default();
                if !shaders.is_empty() {
                    list_state.select(Some(0));
                }
                let has_shader_support = crate::assets::detect_shader_support(&inst.path);
                self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
            } else {
                self.state = AppState::Normal;
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
                if res_val.is_err() {
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

        let version_id = instance.config.version.clone();
        let version_json_path = self.config.game_dir
            .join("versions")
            .join(&version_id)
            .join(format!("{}.json", version_id));
        let jar_version_id = if version_json_path.exists() {
            let launcher = Launcher::new(self.config.clone());
            if let Ok(raw_details) = launcher.load_version_details_raw(&version_id) {
                raw_details.inheritsFrom.clone().unwrap_or_else(|| version_id.clone())
            } else {
                version_id.clone()
            }
        } else {
            version_id.clone()
        };

        let client_jar_path = self.config.game_dir
            .join("versions")
            .join(&jar_version_id)
            .join(format!("{}.jar", jar_version_id));

        if !version_json_path.exists() || !client_jar_path.exists() {
            let api = self.api_client.clone();
            let version_id_clone = version_id.clone();
            
            // Check loader type and fetch profile details
            let details_res = if version_json_path.exists() {
                let launcher = Launcher::new(self.config.clone());
                launcher.load_version_details_raw(&version_id_clone)
            } else if version_id_clone.starts_with("fabric-loader-") {
                if let Some(rest) = version_id_clone.strip_prefix("fabric-loader-") {
                    if let Some((loader_ver, game_ver)) = rest.split_once('-') {
                        api.fetch_fabric_profile(game_ver, loader_ver).await
                    } else {
                        Err("Invalid Fabric version ID format".to_string())
                    }
                } else {
                    Err("Invalid Fabric prefix".to_string())
                }
            } else if version_id_clone.starts_with("forge-") {
                let loader_ver = version_id_clone.strip_prefix("forge-").unwrap_or_default();
                api.fetch_forge_profile(loader_ver).await
            } else if version_id_clone.starts_with("neoforge-") {
                let loader_ver = version_id_clone.strip_prefix("neoforge-").unwrap_or_default();
                api.fetch_neoforge_profile(loader_ver).await
            } else {
                match api.fetch_version_manifest().await {
                    Ok(manifest) => {
                        if let Some(brief) = manifest.versions.iter().find(|v| v.id == version_id_clone) {
                            api.fetch_version_details(&brief.url).await
                        } else {
                            Err(format!("Minecraft version '{}' not found in Mojang manifest.", version_id_clone))
                        }
                    }
                    Err(e) => Err(e),
                }
            };

            let details = match details_res {
                Ok(d) => d,
                Err(e) => {
                    self.status_message = Some((format!("Failed to retrieve version details: {}", e), true));
                    return;
                }
            };

            if !version_json_path.exists() {
                if let Some(parent) = version_json_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(content) = serde_json::to_string_pretty(&details) {
                    let _ = std::fs::write(&version_json_path, content);
                }
            }

            let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
            let game_dir = self.config.game_dir.clone();
            let details_clone = details.clone();
            tokio::spawn(async move {
                let downloader = Downloader::new(tx);
                let _ = downloader.download_version(&game_dir, &details_clone).await;
            });

            self.state = AppState::Downloading {
                completed: 0,
                total: 100,
                current_file: String::new(),
                message: format!("Installing game files for {}...", version_id),
                logs: Vec::new(),
                rx,
                version_details: details,
            };
            self.status_message = None;
            return;
        }

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

        let logo = " ✦ MineCLI Terminal Launcher ✦ ".to_string();
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
        let menu_items = ["[D] Dashboard",
            "[I] Instances",
            "[A] Accounts",
            "[S] Settings"];

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
            if let Some(crash) = crash_analysis
                && let Some(area) = crash_area {
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

    fn draw_instances(&mut self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
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

        f.render_stateful_widget(list, list_chunks[0], &mut self.instances_list_state);

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
                Span::raw(" Import Modpack (.mrpack)  "),
                Span::styled("[M]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), 
                Span::raw(" Manage Mods   "),
                Span::styled("[A]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)), 
                Span::raw(" Asset Manager (Saves/Packs/Screenshots)"),
            ]),
        ];
        
        let help_p = Paragraph::new(help_text)
            .block(Block::default().title(" Instance Actions ").borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        f.render_widget(help_p, chunks[1]);
    }

    fn draw_accounts(&mut self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
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

        f.render_stateful_widget(list, chunks[0], &mut self.account_list_state);

        let help_text = vec![
            Line::from(vec![Span::styled(" [A]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Add Account               "), Span::styled("[O]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)), Span::raw(" Add Offline Profile")]),
            Line::from(vec![Span::styled(" [Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)), Span::raw(" Select Profile Active     "), Span::styled("[X]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)), Span::raw(" Delete Profile")]),
        ];
        
        let help_p = Paragraph::new(help_text)
            .block(Block::default().title(" Profile Actions ").borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(100, 100, 120))));
        f.render_widget(help_p, chunks[1]);
    }

    fn draw_settings(&mut self, f: &mut ratatui::Frame, rect: Rect, border_color: Color, select_color: Color) {
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

        f.render_stateful_widget(list, chunks[0], &mut self.settings_list_state);

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

    fn draw_overlays(&mut self, f: &mut ratatui::Frame, size: Rect, select_color: Color, border_color: Color) {
        match &mut self.state {
            AppState::Normal => {}
            
            AppState::AddOfflineAccount => {
                let area = get_centered_rect_helper(50, 20, size);
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
                let area = get_centered_rect_helper(50, 20, size);
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

            AppState::ModpackMenu { selected_option } => {
                let selected_option = *selected_option;
                let area = get_centered_rect_helper(50, 25, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Import Modpack Option ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(4),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                f.render_widget(Paragraph::new("Select how you want to import the modpack:").style(Style::default().fg(Color::Rgb(180, 180, 180))), chunks[0]);

                let opt0 = if selected_option == 0 { "-> [1] Search Modrinth for Modpacks" } else { "   [1] Search Modrinth for Modpacks" };
                let opt1 = if selected_option == 1 { "-> [2] Import Local .mrpack File" } else { "   [2] Import Local .mrpack File" };
                
                let style0 = if selected_option == 0 { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) };
                let style1 = if selected_option == 1 { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) };

                let text = vec![
                    Line::from(Span::styled(opt0, style0)),
                    Line::from(Span::styled(opt1, style1)),
                ];
                f.render_widget(Paragraph::new(text), chunks[1]);

                let help = Paragraph::new("Press [Up/Down] to navigate, [Enter] to select, [Esc] to cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, chunks[2]);
            }

            AppState::SearchingModpackQuery => {
                let area = get_centered_rect_helper(60, 20, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Search Modrinth Modpacks ")
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

                f.render_widget(Paragraph::new("Enter modpack name or query:"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Search, [Esc] to Back")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::SearchingModpackLoading { query, .. } => {
                let area = get_centered_rect_helper(50, 15, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Searching Modrinth ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let p = Paragraph::new(format!("\nSearching for \"{}\"...\nPlease wait.", query))
                    .style(Style::default().fg(Color::White))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(p, inner);
            }

            AppState::SearchingModpackResults { query, hits, list_state } => {
                let area = get_centered_rect_helper(80, 80, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(format!(" Modrinth Search: \"{}\" ", query))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(10),
                        Constraint::Length(3),
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = hits.iter().map(|hit| {
                    let item_line = Line::from(vec![
                        Span::styled(format!(" {:<30}", hit.title), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" By: {:<15}", hit.author), Style::default().fg(Color::Rgb(150, 150, 150))),
                        Span::styled(format!(" Downloads: {:<12}", hit.downloads), Style::default().fg(Color::Cyan)),
                    ]);
                    ListItem::new(item_line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Matching Modpacks ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let help = Paragraph::new("Press [Up/Down] to navigate, [Enter] to select version, [Esc] to Search Query")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, chunks[1]);
            }

            AppState::SearchingModpackVersionsLoading { hit, .. } => {
                let area = get_centered_rect_helper(50, 15, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Fetching Versions ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let p = Paragraph::new(format!("\nFetching versions for \"{}\"...\nPlease wait.", hit.title))
                    .style(Style::default().fg(Color::White))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(p, inner);
            }

            AppState::SearchingModpackVersions { hit, versions, list_state } => {
                let area = get_centered_rect_helper(75, 75, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(format!(" Versions for \"{}\" ", hit.title))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(10),
                        Constraint::Length(3),
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = versions.iter().map(|ver| {
                    let game_vers = ver.game_versions.join(", ");
                    let item_line = Line::from(vec![
                        Span::styled(format!(" {:<30}", ver.name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" Game Versions: {}", game_vers), Style::default().fg(Color::Cyan)),
                    ]);
                    ListItem::new(item_line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Available Versions ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let help = Paragraph::new("Press [Up/Down] to navigate, [Enter] to download/import, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, chunks[1]);
            }

            AppState::SearchingModpackConfirmId { hit, version, .. } => {
                let area = get_centered_rect_helper(50, 22, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(format!(" Import Modpack: {} ", hit.title))
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

                f.render_widget(Paragraph::new(format!("Version: {} (for MC {})", version.version_number, version.game_versions.join(", "))).style(Style::default().fg(Color::Cyan)), inner_layout[0]);
                f.render_widget(Paragraph::new("Enter desired folder name/ID for this modpack:"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Download & Import, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::ExportingModpackPath { instance_idx } => {
                let instance_idx = *instance_idx;
                let area = get_centered_rect_helper(60, 20, size);
                f.render_widget(Clear, area);
                
                let inst_name = self.instances.get(instance_idx).map(|i| i.config.name.as_str()).unwrap_or("Instance");
                let block = Block::default()
                    .title(format!(" Export '{}' to Modpack (.mrpack) ", inst_name))
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

                f.render_widget(Paragraph::new("Enter output file path (e.g. /path/to/my-pack.mrpack):"), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Export, [Esc] to Cancel")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::ImportingModpackPath => {
                let area = get_centered_rect_helper(60, 20, size);
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
                let area = get_centered_rect_helper(50, 20, size);
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

            AppState::CreatingInstanceVersion { id, name } => {
                let area = get_centered_rect_helper(80, 80, size);
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

                f.render_stateful_widget(list, chunks[1], &mut self.version_list_state);
            }

            AppState::AddMicrosoftAccount { user_code, verification_uri, .. } => {
                let area = get_centered_rect_helper(65, 35, size);
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

            AppState::EditingSetting { input_value, .. } => {
                let area = get_centered_rect_helper(60, 20, size);
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
                let area = get_centered_rect_helper(55, 12, size);
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

            AppState::EditingInstanceSetting { field_idx, input_value, .. } => {
                let area = get_centered_rect_helper(65, 12, size);
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

            AppState::Downloading { completed, total, current_file, message, logs, .. } => {
                let completed = *completed;
                let total = *total;
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

            AppState::SyncingInstanceMods { completed, total, current_file, message, logs, .. } => {
                let completed = *completed;
                let total = *total;
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

            AppState::ImportingModpackProgress { completed, total, current_file, message, logs, .. } => {
                let completed = *completed;
                let total = *total;
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

            AppState::BackupsMenu { instance_idx, backups, backups_list_state } => {
                let instance_idx = *instance_idx;
                let area = get_centered_rect_helper(70, 70, size);
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

                f.render_stateful_widget(list, chunks[0], backups_list_state);

                let help_p = Paragraph::new("Press [B] to Create New Backup\nPress [Enter] to Restore Selected Backup\nPress [Esc] to Close Menu")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);
            }
            AppState::ChoosingModLoader { instance_id: _, instance_name, game_version, loader_options, loader_list_state } => {
                let area = get_centered_rect_helper(60, 60, size);
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

                f.render_stateful_widget(list, chunks[0], loader_list_state);

                let help_p = Paragraph::new("Press [Enter] to Select, [Esc] to Skip (Vanilla)")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);
            }

            AppState::AssetManager { instance_idx, selected_category, has_shader_support } => {
                let instance_idx = *instance_idx;
                let selected_category = *selected_category;
                let has_shader_support = *has_shader_support;
                let area = get_centered_rect_helper(60, 40, size);
                f.render_widget(Clear, area);

                let inst_name = self.instances.get(instance_idx)
                    .map(|i| i.config.name.clone())
                    .unwrap_or_else(|| "?".to_string());

                let block = Block::default()
                    .title(format!(" Asset Manager — {} ", inst_name))
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

                let categories = vec![
                    "📁 Worlds & Save Games",
                    "🎨 Resource Packs",
                    "🕶️ Shader Packs",
                    "📷 Screenshots",
                ];

                let list_items: Vec<ListItem> = categories.iter().enumerate().map(|(idx, cat)| {
                    let mut text = vec![Span::styled(*cat, Style::default().fg(Color::White))];
                    if idx == 2 {
                        let support_span = if has_shader_support {
                            Span::styled(" (Support: Enabled)", Style::default().fg(Color::Green))
                        } else {
                            Span::styled(" (Support: Disabled)", Style::default().fg(Color::Red))
                        };
                        text.push(support_span);
                    }
                    ListItem::new(Line::from(text))
                }).collect();

                let mut category_list_state = ListState::default();
                category_list_state.select(Some(selected_category));

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Categories ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], &mut category_list_state);

                let help_text = "Press [Up/Down] to navigate, [Enter] to select, [Esc] to back";
                let help_p = Paragraph::new(help_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);
            }

            AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete } => {
                let instance_idx = *instance_idx;
                let area = get_centered_rect_helper(80, 80, size);
                f.render_widget(Clear, area);

                let inst_name = self.instances.get(instance_idx)
                    .map(|i| i.config.name.clone())
                    .unwrap_or_else(|| "?".to_string());

                let block = Block::default()
                    .title(format!(" World Manager — {} ({} worlds) ", inst_name, worlds.len()))
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

                let list_items: Vec<ListItem> = worlds.iter().map(|w| {
                    let size_mb = (w.size_bytes as f64) / (1024.0 * 1024.0);
                    let line = Line::from(vec![
                        Span::styled(" • ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled(&w.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" ({})", w.folder_name), Style::default().fg(Color::Rgb(120, 120, 130))),
                        Span::styled(format!("  Size: {:.2} MB", size_mb), Style::default().fg(Color::Yellow)),
                        Span::styled(format!("  Played: {}", w.last_played), Style::default().fg(Color::Rgb(160, 160, 170))),
                    ]);
                    ListItem::new(line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Worlds / Saves ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let help_text = "Press [Up/Down] to navigate, [a] to add datapack, [p] to pre-provision, [b] to backup, [r] to rename, [d] to delete, [Esc] to back";
                let help_p = Paragraph::new(help_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);

                if let Some(world_name) = confirm_delete.as_ref() {
                    let confirm_area = get_centered_rect_helper(50, 25, size);
                    f.render_widget(Clear, confirm_area);
                    let confirm_block = Block::default()
                        .title(" Confirm World Deletion ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
                    f.render_widget(confirm_block.clone(), confirm_area);
                    
                    let confirm_inner = confirm_area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                    let confirm_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1),
                            Constraint::Min(4),
                            Constraint::Length(2),
                        ])
                        .split(confirm_inner);

                    f.render_widget(Paragraph::new("⚠️  CRITICAL WARNING  ⚠️").alignment(ratatui::layout::Alignment::Center).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)), confirm_chunks[0]);
                    
                    let warn_msg = format!(
                        "Are you absolutely sure you want to permanently delete the world:\n\n  \"{}\"\n\nThis action is IRREVERSIBLE!",
                        world_name
                    );
                    f.render_widget(Paragraph::new(warn_msg).wrap(Wrap { trim: true }).alignment(ratatui::layout::Alignment::Center), confirm_chunks[1]);
                    
                    f.render_widget(Paragraph::new("Press [y/Y] to confirm permanent deletion, [any other key] to cancel").style(Style::default().fg(Color::Rgb(160, 160, 170))).alignment(ratatui::layout::Alignment::Center), confirm_chunks[2]);
                }
            }

            AppState::PromptWorldNameForDatapack { instance_idx, input_value } => {
                let _instance_idx = *instance_idx;
                let area = get_centered_rect_helper(50, 25, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Pre-provision Datapack ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(3),
                        Constraint::Min(2),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                f.render_widget(Paragraph::new("Enter the name of the new world to create/pre-provision:")
                    .style(Style::default().fg(Color::White)), chunks[0]);

                let input_widget = Paragraph::new(format!(" > {}", input_value))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)))
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
                f.render_widget(input_widget, chunks[1]);

                let info_msg = "MineCLI will automatically create the required folder structure:\nsaves/<world_name>/datapacks/\nand download the chosen datapack there.";
                f.render_widget(Paragraph::new(info_msg).style(Style::default().fg(Color::Rgb(150, 150, 160))), chunks[2]);

                let help_p = Paragraph::new("Press [Enter] to Search/Install, [Esc] to Cancel")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(120, 120, 130)));
                f.render_widget(help_p, chunks[3]);
            }

            AppState::ResourcePackManager { instance_idx, packs, list_state } => {
                let instance_idx = *instance_idx;
                let area = get_centered_rect_helper(80, 80, size);
                f.render_widget(Clear, area);

                let inst_name = self.instances.get(instance_idx)
                    .map(|i| i.config.name.clone())
                    .unwrap_or_else(|| "?".to_string());

                let block = Block::default()
                    .title(format!(" Resource Packs — {} ({} packs) ", inst_name, packs.len()))
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

                let list_items: Vec<ListItem> = packs.iter().map(|p| {
                    let status = if p.enabled { "✓" } else { "✗" };
                    let status_color = if p.enabled { Color::Green } else { Color::Red };
                    let size_mb = (p.size_bytes as f64) / (1024.0 * 1024.0);
                    
                    let mut lines = vec![
                        Line::from(vec![
                            Span::styled(format!(" [{}] ", status), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                            Span::styled(&p.filename, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                            Span::styled(format!("  Size: {:.2} MB", size_mb), Style::default().fg(Color::Rgb(140, 140, 150))),
                        ])
                    ];

                    if let Some(ref desc) = p.description {
                        lines.push(Line::from(vec![
                            Span::styled(format!("      ↳ {}", desc), Style::default().fg(Color::Rgb(170, 170, 180))),
                        ]));
                    }

                    ListItem::new(lines)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Installed Resource Packs ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let help_text = "Press [Space/Enter] to toggle, [a] to search/add, [r] to rename, [d/Backspace] to delete, [Esc] to back";
                let help_p = Paragraph::new(help_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);
            }

            AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support } => {
                let instance_idx = *instance_idx;
                let has_shader_support = *has_shader_support;
                let area = get_centered_rect_helper(80, 80, size);
                f.render_widget(Clear, area);

                let inst_name = self.instances.get(instance_idx)
                    .map(|i| i.config.name.clone())
                    .unwrap_or_else(|| "?".to_string());

                let block = Block::default()
                    .title(format!(" Shader Pack Manager — {} ({} shaders) ", inst_name, shaders.len()))
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
                        Constraint::Length(2),
                    ])
                    .split(inner);

                let (status_banner, banner_style) = if has_shader_support {
                    ("✓ Shader support is enabled for this instance. Shaders will load successfully in-game.", Style::default().fg(Color::Green))
                } else {
                    ("⚠️ Shader support is not enabled. Press [i/I] to automatically install Iris/Sodium or Oculus/Embeddium mods.", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                };
                let status_p = Paragraph::new(status_banner)
                    .wrap(Wrap { trim: true })
                    .block(Block::default().borders(Borders::ALL).title(" Compatibility Status ").border_style(banner_style));
                f.render_widget(status_p, chunks[0]);

                let list_items: Vec<ListItem> = shaders.iter().map(|s| {
                    let status = if s.enabled { "✓" } else { "✗" };
                    let status_color = if s.enabled { Color::Green } else { Color::Red };
                    let size_mb = (s.size_bytes as f64) / (1024.0 * 1024.0);
                    let line = Line::from(vec![
                        Span::styled(format!(" [{}] ", status), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                        Span::styled(&s.filename, Style::default().fg(Color::White)),
                        Span::styled(format!("  Size: {:.2} MB", size_mb), Style::default().fg(Color::Rgb(140, 140, 150))),
                    ]);
                    ListItem::new(line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Installed Shaders ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[1], list_state);

                let help_text = if has_shader_support {
                    "Press [Space/Enter] to toggle, [a] to search/add, [r] to rename, [d/Backspace] to delete, [Esc] to back"
                } else {
                    "Press [i/I] to install shader compatibility mod, [Esc] to back"
                };
                let help_p = Paragraph::new(help_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[2]);
            }

            AppState::PromptRenameAsset { instance_idx: _, asset_type, old_filename: _, input_value } => {
                let area = get_centered_rect_helper(50, 25, size);
                f.render_widget(Clear, area);
                
                let title = match asset_type.as_str() {
                    "world" => " Rename World ",
                    "resourcepack" => " Rename Resource Pack ",
                    "shaderpack" => " Rename Shader Pack ",
                    _ => " Rename Asset ",
                };

                let block = Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(3),
                        Constraint::Min(2),
                        Constraint::Length(2),
                    ])
                    .split(inner);

                let label = match asset_type.as_str() {
                    "world" => "Enter new world folder name:",
                    "resourcepack" => "Enter new resource pack filename (excluding extension):",
                    "shaderpack" => "Enter new shader pack filename (excluding extension):",
                    _ => "Enter new name:",
                };
                f.render_widget(Paragraph::new(label)
                    .style(Style::default().fg(Color::White)), chunks[0]);

                let input_widget = Paragraph::new(format!(" > {}", input_value))
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)))
                    .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
                f.render_widget(input_widget, chunks[1]);

                let info_msg = "Renaming the file directly renames the pack for Minecraft.\nExtensions and enabled/disabled states are automatically preserved.";
                f.render_widget(Paragraph::new(info_msg).style(Style::default().fg(Color::Rgb(150, 150, 160))), chunks[2]);

                let help_p = Paragraph::new("Press [Enter] to Rename, [Esc] to Cancel")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(120, 120, 130)));
                f.render_widget(help_p, chunks[3]);
            }

            AppState::InstallingShaderSupport { instance_idx: _, completed, total, current_file, message, logs, rx: _ } => {
                let area = get_centered_rect_helper(80, 60, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(" Installing Shader Support Mods ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(4),
                    ])
                    .split(inner);

                f.render_widget(Paragraph::new(message.as_str()).style(Style::default().fg(Color::Cyan)), chunks[0]);

                let pct = if *total > 0 { (*completed as f64 / *total as f64 * 100.0) as u16 } else { 0 };
                let label = format!("{} / {} ({}%)", completed, total, pct);
                let gauge = Gauge::default()
                    .block(Block::default().borders(Borders::ALL).title(current_file.as_str()))
                    .gauge_style(Style::default().fg(Color::Green).bg(Color::Rgb(40, 40, 40)))
                    .percent(pct)
                    .label(label);
                f.render_widget(gauge, chunks[1]);

                let log_items: Vec<ListItem> = logs.iter().rev().take(10).map(|log| {
                    ListItem::new(Line::from(vec![
                        Span::styled(" • ", Style::default().fg(Color::Green)),
                        Span::styled(log, Style::default().fg(Color::Rgb(180, 180, 180))),
                    ]))
                }).collect();

                let logs_list = List::new(log_items)
                    .block(Block::default().borders(Borders::ALL).title(" Installation Log ").border_style(Style::default().fg(border_color)));
                f.render_widget(logs_list, chunks[2]);
            }

            AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete } => {
                let instance_idx = *instance_idx;
                let area = get_centered_rect_helper(80, 80, size);
                f.render_widget(Clear, area);

                let inst_name = self.instances.get(instance_idx)
                    .map(|i| i.config.name.clone())
                    .unwrap_or_else(|| "?".to_string());

                let block = Block::default()
                    .title(format!(" Screenshot Manager — {} ({} screenshots) ", inst_name, screenshots.len()))
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

                let list_items: Vec<ListItem> = screenshots.iter().map(|s| {
                    let size_kb = (s.size_bytes as f64) / 1024.0;
                    let line = Line::from(vec![
                        Span::styled(" • ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                        Span::styled(&s.filename, Style::default().fg(Color::White)),
                        Span::styled(format!("  Size: {:.1} KB", size_kb), Style::default().fg(Color::Yellow)),
                        Span::styled(format!("  Created: {}", s.created), Style::default().fg(Color::Rgb(160, 160, 170))),
                    ]);
                    ListItem::new(line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Screenshots ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let help_text = "Press [d] to delete screenshot, [Esc] to back";
                let help_p = Paragraph::new(help_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[1]);

                if let Some(screenshot_name) = confirm_delete.as_ref() {
                    let confirm_area = get_centered_rect_helper(50, 20, size);
                    f.render_widget(Clear, confirm_area);
                    let confirm_block = Block::default()
                        .title(" Confirm Deletion ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
                    f.render_widget(confirm_block.clone(), confirm_area);
                    
                    let confirm_inner = confirm_area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                    let confirm_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Min(4),
                            Constraint::Length(2),
                        ])
                        .split(confirm_inner);

                    let warn_msg = format!(
                        "Are you sure you want to permanently delete screenshot:\n\n  \"{}\"",
                        screenshot_name
                    );
                    f.render_widget(Paragraph::new(warn_msg).wrap(Wrap { trim: true }).alignment(ratatui::layout::Alignment::Center), confirm_chunks[0]);
                    
                    f.render_widget(Paragraph::new("Press [y/Y] to confirm, [any other key] to cancel").style(Style::default().fg(Color::Rgb(160, 160, 170))).alignment(ratatui::layout::Alignment::Center), confirm_chunks[1]);
                }
            }

            AppState::ModManager { instance_idx, mods, mod_list_state } => {
                let instance_idx = *instance_idx;
                let area = get_centered_rect_helper(80, 80, size);
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
                        Constraint::Length(2),
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

                f.render_stateful_widget(list, chunks[0], mod_list_state);

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

                let help_text = "Press [Space/Enter] to toggle, [a] to search/add mod, [d/Backspace] to delete, [Esc] to back";
                let help_p = Paragraph::new(help_text)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(help_p, chunks[2]);
            }

            AppState::SearchingModQuery { instance_idx: _, search_type } => {
                let (asset_title, asset_label) = match search_type {
                    AssetSearchType::Mod => ("Mods", "mod"),
                    AssetSearchType::Shader => ("Shaders", "shader"),
                    AssetSearchType::ResourcePack => ("Resource Packs", "resource pack"),
                    AssetSearchType::Datapack { .. } => ("Datapacks", "datapack"),
                };
                let area = get_centered_rect_helper(60, 20, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(format!(" Search Modrinth {} ", asset_title))
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

                f.render_widget(Paragraph::new(format!("Enter {} name or query:", asset_label)), inner_layout[1]);

                let input_p = Paragraph::new(self.version_search_query.clone()) 
                    .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Yellow)));
                f.render_widget(input_p, inner_layout[2]);

                let help = Paragraph::new("Press [Enter] to Search, [Esc] to Back")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, inner_layout[3]);
            }

            AppState::SearchingModLoading { query, search_type, .. } => {
                let asset_title = match search_type {
                    AssetSearchType::Mod => "Mods",
                    AssetSearchType::Shader => "Shaders",
                    AssetSearchType::ResourcePack => "Resource Packs",
                    AssetSearchType::Datapack { .. } => "Datapacks",
                };
                let area = get_centered_rect_helper(50, 15, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(format!(" Searching Modrinth {} ", asset_title))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let p = Paragraph::new(format!("\nSearching for \"{}\"...\nPlease wait.", query))
                    .style(Style::default().fg(Color::White))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(p, inner);
            }

            AppState::SearchingModResults { query, search_type, hits, list_state, .. } => {
                let asset_title = match search_type {
                    AssetSearchType::Mod => "Mods",
                    AssetSearchType::Shader => "Shaders",
                    AssetSearchType::ResourcePack => "Resource Packs",
                    AssetSearchType::Datapack { .. } => "Datapacks",
                };
                let area = get_centered_rect_helper(85, 85, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(format!(" Modrinth {} Search: \"{}\" ", asset_title, query))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(10),
                        Constraint::Length(6), 
                        Constraint::Length(3),
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = hits.iter().map(|hit| {
                    let item_line = Line::from(vec![
                        Span::styled(format!(" {:<30}", hit.title), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" By: {:<15}", hit.author), Style::default().fg(Color::Rgb(150, 150, 150))),
                        Span::styled(format!(" Downloads: {:<12}", hit.downloads), Style::default().fg(Color::Cyan)),
                    ]);
                    ListItem::new(item_line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(format!(" Matching {} ", asset_title)).border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let desc_text = if let Some(idx) = list_state.selected() {
                    if let Some(hit) = hits.get(idx) {
                        format!("{}\n{}", hit.title, hit.description)
                    } else {
                        String::new()
                    }
                } else {
                    format!("No {} selected.", asset_title.to_lowercase())
                };

                let desc_p = Paragraph::new(desc_text)
                    .style(Style::default().fg(Color::Rgb(180, 180, 200)))
                    .wrap(Wrap { trim: true })
                    .block(Block::default().borders(Borders::ALL).title(format!(" {} Info ", asset_title)).border_style(Style::default().fg(border_color)));
                f.render_widget(desc_p, chunks[1]);

                let help = Paragraph::new("Press [Up/Down] to navigate, [Enter] to select version, [Esc] to Search Query")
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, chunks[2]);
            }

            AppState::SearchingModVersionsLoading { hit, .. } => {
                let area = get_centered_rect_helper(50, 15, size);
                f.render_widget(Clear, area);
                
                let block = Block::default()
                    .title(" Fetching Versions ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let p = Paragraph::new(format!("\nFetching versions for \"{}\"...\nPlease wait.", hit.title))
                    .style(Style::default().fg(Color::White))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(p, inner);
            }

            AppState::SearchingModVersions { hit, search_type, versions, list_state, .. } => {
                let asset_label = match search_type {
                    AssetSearchType::Mod => "mod",
                    AssetSearchType::Shader => "shader pack",
                    AssetSearchType::ResourcePack => "resource pack",
                    AssetSearchType::Datapack { .. } => "datapack",
                };
                let area = get_centered_rect_helper(75, 75, size);
                f.render_widget(Clear, area);

                let block = Block::default()
                    .title(format!(" Compatible Versions for \"{}\" ", hit.title))
                    .borders(Borders::ALL)
                    .border_type(BorderType::Double)
                    .border_style(Style::default().fg(select_color));
                f.render_widget(block, area);

                let inner = area.inner(&ratatui::layout::Margin { horizontal: 2, vertical: 1 });
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Min(10),
                        Constraint::Length(3),
                    ])
                    .split(inner);

                let list_items: Vec<ListItem> = versions.iter().map(|ver| {
                    let game_vers = ver.game_versions.join(", ");
                    let loaders = ver.loaders.join(", ");
                    let item_line = Line::from(vec![
                        Span::styled(format!(" {:<30}", ver.name), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" Game Versions: {} | Loaders: {}", game_vers, loaders), Style::default().fg(Color::Cyan)),
                    ]);
                    ListItem::new(item_line)
                }).collect();

                let list = List::new(list_items)
                    .block(Block::default().borders(Borders::ALL).title(" Available Versions ").border_style(Style::default().fg(border_color)))
                    .highlight_style(Style::default().bg(select_color).fg(Color::White).add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[0], list_state);

                let help = Paragraph::new(format!("Press [Up/Down] to navigate, [Enter] to install {}, [Esc] to Cancel", asset_label))
                    .style(Style::default().fg(Color::Rgb(150, 150, 150)))
                    .alignment(ratatui::layout::Alignment::Center);
                f.render_widget(help, chunks[1]);
            }

            AppState::InstallingModProgress { completed, total, current_file, message, search_type, .. } => {
                let completed = *completed;
                let total = *total;
                let asset_name = match search_type {
                    AssetSearchType::Mod => "Mod",
                    AssetSearchType::Shader => "Shader Pack",
                    AssetSearchType::ResourcePack => "Resource Pack",
                    AssetSearchType::Datapack { .. } => "Datapack",
                };
                f.render_widget(Clear, size);
                
                let block = Block::default()
                    .title(format!(" Installing {} ", asset_name))
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

                let status_card = Paragraph::new(format!("\nPlease wait while MineCLI downloads and\ninstalls the selected {}.", asset_name.to_lowercase()))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().fg(Color::Rgb(150, 150, 160)));
                f.render_widget(status_card, chunks[2]);
            }

            AppState::GameRunning { .. } => {}
        }
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

            AppState::ModpackMenu { ref mut selected_option } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if *selected_option > 0 {
                            *selected_option -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if *selected_option < 1 {
                            *selected_option += 1;
                        }
                    }
                    KeyCode::Enter => {
                        if *selected_option == 0 {
                            self.state = AppState::SearchingModpackQuery;
                        } else {
                            self.state = AppState::ImportingModpackPath;
                        }
                        self.version_search_query.clear();
                    }
                    _ => {}
                }
            }

            AppState::SearchingModpackQuery => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::ModpackMenu { selected_option: 0 };
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let query = self.version_search_query.trim().to_string();
                        if !query.is_empty() {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            let client = self.api_client.clone();
                            let query_clone = query.clone();
                            tokio::spawn(async move {
                                let res = client.search_modpacks(&query_clone).await;
                                let _ = tx.send(res);
                            });
                            self.state = AppState::SearchingModpackLoading { query, rx };
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

            AppState::SearchingModpackLoading { .. } => {}

            AppState::SearchingModpackResults { ref query, ref hits, ref mut list_state } => {
                match key.code {
                    KeyCode::Esc => {
                        let q = query.clone();
                        self.state = AppState::SearchingModpackQuery;
                        self.version_search_query = q;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected + 1 < hits.len() {
                            list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = list_state.selected()
                            && let Some(hit) = hits.get(idx) {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                let client = self.api_client.clone();
                                let project_id = hit.project_id.clone();
                                tokio::spawn(async move {
                                    let res = client.fetch_modpack_versions(&project_id).await;
                                    let _ = tx.send(res);
                                });
                                self.state = AppState::SearchingModpackVersionsLoading { hit: hit.clone(), rx };
                            }
                    }
                    _ => {}
                }
            }

            AppState::SearchingModpackVersionsLoading { .. } => {}

            AppState::SearchingModpackVersions { ref hit, ref versions, ref mut list_state } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected + 1 < versions.len() {
                            list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = list_state.selected()
                            && let Some(version) = versions.get(idx) {
                                if let Some(file) = version.files.iter().find(|f| f.primary || f.filename.ends_with(".mrpack")) {
                                    let url = file.url.clone();
                                    let filename = file.filename.clone();
                                    let default_id = hit.title.to_lowercase().replace(' ', "-");
                                    self.version_search_query = default_id;
                                    self.state = AppState::SearchingModpackConfirmId {
                                        hit: hit.clone(),
                                        version: version.clone(),
                                        url,
                                        filename,
                                    };
                                } else {
                                    self.status_message = Some(("No primary .mrpack file found in this version.".to_string(), true));
                                }
                            }
                    }
                    _ => {}
                }
            }

            AppState::SearchingModpackConfirmId { hit: _, version: _, ref url, ref filename } => {
                let url = url.clone();
                let filename = filename.clone();
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let custom_id = self.version_search_query.trim().to_string();
                        if !custom_id.is_empty() {
                            let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                            let game_dir = self.config.game_dir.clone();
                            let url_clone = url.clone();
                            let filename_clone = filename.clone();
                            let custom_id_clone = custom_id.clone();
                            
                            tokio::spawn(async move {
                                let temp_dir = game_dir.join("cache").join("temp_packs");
                                let _ = fs::create_dir_all(&temp_dir);
                                let dest_path = temp_dir.join(&filename_clone);
                                
                                let _ = tx.send(ProgressUpdate::Started {
                                    total: 100,
                                    message: format!("Downloading modpack {}...", filename_clone),
                                }).await;
                                
                                let client = reqwest::Client::new();
                                let res = match client.get(&url_clone).header("User-Agent", "minecli/0.1.0").send().await {
                                    Ok(r) => r,
                                    Err(e) => {
                                        let _ = tx.send(ProgressUpdate::Error(format!("Failed to download modpack: {}", e))).await;
                                        return;
                                    }
                                };
                                
                                let total_size = res.content_length().unwrap_or(1);
                                let mut file = match File::create(&dest_path) {
                                    Ok(f) => f,
                                    Err(e) => {
                                        let _ = tx.send(ProgressUpdate::Error(format!("Failed to create temp file: {}", e))).await;
                                        return;
                                    }
                                };
                                
                                let mut bytes_stream = res.bytes_stream();
                                use futures_util::StreamExt;
                                let mut downloaded = 0;
                                while let Some(chunk_res) = bytes_stream.next().await {
                                    let chunk = match chunk_res {
                                        Ok(c) => c,
                                        Err(e) => {
                                            let _ = tx.send(ProgressUpdate::Error(format!("Error while streaming download: {}", e))).await;
                                            return;
                                        }
                                    };
                                    if let Err(e) = std::io::copy(&mut chunk.as_ref(), &mut file) {
                                        let _ = tx.send(ProgressUpdate::Error(format!("Failed to write chunk: {}", e))).await;
                                        return;
                                    }
                                    downloaded += chunk.len() as u64;
                                    let pct = ((downloaded * 100) / total_size) as usize;
                                    let _ = tx.send(ProgressUpdate::Progress {
                                        completed: pct,
                                        total: 100,
                                        current_file: filename_clone.clone(),
                                    }).await;
                                }
                                
                                let _ = Instance::import_mrpack(&game_dir, &dest_path, &custom_id_clone, &tx).await;
                            });

                            self.version_search_query.clear();
                            self.state = AppState::ImportingModpackProgress {
                                completed: 0,
                                total: 100,
                                current_file: String::new(),
                                message: "Downloading pack...".to_string(),
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

            AppState::ExportingModpackPath { instance_idx } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let path_str = self.version_search_query.trim().to_string();
                        if !path_str.is_empty() {
                            let path = std::path::Path::new(&path_str);
                            if let Some(inst) = self.instances.get(instance_idx) {
                                match inst.export_mrpack(path) {
                                    Ok(()) => {
                                        self.status_message = Some((format!("Successfully exported modpack to {}", path_str), false));
                                    }
                                    Err(e) => {
                                        self.status_message = Some((format!("Failed to export modpack: {}", e), true));
                                    }
                                }
                            }
                            self.state = AppState::Normal;
                            self.version_search_query.clear();
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
                        if let Some(idx) = self.version_list_state.selected()
                            && let Some(brief) = self.filtered_version_briefs.get(idx).cloned() {
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
                                            match launcher.load_version_details_raw(&version_id) {
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
                                                        version_details: launcher.load_version_details_raw(&version_id).unwrap(),
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

            AppState::AssetManager { instance_idx, selected_category, has_shader_support } => {
                match key.code {
                    KeyCode::Esc => {
                        self.state = AppState::Normal;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let next_cat = if selected_category > 0 { selected_category - 1 } else { 3 };
                        self.state = AppState::AssetManager { instance_idx, selected_category: next_cat, has_shader_support };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let next_cat = if selected_category < 3 { selected_category + 1 } else { 0 };
                        self.state = AppState::AssetManager { instance_idx, selected_category: next_cat, has_shader_support };
                    }
                    KeyCode::Enter => {
                        if let Some(inst) = self.instances.get(instance_idx) {
                            match selected_category {
                                0 => {
                                    let worlds = crate::assets::list_worlds(&inst.path).unwrap_or_default();
                                    let mut state = ListState::default();
                                    if !worlds.is_empty() {
                                        state.select(Some(0));
                                    }
                                    self.state = AppState::WorldManager { instance_idx, worlds, list_state: state, confirm_delete: None };
                                }
                                1 => {
                                    let packs = crate::assets::list_resourcepacks(&inst.path).unwrap_or_default();
                                    let mut state = ListState::default();
                                    if !packs.is_empty() {
                                        state.select(Some(0));
                                    }
                                    self.state = AppState::ResourcePackManager { instance_idx, packs, list_state: state };
                                }
                                2 => {
                                    let shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                                    let mut state = ListState::default();
                                    if !shaders.is_empty() {
                                        state.select(Some(0));
                                    }
                                    self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state: state, has_shader_support };
                                }
                                3 => {
                                    let screenshots = crate::assets::list_screenshots(&inst.path).unwrap_or_default();
                                    let mut state = ListState::default();
                                    if !screenshots.is_empty() {
                                        state.select(Some(0));
                                    }
                                    self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state: state, confirm_delete: None };
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }

            AppState::WorldManager { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::WorldManager { instance_idx, mut worlds, mut list_state, confirm_delete } = state {
                    if let Some(ref world_name) = confirm_delete {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(inst) = self.instances.get(instance_idx) {
                                    if let Err(e) = crate::assets::delete_world(&inst.path, world_name) {
                                        self.status_message = Some((format!("Failed to delete world: {}", e), true));
                                    } else {
                                        self.status_message = Some((format!("Deleted world '{}'", world_name), false));
                                    }
                                    worlds = crate::assets::list_worlds(&inst.path).unwrap_or_default();
                                    let new_selected = list_state.selected().map(|s| s.min(worlds.len().saturating_sub(1)));
                                    list_state.select(new_selected);
                                }
                                self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                            }
                            _ => {
                                self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                            }
                        }
                    } else {
                        match key.code {
                            KeyCode::Esc => {
                                let has_shader_support = self.instances.get(instance_idx)
                                    .map(|inst| crate::assets::detect_shader_support(&inst.path))
                                    .unwrap_or(false);
                                self.state = AppState::AssetManager { instance_idx, selected_category: 0, has_shader_support };
                            }
                            KeyCode::Up => {
                                let selected = list_state.selected().unwrap_or(0);
                                if selected > 0 {
                                    list_state.select(Some(selected - 1));
                                }
                                self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                            }
                            KeyCode::Down => {
                                let selected = list_state.selected().unwrap_or(0);
                                if selected + 1 < worlds.len() {
                                    list_state.select(Some(selected + 1));
                                }
                                self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(w) = worlds.get(selected) {
                                        let name_to_delete = w.folder_name.clone();
                                        self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: Some(name_to_delete) };
                                    } else {
                                        self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                    }
                                } else {
                                    self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                }
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') => {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(w) = worlds.get(selected) {
                                        self.version_search_query.clear();
                                        self.state = AppState::SearchingModQuery {
                                            instance_idx,
                                            search_type: AssetSearchType::Datapack { world_name: w.folder_name.clone() },
                                        };
                                    } else {
                                        self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                    }
                                } else {
                                    self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                }
                            }
                            KeyCode::Char('p') | KeyCode::Char('P') => {
                                self.state = AppState::PromptWorldNameForDatapack {
                                    instance_idx,
                                    input_value: String::new(),
                                };
                            }
                            KeyCode::Char('b') | KeyCode::Char('B') => {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(w) = worlds.get(selected) {
                                        if let Some(inst) = self.instances.get(instance_idx) {
                                            match crate::assets::backup_world(&inst.path, &w.folder_name) {
                                                Ok(path) => {
                                                    self.status_message = Some((format!("Backup created: {}", path.file_name().unwrap().to_string_lossy()), false));
                                                }
                                                Err(e) => {
                                                    self.status_message = Some((format!("Backup failed: {}", e), true));
                                                }
                                            }
                                        }
                                    }
                                }
                                self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                            }
                            KeyCode::Char('r') | KeyCode::Char('R') => {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(w) = worlds.get(selected) {
                                        self.state = AppState::PromptRenameAsset {
                                            instance_idx,
                                            asset_type: "world".to_string(),
                                            old_filename: w.folder_name.clone(),
                                            input_value: w.folder_name.clone(),
                                        };
                                    } else {
                                        self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                    }
                                } else {
                                    self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                }
                            }
                            _ => {
                                self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                            }
                        }
                    }
                }
            }

            AppState::PromptWorldNameForDatapack { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::PromptWorldNameForDatapack { instance_idx, mut input_value } = state {
                    match key.code {
                        KeyCode::Esc => {
                            let worlds = self.instances.get(instance_idx)
                                .map(|inst| crate::assets::list_worlds(&inst.path).unwrap_or_default())
                                .unwrap_or_default();
                            let mut list_state = ListState::default();
                            if !worlds.is_empty() {
                                list_state.select(Some(0));
                            }
                            self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                        }
                        KeyCode::Enter => {
                            let world_name = input_value.trim().to_string();
                            if !world_name.is_empty() {
                                self.version_search_query.clear();
                                self.state = AppState::SearchingModQuery {
                                    instance_idx,
                                    search_type: AssetSearchType::Datapack { world_name },
                                };
                            } else {
                                self.state = AppState::PromptWorldNameForDatapack { instance_idx, input_value };
                            }
                        }
                        KeyCode::Char(c) => {
                            input_value.push(c);
                            self.state = AppState::PromptWorldNameForDatapack { instance_idx, input_value };
                        }
                        KeyCode::Backspace => {
                            input_value.pop();
                            self.state = AppState::PromptWorldNameForDatapack { instance_idx, input_value };
                        }
                        _ => {
                            self.state = AppState::PromptWorldNameForDatapack { instance_idx, input_value };
                        }
                    }
                }
            }

            AppState::PromptRenameAsset { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::PromptRenameAsset { instance_idx, asset_type, old_filename, mut input_value } = state {
                    match key.code {
                        KeyCode::Esc => {
                            if let Some(inst) = self.instances.get(instance_idx) {
                                match asset_type.as_str() {
                                    "world" => {
                                        let worlds = crate::assets::list_worlds(&inst.path).unwrap_or_default();
                                        let mut list_state = ListState::default();
                                        if !worlds.is_empty() {
                                            list_state.select(Some(0));
                                        }
                                        self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                    }
                                    "resourcepack" => {
                                        let packs = crate::assets::list_resourcepacks(&inst.path).unwrap_or_default();
                                        let mut list_state = ListState::default();
                                        if !packs.is_empty() {
                                            list_state.select(Some(0));
                                        }
                                        self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                                    }
                                    "shaderpack" => {
                                        let shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                                        let mut list_state = ListState::default();
                                        if !shaders.is_empty() {
                                            list_state.select(Some(0));
                                        }
                                        let has_shader_support = crate::assets::detect_shader_support(&inst.path);
                                        self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                                    }
                                    _ => {
                                        self.state = AppState::Normal;
                                    }
                                }
                            } else {
                                self.state = AppState::Normal;
                            }
                        }
                        KeyCode::Enter => {
                            let new_name = input_value.trim().to_string();
                            if !new_name.is_empty() {
                                if let Some(inst) = self.instances.get(instance_idx) {
                                    match asset_type.as_str() {
                                        "world" => {
                                            let old_path = inst.path.join("saves").join(&old_filename);
                                            let new_path = inst.path.join("saves").join(&new_name);
                                            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                                                self.status_message = Some((format!("Rename failed: {}", e), true));
                                            } else {
                                                self.status_message = Some((format!("Renamed world to '{}'", new_name), false));
                                            }
                                            let worlds = crate::assets::list_worlds(&inst.path).unwrap_or_default();
                                            let mut list_state = ListState::default();
                                            if !worlds.is_empty() {
                                                list_state.select(Some(0));
                                            }
                                            self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                        }
                                        "resourcepack" => {
                                            let old_path = inst.path.join("resourcepacks").join(&old_filename);
                                            let is_disabled = old_filename.ends_with(".disabled");
                                            let base_old = if is_disabled {
                                                old_filename.strip_suffix(".disabled").unwrap_or(&old_filename)
                                            } else {
                                                &old_filename
                                            };
                                            let ext = std::path::Path::new(base_old).extension().and_then(|s| s.to_str()).unwrap_or("zip");
                                            let mut target_name = format!("{}.{}", new_name, ext);
                                            if is_disabled {
                                                target_name.push_str(".disabled");
                                            }
                                            let new_path = inst.path.join("resourcepacks").join(&target_name);
                                            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                                                self.status_message = Some((format!("Rename failed: {}", e), true));
                                            } else {
                                                self.status_message = Some((format!("Renamed pack to '{}'", target_name), false));
                                            }
                                            let packs = crate::assets::list_resourcepacks(&inst.path).unwrap_or_default();
                                            let mut list_state = ListState::default();
                                            if !packs.is_empty() {
                                                list_state.select(Some(0));
                                            }
                                            self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                                        }
                                        "shaderpack" => {
                                            let old_path = inst.path.join("shaderpacks").join(&old_filename);
                                            let is_disabled = old_filename.ends_with(".disabled");
                                            let base_old = if is_disabled {
                                                old_filename.strip_suffix(".disabled").unwrap_or(&old_filename)
                                            } else {
                                                &old_filename
                                            };
                                            let ext = std::path::Path::new(base_old).extension().and_then(|s| s.to_str()).unwrap_or("zip");
                                            let mut target_name = format!("{}.{}", new_name, ext);
                                            if is_disabled {
                                                target_name.push_str(".disabled");
                                            }
                                            let new_path = inst.path.join("shaderpacks").join(&target_name);
                                            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                                                self.status_message = Some((format!("Rename failed: {}", e), true));
                                            } else {
                                                self.status_message = Some((format!("Renamed shader to '{}'", target_name), false));
                                            }
                                            let shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                                            let mut list_state = ListState::default();
                                            if !shaders.is_empty() {
                                                list_state.select(Some(0));
                                            }
                                            let has_shader_support = crate::assets::detect_shader_support(&inst.path);
                                            self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                                        }
                                        _ => {
                                            self.state = AppState::Normal;
                                        }
                                    }
                                } else {
                                    self.state = AppState::Normal;
                                }
                            } else {
                                self.state = AppState::PromptRenameAsset { instance_idx, asset_type, old_filename, input_value };
                            }
                        }
                        KeyCode::Char(c) => {
                            input_value.push(c);
                            self.state = AppState::PromptRenameAsset { instance_idx, asset_type, old_filename, input_value };
                        }
                        KeyCode::Backspace => {
                            input_value.pop();
                            self.state = AppState::PromptRenameAsset { instance_idx, asset_type, old_filename, input_value };
                        }
                        _ => {
                            self.state = AppState::PromptRenameAsset { instance_idx, asset_type, old_filename, input_value };
                        }
                    }
                }
            }

            AppState::ResourcePackManager { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::ResourcePackManager { instance_idx, mut packs, mut list_state } = state {
                    match key.code {
                        KeyCode::Esc => {
                            let has_shader_support = self.instances.get(instance_idx)
                                .map(|inst| crate::assets::detect_shader_support(&inst.path))
                                .unwrap_or(false);
                            self.state = AppState::AssetManager { instance_idx, selected_category: 1, has_shader_support };
                        }
                        KeyCode::Up => {
                            let selected = list_state.selected().unwrap_or(0);
                            if selected > 0 {
                                list_state.select(Some(selected - 1));
                            }
                            self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                        }
                        KeyCode::Down => {
                            let selected = list_state.selected().unwrap_or(0);
                            if selected + 1 < packs.len() {
                                list_state.select(Some(selected + 1));
                            }
                            self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            if let Some(selected) = list_state.selected() {
                                if let Some(pack) = packs.get_mut(selected) {
                                    if let Some(inst) = self.instances.get(instance_idx) {
                                        if let Err(e) = crate::assets::toggle_resourcepack(&inst.path, &pack.filename) {
                                            self.status_message = Some((format!("Toggle failed: {}", e), true));
                                        } else {
                                            pack.enabled = !pack.enabled;
                                            pack.filename = if pack.enabled {
                                                pack.filename.strip_suffix(".disabled").unwrap_or(&pack.filename).to_string()
                                            } else {
                                                format!("{}.disabled", pack.filename)
                                            };
                                            self.status_message = Some((format!("Toggled {}", pack.filename), false));
                                        }
                                    }
                                }
                            }
                            self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                        }
                        KeyCode::Char('a') => {
                            self.version_search_query.clear();
                            self.state = AppState::SearchingModQuery { instance_idx, search_type: AssetSearchType::ResourcePack };
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            if let Some(selected) = list_state.selected() {
                                if let Some(pack) = packs.get(selected) {
                                    let is_disabled = pack.filename.ends_with(".disabled");
                                    let base_old = if is_disabled {
                                        pack.filename.strip_suffix(".disabled").unwrap_or(&pack.filename)
                                    } else {
                                        &pack.filename
                                    };
                                    let stem = std::path::Path::new(base_old).file_stem().and_then(|s| s.to_str()).unwrap_or(base_old).to_string();
                                    self.state = AppState::PromptRenameAsset {
                                        instance_idx,
                                        asset_type: "resourcepack".to_string(),
                                        old_filename: pack.filename.clone(),
                                        input_value: stem,
                                    };
                                } else {
                                    self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                                }
                            } else {
                                self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                            }
                        }
                        KeyCode::Char('d') | KeyCode::Backspace => {
                            if let Some(selected) = list_state.selected()
                                && let Some(pack) = packs.get(selected) {
                                    if let Some(inst) = self.instances.get(instance_idx) {
                                        let pack_path = inst.path.join("resourcepacks").join(&pack.filename);
                                        if std::fs::remove_file(&pack_path).is_ok() {
                                            self.status_message = Some((format!("Deleted resource pack '{}'", pack.filename), false));
                                        } else {
                                            self.status_message = Some(("Failed to delete resource pack".to_string(), true));
                                        }
                                        let new_packs = crate::assets::list_resourcepacks(&inst.path).unwrap_or_default();
                                        packs = new_packs;
                                        if selected >= packs.len() && !packs.is_empty() {
                                            list_state.select(Some(packs.len() - 1));
                                        } else if packs.is_empty() {
                                            list_state.select(None);
                                        }
                                    }
                                }
                            self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                        }
                        _ => {
                            self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                        }
                    }
                }
            }

            AppState::ShaderPackManager { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::ShaderPackManager { instance_idx, mut shaders, mut list_state, has_shader_support } = state {
                    match key.code {
                        KeyCode::Esc => {
                            self.state = AppState::AssetManager { instance_idx, selected_category: 2, has_shader_support };
                        }
                        KeyCode::Up => {
                            let selected = list_state.selected().unwrap_or(0);
                            if selected > 0 {
                                list_state.select(Some(selected - 1));
                            }
                            self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                        }
                        KeyCode::Down => {
                            let selected = list_state.selected().unwrap_or(0);
                            if selected + 1 < shaders.len() {
                                list_state.select(Some(selected + 1));
                            }
                            self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                        }
                        KeyCode::Enter | KeyCode::Char(' ') => {
                            if has_shader_support {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(shader) = shaders.get_mut(selected) {
                                        if let Some(inst) = self.instances.get(instance_idx) {
                                            if let Err(e) = crate::assets::toggle_shaderpack(&inst.path, &shader.filename) {
                                                self.status_message = Some((format!("Toggle failed: {}", e), true));
                                            } else {
                                                shader.enabled = !shader.enabled;
                                                shader.filename = if shader.enabled {
                                                    shader.filename.strip_suffix(".disabled").unwrap_or(&shader.filename).to_string()
                                                } else {
                                                    format!("{}.disabled", shader.filename)
                                                };
                                                self.status_message = Some((format!("Toggled {}", shader.filename), false));
                                            }
                                        }
                                    }
                                }
                            }
                            self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                        }
                        KeyCode::Char('i') | KeyCode::Char('I') => {
                            if !has_shader_support {
                                if let Some(inst) = self.instances.get(instance_idx) {
                                    let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                                    let game_dir = self.config.game_dir.clone();
                                    let mut inst_clone = inst.clone();
                                    
                                    tokio::spawn(async move {
                                        let _ = inst_clone.install_shader_support(&game_dir, tx).await;
                                    });

                                    self.state = AppState::InstallingShaderSupport {
                                        instance_idx,
                                        completed: 0,
                                        total: 100,
                                        current_file: String::new(),
                                        message: "Resolving shader mods...".to_string(),
                                        logs: Vec::new(),
                                        rx,
                                    };
                                } else {
                                    self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                                }
                            } else {
                                self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                            }
                        }
                        KeyCode::Char('a') => {
                            self.version_search_query.clear();
                            self.state = AppState::SearchingModQuery { instance_idx, search_type: AssetSearchType::Shader };
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            if let Some(selected) = list_state.selected() {
                                if let Some(shader) = shaders.get(selected) {
                                    let is_disabled = shader.filename.ends_with(".disabled");
                                    let base_old = if is_disabled {
                                        shader.filename.strip_suffix(".disabled").unwrap_or(&shader.filename)
                                    } else {
                                        &shader.filename
                                    };
                                    let stem = std::path::Path::new(base_old).file_stem().and_then(|s| s.to_str()).unwrap_or(base_old).to_string();
                                    self.state = AppState::PromptRenameAsset {
                                        instance_idx,
                                        asset_type: "shaderpack".to_string(),
                                        old_filename: shader.filename.clone(),
                                        input_value: stem,
                                    };
                                } else {
                                    self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                                }
                            } else {
                                self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                            }
                        }
                        KeyCode::Char('d') | KeyCode::Backspace => {
                            if let Some(selected) = list_state.selected()
                                && let Some(shader) = shaders.get(selected) {
                                    if let Some(inst) = self.instances.get(instance_idx) {
                                        let shader_path = inst.path.join("shaderpacks").join(&shader.filename);
                                        if std::fs::remove_file(&shader_path).is_ok() {
                                            self.status_message = Some((format!("Deleted shader pack '{}'", shader.filename), false));
                                        } else {
                                            self.status_message = Some(("Failed to delete shader pack".to_string(), true));
                                        }
                                        let new_shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                                        shaders = new_shaders;
                                        if selected >= shaders.len() && !shaders.is_empty() {
                                            list_state.select(Some(shaders.len() - 1));
                                        } else if shaders.is_empty() {
                                            list_state.select(None);
                                        }
                                    }
                                }
                            self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                        }
                        _ => {
                            self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                        }
                    }
                }
            }

            AppState::ScreenshotManager { .. } => {
                let state = std::mem::replace(&mut self.state, AppState::Normal);
                if let AppState::ScreenshotManager { instance_idx, mut screenshots, mut list_state, confirm_delete } = state {
                    if let Some(ref filename) = confirm_delete {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                if let Some(inst) = self.instances.get(instance_idx) {
                                    if let Err(e) = crate::assets::delete_screenshot(&inst.path, filename) {
                                        self.status_message = Some((format!("Failed to delete: {}", e), true));
                                    } else {
                                        self.status_message = Some(("Deleted screenshot".to_string(), false));
                                    }
                                    screenshots = crate::assets::list_screenshots(&inst.path).unwrap_or_default();
                                    let new_selected = list_state.selected().map(|s| s.min(screenshots.len().saturating_sub(1)));
                                    list_state.select(new_selected);
                                }
                                self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                            }
                            _ => {
                                self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                            }
                        }
                    } else {
                        match key.code {
                            KeyCode::Esc => {
                                let has_shader_support = self.instances.get(instance_idx)
                                    .map(|inst| crate::assets::detect_shader_support(&inst.path))
                                    .unwrap_or(false);
                                self.state = AppState::AssetManager { instance_idx, selected_category: 3, has_shader_support };
                            }
                            KeyCode::Up => {
                                let selected = list_state.selected().unwrap_or(0);
                                if selected > 0 {
                                    list_state.select(Some(selected - 1));
                                }
                                self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                            }
                            KeyCode::Down => {
                                let selected = list_state.selected().unwrap_or(0);
                                if selected + 1 < screenshots.len() {
                                    list_state.select(Some(selected + 1));
                                }
                                self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                            }
                            KeyCode::Char('d') | KeyCode::Char('D') => {
                                if let Some(selected) = list_state.selected() {
                                    if let Some(s) = screenshots.get(selected) {
                                        let name_to_delete = s.filename.clone();
                                        self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: Some(name_to_delete) };
                                    } else {
                                        self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                                    }
                                } else {
                                    self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                                }
                            }
                            _ => {
                                self.state = AppState::ScreenshotManager { instance_idx, screenshots, list_state, confirm_delete: None };
                            }
                        }
                    }
                }
            }

            AppState::InstallingShaderSupport { .. } => {}

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
                            if let Some(selected) = mod_list_state.selected()
                                && let Some(inst) = self.instances.get(instance_idx)
                                    && let Some(m) = mods.get_mut(selected) {
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
                            self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                        }
                        KeyCode::Char('a') => {
                            self.version_search_query.clear();
                            self.state = AppState::SearchingModQuery { instance_idx, search_type: AssetSearchType::Mod };
                        }
                        KeyCode::Char('d') | KeyCode::Backspace => {
                            if let Some(selected) = mod_list_state.selected()
                                && let Some(inst) = self.instances.get(instance_idx)
                                && let Some(m) = mods.get(selected) {
                                    let mut inst_mut = inst.clone();
                                    let filename = m.filename.clone();
                                    let mod_name = m.metadata.name.clone();
                                    if let Err(e) = inst_mut.remove_mod(&filename, true) {
                                        self.status_message = Some((format!("Failed to delete mod: {}", e), true));
                                    } else {
                                        self.status_message = Some((format!("Deleted mod '{}'", mod_name), false));
                                    }
                                    if let Ok(new_mods) = inst_mut.get_mods() {
                                        mods = new_mods;
                                        if selected >= mods.len() && !mods.is_empty() {
                                            mod_list_state.select(Some(mods.len() - 1));
                                        } else if mods.is_empty() {
                                            mod_list_state.select(None);
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

            AppState::SearchingModQuery { instance_idx, ref search_type } => {
                match key.code {
                    KeyCode::Esc => {
                        if let Some(inst) = self.instances.get(instance_idx) {
                            match search_type {
                                AssetSearchType::Mod => {
                                    if let Ok(mods) = inst.get_mods() {
                                        let mut mod_list_state = ListState::default();
                                        if !mods.is_empty() {
                                            mod_list_state.select(Some(0));
                                        }
                                        self.state = AppState::ModManager { instance_idx, mods, mod_list_state };
                                    } else {
                                        self.state = AppState::Normal;
                                    }
                                }
                                AssetSearchType::Shader => {
                                    let shaders = crate::assets::list_shaderpacks(&inst.path).unwrap_or_default();
                                    let mut list_state = ListState::default();
                                    if !shaders.is_empty() {
                                        list_state.select(Some(0));
                                    }
                                    let has_shader_support = crate::assets::detect_shader_support(&inst.path);
                                    self.state = AppState::ShaderPackManager { instance_idx, shaders, list_state, has_shader_support };
                                }
                                AssetSearchType::ResourcePack => {
                                    let packs = crate::assets::list_resourcepacks(&inst.path).unwrap_or_default();
                                    let mut list_state = ListState::default();
                                    if !packs.is_empty() {
                                        list_state.select(Some(0));
                                    }
                                    self.state = AppState::ResourcePackManager { instance_idx, packs, list_state };
                                }
                                AssetSearchType::Datapack { .. } => {
                                    let worlds = crate::assets::list_worlds(&inst.path).unwrap_or_default();
                                    let mut list_state = ListState::default();
                                    if !worlds.is_empty() {
                                        list_state.select(Some(0));
                                    }
                                    self.state = AppState::WorldManager { instance_idx, worlds, list_state, confirm_delete: None };
                                }
                            }
                        } else {
                            self.state = AppState::Normal;
                        }
                        self.version_search_query.clear();
                    }
                    KeyCode::Enter => {
                        let query = self.version_search_query.trim().to_string();
                        if !query.is_empty() {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            let client = self.api_client.clone();
                            let query_clone = query.clone();
                            if let Some(inst) = self.instances.get(instance_idx) {
                                let (game_version, loader) = inst.get_game_version_and_loader(&self.config.game_dir);
                                let search_type_clone = search_type.clone();
                                tokio::spawn(async move {
                                    let type_filter = match search_type_clone {
                                        AssetSearchType::Mod => "mod",
                                        AssetSearchType::Shader => "shader",
                                        AssetSearchType::ResourcePack => "resourcepack",
                                        AssetSearchType::Datapack { .. } => "datapack",
                                    };
                                    let loader_filter = if type_filter == "mod" { loader.as_deref() } else { None };
                                    let res = client.search_projects(&query_clone, Some(&game_version), loader_filter, type_filter).await;
                                    let _ = tx.send(res);
                                });
                                self.state = AppState::SearchingModLoading { instance_idx, search_type: search_type.clone(), query, rx };
                            }
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

            AppState::SearchingModLoading { .. } => {}

            AppState::SearchingModResults { instance_idx, ref search_type, ref query, ref hits, ref mut list_state } => {
                match key.code {
                    KeyCode::Esc => {
                        let q = query.clone();
                        self.state = AppState::SearchingModQuery { instance_idx, search_type: search_type.clone() };
                        self.version_search_query = q;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected + 1 < hits.len() {
                            list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = list_state.selected()
                            && let Some(hit) = hits.get(idx) {
                                let (tx, rx) = tokio::sync::oneshot::channel();
                                let client = self.api_client.clone();
                                let project_id = hit.project_id.clone();
                                tokio::spawn(async move {
                                    let res = client.fetch_modpack_versions(&project_id).await;
                                    let _ = tx.send(res);
                                });
                                self.state = AppState::SearchingModVersionsLoading { instance_idx, search_type: search_type.clone(), hit: hit.clone(), rx };
                            }
                    }
                    _ => {}
                }
            }

            AppState::SearchingModVersionsLoading { .. } => {}

            AppState::SearchingModVersions { instance_idx, ref search_type, ref hit, ref versions, ref mut list_state } => {
                match key.code {
                    KeyCode::Esc => {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        let client = self.api_client.clone();
                        let query = self.version_search_query.clone();
                        let query_clone = query.clone();
                        if let Some(inst) = self.instances.get(instance_idx) {
                            let (game_version, loader) = inst.get_game_version_and_loader(&self.config.game_dir);
                            let search_type_clone = search_type.clone();
                            tokio::spawn(async move {
                                let type_filter = match search_type_clone {
                                    AssetSearchType::Mod => "mod",
                                    AssetSearchType::Shader => "shader",
                                    AssetSearchType::ResourcePack => "resourcepack",
                                    AssetSearchType::Datapack { .. } => "datapack",
                                };
                                let loader_filter = if type_filter == "mod" { loader.as_deref() } else { None };
                                let res = client.search_projects(&query_clone, Some(&game_version), loader_filter, type_filter).await;
                                  let _ = tx.send(res);
                            });
                            self.state = AppState::SearchingModLoading { instance_idx, search_type: search_type.clone(), query, rx };
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected > 0 {
                            list_state.select(Some(selected - 1));
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        let selected = list_state.selected().unwrap_or(0);
                        if selected + 1 < versions.len() {
                            list_state.select(Some(selected + 1));
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(idx) = list_state.selected()
                            && let Some(version) = versions.get(idx) {
                                let (tx, rx) = mpsc::channel::<ProgressUpdate>(100);
                                let game_dir = self.config.game_dir.clone();
                                let api = self.api_client.clone();
                                let mut inst = self.instances[instance_idx].clone();
                                
                                let version_id = version.id.clone();
                                let version_files = version.files.clone();
                                let dependencies = version.dependencies.clone();
                                
                                let (game_version, loader) = inst.get_game_version_and_loader(&game_dir);
                                let game_version_clone = game_version.clone();
                                let loader_clone = loader.clone();
                                
                                let project_id = hit.project_id.clone();

                                let asset_label = match search_type {
                                    AssetSearchType::Mod => "mod",
                                    AssetSearchType::Shader => "shader pack",
                                    AssetSearchType::ResourcePack => "resource pack",
                                    AssetSearchType::Datapack { .. } => "datapack",
                                };

                                match search_type {
                                    AssetSearchType::Mod => {
                                        tokio::spawn(async move {
                                            let _ = tx.send(ProgressUpdate::Started {
                                                total: 1 + dependencies.len(),
                                                message: "Installing mod...".to_string(),
                                            }).await;

                                            if let Some(file) = version_files.iter().find(|f| f.primary || f.filename.ends_with(".jar")).or_else(|| version_files.first()) {
                                                let _ = tx.send(ProgressUpdate::Progress {
                                                    completed: 0,
                                                    total: 1 + dependencies.len(),
                                                    current_file: file.filename.clone(),
                                                }).await;

                                                if let Err(e) = inst.install_mod_from_url(&game_dir, &file.filename, &file.url, None, true).await {
                                                    let _ = tx.send(ProgressUpdate::Error(format!("Failed to install mod {}: {}", file.filename, e))).await;
                                                    return;
                                                }
                                            }

                                            let mut completed = 1;
                                            let mut installed_projects = std::collections::HashSet::new();
                                            installed_projects.insert(version_id);

                                            for dep in dependencies {
                                                if dep.dependency_type == "required" {
                                                    if let Some(dep_project_id) = dep.project_id {
                                                        if installed_projects.contains(&dep_project_id) {
                                                            continue;
                                                        }

                                                        let dep_name = match api.fetch_project(&dep_project_id).await {
                                                            Ok(p) => p.title,
                                                            Err(_) => dep_project_id.clone(),
                                                        };

                                                        let _ = tx.send(ProgressUpdate::Progress {
                                                            completed,
                                                            total: 1 + completed,
                                                            current_file: format!("Dependency: {}", dep_name),
                                                        }).await;

                                                        if let Ok(dep_versions) = api.fetch_modpack_versions(&dep_project_id).await {
                                                            let comp_ver = dep_versions.into_iter().find(|v| {
                                                                v.game_versions.contains(&game_version_clone) && match loader_clone.as_deref() {
                                                                    Some(l) => v.loaders.iter().any(|loader_name| loader_name.to_lowercase() == l.to_lowercase()),
                                                                    None => true,
                                                                }
                                                            });

                                                            if let Some(cv) = comp_ver {
                                                                if let Some(dep_file) = cv.files.iter().find(|f| f.primary || f.filename.ends_with(".jar")).or_else(|| cv.files.first()) {
                                                                    let _ = inst.install_mod_from_url(&game_dir, &dep_file.filename, &dep_file.url, None, true).await;
                                                                }
                                                            }
                                                        }
                                                        completed += 1;
                                                    }
                                                }
                                            }

                                            let _ = tx.send(ProgressUpdate::Finished).await;
                                        });
                                    }
                                    _ => {
                                        let asset_type = match &search_type {
                                            AssetSearchType::Shader => "shaderpack",
                                            AssetSearchType::ResourcePack => "resourcepack",
                                            AssetSearchType::Datapack { .. } => "datapack",
                                            _ => unreachable!(),
                                        };
                                        let world_name = match &search_type {
                                            AssetSearchType::Datapack { world_name } => Some(world_name.clone()),
                                            _ => None,
                                        };
                                        tokio::spawn(async move {
                                            let _ = tx.send(ProgressUpdate::Started {
                                                total: 1,
                                                message: format!("Installing {}...", asset_label),
                                            }).await;

                                            if let Some(file) = version_files.iter().find(|f| f.primary || f.filename.ends_with(".zip")).or_else(|| version_files.first()) {
                                                let project_slug = match api.fetch_project(&project_id).await {
                                                    Ok(p) => p.slug,
                                                    Err(_) => project_id.clone(),
                                                };
                                                let extension = if file.filename.ends_with(".jar") { "jar" } else { "zip" };
                                                let target_filename = format!("{}.{}", project_slug, extension);

                                                let _ = tx.send(ProgressUpdate::Progress {
                                                    completed: 0,
                                                    total: 1,
                                                    current_file: target_filename.clone(),
                                                }).await;

                                                if let Err(e) = inst.install_asset_from_url(&game_dir, &target_filename, &file.url, None, asset_type, world_name.as_deref()).await {
                                                    let _ = tx.send(ProgressUpdate::Error(format!("Failed to install {}: {}", asset_label, e))).await;
                                                    return;
                                                }
                                            }

                                            let _ = tx.send(ProgressUpdate::Finished).await;
                                        });
                                    }
                                }

                                self.state = AppState::InstallingModProgress {
                                    instance_idx,
                                    search_type: search_type.clone(),
                                    completed: 0,
                                    total: 1,
                                    current_file: String::new(),
                                    message: "Initializing...".to_string(),
                                    rx,
                                };
                            }
                    }
                    _ => {}
                }
            }

            AppState::InstallingModProgress { .. } => {}

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
                        if let Some(idx) = self.instances_list_state.selected()
                            && let Some(inst) = self.instances.get(idx) {
                                self.config.active_instance = Some(inst.id.clone());
                                let _ = self.config.save();
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
                        if let Some(idx) = self.instances_list_state.selected()
                            && let Some(inst) = self.instances.get(idx).cloned() {
                                let _ = inst.delete();
                                if self.config.active_instance.as_ref() == Some(&inst.id) {
                                    self.config.active_instance = None;
                                    let _ = self.config.save();
                                }
                                self.refresh_instances();
                            }
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        if let Some(idx) = self.instances_list_state.selected()
                            && let Some(inst) = self.instances.get(idx).cloned() {
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
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        if let Some(idx) = self.instances_list_state.selected()
                            && let Some(inst) = self.instances.get(idx) {
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
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        if let Some(idx) = self.instances_list_state.selected()
                            && let Some(inst) = self.instances.get(idx) {
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
                    KeyCode::Char('p') | KeyCode::Char('P') => {
                        self.state = AppState::ModpackMenu { selected_option: 0 };
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        if let Some(idx) = self.instances_list_state.selected() {
                            self.state = AppState::ExportingModpackPath { instance_idx: idx };
                            self.version_search_query.clear();
                        }
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        if let Some(idx) = self.instances_list_state.selected()
                            && let Some(inst) = self.instances.get(idx) {
                                let has_shader_support = crate::assets::detect_shader_support(&inst.path);
                                self.state = AppState::AssetManager {
                                    instance_idx: idx,
                                    selected_category: 0,
                                    has_shader_support,
                                };
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
                        if let Some(idx) = self.account_list_state.selected()
                            && let Some(acc) = self.config.accounts.get(idx) {
                                self.config.active_account_uuid = Some(acc.uuid.clone());
                                let _ = self.config.save();
                            }
                    }
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        self.start_offline_account_flow();
                    }
                    KeyCode::Char('a') | KeyCode::Char('A') => {
                        self.start_microsoft_account_flow();
                    }
                    KeyCode::Char('x') | KeyCode::Char('X') => {
                        if let Some(idx) = self.account_list_state.selected()
                            && let Some(acc) = self.config.accounts.get(idx).cloned() {
                                self.config.remove_account(&acc.uuid);
                                let len = self.config.accounts.len();
                                if len == 0 {
                                    self.account_list_state.select(None);
                                } else {
                                    self.account_list_state.select(Some(idx.min(len - 1)));
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

        if event::poll(Duration::from_millis(50)).map_err(|e| format!("Poll failed: {}", e))?
            && let Event::Key(key) = event::read().map_err(|e| format!("Failed to read key: {}", e))?
                && key.kind == event::KeyEventKind::Press {
                    let should_quit = app.handle_key(key).await;
                    if should_quit {
                        break;
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

fn get_centered_rect_helper(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
