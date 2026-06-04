mod config;
mod api;
mod downloader;
mod launcher;
mod tui;
mod java;
mod instance;
mod crash_analyzer;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::{Config, Account, AccountType, MicrosoftAuth};
use crate::api::ApiClient;
use crate::downloader::{Downloader, ProgressUpdate};
use crate::launcher::Launcher;
use crate::instance::Instance;

#[derive(Parser, Debug)]
#[command(
    name = "minecli",
    author = "MineCLI Team",
    version = "0.1.0",
    about = "Terminal-based Minecraft Launcher"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Launch Minecraft directly from the terminal
    Launch {
        /// Minecraft Instance ID to launch (uses active instance if not specified)
        instance: Option<String>,

        /// Username to run with (offline or online username override)
        #[arg(short, long)]
        username: Option<String>,

        /// Launch in offline mode
        #[arg(short, long)]
        offline: bool,

        /// Do not check or update files, launch immediately
        #[arg(long)]
        offline_mode: bool,
    },
    /// Manage instances (create, delete, list, backup, restore, sync)
    Instance {
        #[command(subcommand)]
        action: InstanceAction,
    },
    /// List all locally downloaded versions
    List {
        /// Filter local versions by name
        #[arg(short, long)]
        search: Option<String>,
    },
    /// List all remote versions available from Mojang
    ListRemote {
        /// Show only official releases
        #[arg(short, long)]
        release: bool,

        /// Show only snapshots
        #[arg(short, long)]
        snapshot: bool,

        /// Filter versions by name
        #[arg(short, long)]
        search: Option<String>,

        /// Limit the number of printed results (default: 40)
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Download and verify all game files for a version without launching
    Download {
        /// Minecraft version ID (e.g., 1.20.4)
        version: String,
    },
    /// Manage accounts (add, delete, list, select)
    Accounts {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// View or edit settings
    Settings {
        #[command(subcommand)]
        action: SettingsAction,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum InstanceAction {
    /// List all local instances
    List,
    /// Create a new instance
    Create {
        id: String,
        version: String,
        #[arg(short, long)]
        name: Option<String>,
        /// Optional mod loader (e.g. fabric, forge, neoforge)
        #[arg(long)]
        loader: Option<String>,
        /// Optional mod loader version
        #[arg(long)]
        loader_version: Option<String>,
    },
    /// Delete an instance
    Delete {
        id: String,
    },
    /// Create a snapshot/backup of an instance
    Backup {
        id: String,
    },
    /// Restore an instance to a backup snapshot
    Restore {
        id: String,
        #[arg(short, long)]
        backup: String, // backup filename or timestamp
    },
    /// List all backups for an instance
    ListBackups {
        id: String,
    },
    /// List mods for an instance
    ListMods {
        id: String,
    },
    /// Enable a mod in an instance
    EnableMod {
        id: String,
        filename: String,
    },
    /// Disable a mod in an instance
    DisableMod {
        id: String,
        filename: String,
    },
    /// Synchronize/download declarative mods for an instance
    Sync {
        id: String,
    },
    /// Edit instance configuration (such as Java settings)
    Edit {
        id: String,
        /// Set a custom Java executable path for this instance (use "clear" or empty to revert to default)
        #[arg(long)]
        java_path: Option<String>,
        /// Set a specific JRE major version to download and use (e.g. 8, 17, 21. Use 0 to revert to auto-detection)
        #[arg(long)]
        java_version: Option<u32>,
    },
    /// Import a Modrinth .mrpack modpack file as a new instance
    ImportPack {
        /// Path to the .mrpack file
        path: String,
        /// Instance ID to use
        #[arg(short, long)]
        id: Option<String>,
    },
    /// Search for Modrinth modpacks
    SearchPack {
        /// Search query
        query: String,
    },
    /// Export an instance as a Modrinth .mrpack modpack
    ExportPack {
        /// Instance ID to export
        id: String,
        /// Output path for the .mrpack file
        output_path: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum AccountAction {
    /// List all configured accounts
    List,
    /// Set the active account
    Select {
        /// Username or UUID of the account
        name_or_uuid: String,
    },
    /// Add a new offline profile
    AddOffline {
        /// Desired username
        username: String,
    },
    /// Add a new Microsoft online account (starts device code flow)
    Add,
    /// Remove an account profile
    Remove {
        /// UUID or username of the account to remove
        name_or_uuid: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
enum SettingsAction {
    /// Show current settings
    Show,
    /// Set the game directory path
    SetGameDir {
        path: String,
    },
    /// Set the Java executable path
    SetJava {
        path: String,
    },
    /// Set JVM arguments (pass quotes)
    SetJvmArgs {
        #[arg(allow_hyphen_values = true)]
        args: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Launch { instance, username, offline, offline_mode }) => {
            if let Err(e) = handle_cli_launch(instance, username, !offline, offline_mode).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Instance { action }) => {
            if let Err(e) = handle_instance_command(action).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::List { search }) => {
            let config = Config::load();
            let launcher = Launcher::new(config);
            let mut versions = launcher.get_available_local_versions();
            if let Some(ref q) = search {
                let q_lower = q.to_lowercase();
                versions.retain(|v| v.to_lowercase().contains(&q_lower));
            }
            if versions.is_empty() {
                if search.is_some() {
                    println!("No local versions match the search filter.");
                } else {
                    println!("No local versions downloaded. Run `minecli` to select and download one.");
                }
            } else {
                println!("Downloaded Minecraft versions:");
                for v in versions {
                    println!(" - {}", v);
                }
            }
        }
        Some(Commands::ListRemote { release, snapshot, search, limit }) => {
            println!("Fetching available Minecraft versions...");
            let api = ApiClient::new();
            match api.fetch_version_manifest().await {
                Ok(manifest) => {
                    let mut filtered_versions = manifest.versions;
                    
                    if release || snapshot {
                        filtered_versions.retain(|v| {
                            (release && v.r#type == "release") || (snapshot && v.r#type == "snapshot")
                        });
                    }
                    
                    if let Some(ref q) = search {
                        let q_lower = q.to_lowercase();
                        filtered_versions.retain(|v| v.id.to_lowercase().contains(&q_lower));
                    }
                    
                    let total_count = filtered_versions.len();
                    let print_limit = limit.unwrap_or(40);
                    let display_list: Vec<_> = filtered_versions.iter().take(print_limit).collect();
                    
                    if display_list.is_empty() {
                        println!("No remote versions match the specified filters.");
                    } else {
                        println!("{:<18} | {:<10} | {}", "Version ID", "Type", "Release Time");
                        println!("{}", "-".repeat(50));
                        for v in &display_list {
                            println!("{:<18} | {:<10} | {}", v.id, v.r#type, v.releaseTime);
                        }
                        if total_count > print_limit {
                            println!("... and {} more. Launch with `minecli launch <version>` to play.", total_count - print_limit);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to fetch remote versions: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Download { version }) => {
            let config = Config::load();
            if let Err(e) = download_version_files(&config, &version).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Accounts { action }) => {
            if let Err(e) = handle_accounts_command(action).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Settings { action }) => {
            if let Err(e) = handle_settings_command(action) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        None => {
            if let Err(e) = tui::run_tui().await {
                eprintln!("Launcher crashed: {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn download_version_files(config: &Config, version_id: &str) -> Result<(), String> {
    let api = ApiClient::new();
    let version_json_path = config.game_dir
        .join("versions")
        .join(version_id)
        .join(format!("{}.json", version_id));

    let details = if version_json_path.exists() {
        let launcher = Launcher::new(config.clone());
        launcher.load_version_details_raw(version_id)?
    } else {
        if version_id.starts_with("fabric-loader-") {
            let rest = version_id.strip_prefix("fabric-loader-").unwrap();
            let (loader_ver, game_ver) = rest.split_once('-')
                .ok_or_else(|| format!("Invalid Fabric version ID format: {}", version_id))?;
            println!("Fetching Fabric profile (Loader: {}, Game: {})...", loader_ver, game_ver);
            api.fetch_fabric_profile(game_ver, loader_ver).await?
        } else if version_id.starts_with("forge-") {
            let loader_ver = version_id.strip_prefix("forge-").unwrap();
            println!("Fetching Forge profile (Version: {})...", loader_ver);
            api.fetch_forge_profile(loader_ver).await?
        } else if version_id.starts_with("neoforge-") {
            let loader_ver = version_id.strip_prefix("neoforge-").unwrap();
            println!("Fetching NeoForge profile (Version: {})...", loader_ver);
            api.fetch_neoforge_profile(loader_ver).await?
        } else {
            println!("Fetching details for Minecraft version {}...", version_id);
            let manifest = api.fetch_version_manifest().await?;
            let brief = manifest.versions.iter()
                .find(|v| v.id == version_id)
                .ok_or_else(|| format!("Minecraft version '{}' not found in Mojang manifest.", version_id))?;

            api.fetch_version_details(&brief.url).await?
        }
    };

    if !version_json_path.exists() {
        if let Some(parent) = version_json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = serde_json::to_string_pretty(&details).map_err(|e| e.to_string())?;
        std::fs::write(&version_json_path, content).map_err(|e| e.to_string())?;
    }

    let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
    let downloader = Downloader::new(tx);
    let game_dir = config.game_dir.clone();

    tokio::spawn(async move {
        let _ = downloader.download_version(&game_dir, &details).await;
    });

    while let Some(update) = rx.recv().await {
        match update {
            ProgressUpdate::Started { total: _, message } => {
                println!("\n\x1b[33m→ {}\x1b[0m", message);
            }
            ProgressUpdate::Progress { completed, total, current_file } => {
                print!("\r[\x1b[36m{}/{}\x1b[0m] Downloading: {}                     ", completed, total, current_file);
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            ProgressUpdate::Message(msg) => {
                println!("\n\x1b[32m✔ {}\x1b[0m", msg);
            }
            ProgressUpdate::Finished => {
                println!("\n\x1b[32m✔ Download and integrity checks complete!\x1b[0m");
            }
            ProgressUpdate::Error(e) => {
                return Err(format!("Download failed: {}", e));
            }
        }
    }

    Ok(())
}

async fn handle_accounts_command(action: AccountAction) -> Result<(), String> {
    let mut config = Config::load();
    match action {
        AccountAction::List => {
            if config.accounts.is_empty() {
                println!("No accounts configured. Use the `accounts add-offline` subcommand or TUI.");
            } else {
                println!("Configured Accounts:");
                for acc in &config.accounts {
                    let active_marker = if config.active_account_uuid.as_deref() == Some(&acc.uuid) { " (ACTIVE)" } else { "" };
                    let acc_type = match acc.account_type {
                        AccountType::Offline => "Offline",
                        AccountType::Microsoft => "Microsoft",
                    };
                    println!(" - {} [{}]{}", acc.username, acc_type, active_marker);
                }
            }
        }
        AccountAction::Select { name_or_uuid } => {
            let acc = config.accounts.iter().find(|a| a.username == name_or_uuid || a.uuid == name_or_uuid).cloned();
            if let Some(account) = acc {
                config.active_account_uuid = Some(account.uuid.clone());
                config.save()?;
                println!("Set active account to: {} ({})", account.username, account.uuid);
            } else {
                return Err(format!("Account '{}' not found.", name_or_uuid));
            }
        }
        AccountAction::AddOffline { username } => {
            let uuid = Uuid::new_v4().simple().to_string();
            let account = Account {
                uuid,
                username: username.clone(),
                account_type: AccountType::Offline,
                microsoft_auth: None,
            };
            config.add_account(account);
            println!("Successfully added offline profile for: {}", username);
        }
        AccountAction::Add => {
            let api = ApiClient::new();
            let dev_code = api.request_device_code().await?;
            println!("To log in, open a web browser and navigate to:");
            println!("  \x1b[36m\x1b[4m{}\x1b[0m", dev_code.verification_uri);
            println!("Enter the code below to authorize this launcher:");
            println!("  \x1b[1m\x1b[32m{}\x1b[0m", dev_code.user_code);
            println!("Waiting for authentication...");

            let poll_interval = std::time::Duration::from_secs(dev_code.interval.max(1));
            let mut expires_in = dev_code.expires_in;

            while expires_in > 0 {
                tokio::time::sleep(poll_interval).await;
                expires_in = expires_in.saturating_sub(poll_interval.as_secs());

                match api.poll_token(&dev_code.device_code).await {
                    Ok(Some(token_res)) => {
                        print!("Exchanging tokens with Mojang... ");
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        
                        let mc_res = api.login_with_microsoft(&token_res.access_token).await?;
                        let profile = api.fetch_profile(&mc_res.access_token).await?;
                        
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
                        
                        println!("Success!");
                        println!("Logged in online as: {}", account.username);
                        config.add_account(account);
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        return Err(format!("Authentication failed: {}", e));
                    }
                }
            }
        }
        AccountAction::Remove { name_or_uuid } => {
            let acc = config.accounts.iter().find(|a| a.username == name_or_uuid || a.uuid == name_or_uuid).cloned();
            if let Some(account) = acc {
                config.remove_account(&account.uuid);
                println!("Removed account profile: {}", account.username);
            } else {
                return Err(format!("Account '{}' not found.", name_or_uuid));
            }
        }
    }
    Ok(())
}

fn handle_settings_command(action: SettingsAction) -> Result<(), String> {
    let mut config = Config::load();
    match action {
        SettingsAction::Show => {
            println!("Launcher Settings:");
            println!("  Game Directory:       {}", config.game_dir.display());
            println!("  Java Executable Path: {}", config.java_path.display());
            println!("  JVM Extra Arguments:  {}", config.jvm_args.join(" "));
        }
        SettingsAction::SetGameDir { path } => {
            config.game_dir = std::path::PathBuf::from(path.clone());
            config.save()?;
            println!("Successfully set game directory to: {}", path);
        }
        SettingsAction::SetJava { path } => {
            config.java_path = std::path::PathBuf::from(path.clone());
            config.save()?;
            println!("Successfully set Java executable path to: {}", path);
        }
        SettingsAction::SetJvmArgs { args } => {
            config.jvm_args = args.split_whitespace().map(|s| s.to_string()).collect();
            config.save()?;
            println!("Successfully set JVM arguments to: {}", args);
        }
    }
    Ok(())
}
async fn resolve_and_setup_loader(
    api: &ApiClient,
    game_dir: &std::path::Path,
    game_version: &str,
    loader_name: &str,
    loader_version_opt: Option<String>,
) -> Result<String, String> {
    let loader_lower = loader_name.to_lowercase();
    if loader_lower == "fabric" {
        let loaders = api.fetch_fabric_loaders(game_version).await?;
        let loader_ver = if let Some(v) = loader_version_opt {
            if !loaders.iter().any(|l| l.loader.version == v) {
                return Err(format!("Fabric loader version '{}' not found for Minecraft {}.", v, game_version));
            }
            v
        } else {
            let selected = loaders.iter()
                .find(|l| l.loader.stable)
                .or_else(|| loaders.first())
                .ok_or_else(|| format!("No Fabric loaders found for Minecraft {}.", game_version))?;
            selected.loader.version.clone()
        };

        let version_id = format!("fabric-loader-{}-{}", loader_ver, game_version);
        println!("Fetching Fabric profile for loader {}...", loader_ver);
        let profile = api.fetch_fabric_profile(game_version, &loader_ver).await?;
        
        let version_dir = game_dir.join("versions").join(&version_id);
        std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;
        let json_path = version_dir.join(format!("{}.json", version_id));
        let json_str = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
        std::fs::write(&json_path, json_str).map_err(|e| e.to_string())?;

        Ok(version_id)
    } else if loader_lower == "forge" || loader_lower == "neoforge" {
        let is_neoforge = loader_lower == "neoforge";
        println!("Fetching {} version index...", if is_neoforge { "NeoForge" } else { "Forge" });
        let index = if is_neoforge {
            api.fetch_neoforge_versions().await?
        } else {
            api.fetch_forge_versions().await?
        };

        // Filter versions matching game version
        let matching: Vec<_> = index.versions.iter().filter(|v| {
            v.requires.iter().any(|req| req.uid == "net.minecraft" && req.equals == game_version)
        }).collect();

        if matching.is_empty() {
            return Err(format!("No {} versions found matching Minecraft version {}.", if is_neoforge { "NeoForge" } else { "Forge" }, game_version));
        }

        let selected_ver = if let Some(v) = loader_version_opt {
            if !matching.iter().any(|m| m.version == v) {
                return Err(format!("{} version '{}' not found/supported for Minecraft {}.", if is_neoforge { "NeoForge" } else { "Forge" }, v, game_version));
            }
            v
        } else {
            let rec = matching.iter().find(|m| m.recommended).or_else(|| matching.first());
            rec.unwrap().version.clone()
        };

        let version_id = if is_neoforge {
            format!("neoforge-{}", selected_ver)
        } else {
            format!("forge-{}", selected_ver)
        };

        println!("Fetching {} profile for version {}...", if is_neoforge { "NeoForge" } else { "Forge" }, selected_ver);
        let profile = if is_neoforge {
            api.fetch_neoforge_profile(&selected_ver).await?
        } else {
            api.fetch_forge_profile(&selected_ver).await?
        };

        let version_dir = game_dir.join("versions").join(&version_id);
        std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;
        let json_path = version_dir.join(format!("{}.json", version_id));
        let json_str = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
        std::fs::write(&json_path, json_str).map_err(|e| e.to_string())?;

        Ok(version_id)
    } else {
        Err(format!("Unsupported mod loader '{}'. Supported: fabric, forge, neoforge", loader_name))
    }
}

async fn handle_instance_command(action: InstanceAction) -> Result<(), String> {
    let mut config = Config::load();
    match action {
        InstanceAction::List => {
            let list = Instance::load_all(&config.game_dir);
            if list.is_empty() {
                println!("No instances configured. Use `minecli instance create` or TUI.");
            } else {
                println!("Available Instances:");
                for inst in list {
                    let active_marker = if config.active_instance.as_ref() == Some(&inst.id) { " (ACTIVE)" } else { "" };
                    println!(" - {} [{}] [Version: {}]{}", inst.config.name, inst.id, inst.config.version, active_marker);
                }
            }
        }
        InstanceAction::Create { id, version, name, loader, loader_version } => {
            let name_str = name.unwrap_or_else(|| format!("{} Profile", id));
            let api = ApiClient::new();
            
            let resolved_version = if let Some(ref l) = loader {
                resolve_and_setup_loader(&api, &config.game_dir, &version, l, loader_version).await?
            } else {
                println!("Validating Minecraft version '{}'...", version);
                let manifest = api.fetch_version_manifest().await?;
                if !manifest.versions.iter().any(|v| v.id == version) {
                    return Err(format!("Version '{}' not found in Mojang version manifest.", version));
                }
                version.clone()
            };

            let inst = Instance::create(&config.game_dir, &id, &name_str, &resolved_version)?;
            config.active_instance = Some(inst.id.clone());
            config.save()?;
            
            println!("Created instance '{}' ({} - {}) and set it as active.", id, name_str, resolved_version);
            println!("Run `minecli launch {}` to play.", id);
        }
        InstanceAction::Delete { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            inst.delete()?;
            if config.active_instance.as_ref() == Some(&id) {
                config.active_instance = None;
                config.save()?;
            }
            println!("Deleted instance '{}'.", id);
        }
        InstanceAction::Backup { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Creating backup of instance '{}'...", id);
            let path = inst.backup()?;
            println!("Backup created at: {}", path.display());
        }
        InstanceAction::Restore { id, backup } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Restoring backup '{}' for instance '{}'...", backup, id);
            inst.restore(&backup)?;
            println!("Instance restored successfully!");
        }
        InstanceAction::ListBackups { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let backups = inst.list_backups();
            if backups.is_empty() {
                println!("No backups found for instance '{}'.", id);
            } else {
                println!("Available backups for instance '{}':", id);
                for backup in backups {
                    println!(" - {}", backup);
                }
            }
        }
        InstanceAction::ListMods { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let mods = inst.get_mods()?;
            if mods.is_empty() {
                println!("No mods found for instance '{}'.", id);
            } else {
                println!("Mods for instance '{}':", id);
                for m in mods {
                    let status = if m.enabled { "ENABLED" } else { "DISABLED" };
                    println!(" - {:<35} [{}] (Version: {})", m.filename, status, m.metadata.version);
                }
            }
        }
        InstanceAction::EnableMod { id, filename } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            inst.enable_mod(&filename)?;
            println!("Enabled mod '{}' in instance '{}'.", filename, id);
        }
        InstanceAction::DisableMod { id, filename } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            inst.disable_mod(&filename)?;
            println!("Disabled mod '{}' in instance '{}'.", filename, id);
        }
        InstanceAction::Sync { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Syncing mods for instance '{}'...", id);
            let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
            let game_dir = config.game_dir.clone();
            tokio::spawn(async move {
                let _ = inst.sync_mods(&game_dir, tx).await;
            });
            while let Some(update) = rx.recv().await {
                match update {
                    ProgressUpdate::Started { total, message } => {
                        println!("Sync started: {} (Total: {})", message, total);
                    }
                    ProgressUpdate::Progress { completed, total, current_file } => {
                        print!("\r[{}/{}] Syncing: {}                             ", completed, total, current_file);
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                    ProgressUpdate::Message(msg) => {
                        println!("\n{}", msg);
                    }
                    ProgressUpdate::Finished => {
                        println!("\nMod sync completed successfully!");
                    }
                    ProgressUpdate::Error(e) => {
                        return Err(format!("Mod sync failed: {}", e));
                    }
                }
            }
        }
        InstanceAction::Edit { id, java_path, java_version } => {
            let mut inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let mut modified = false;

            if let Some(path) = java_path {
                if path.is_empty() || path == "clear" {
                    inst.config.java_path = None;
                    println!("Cleared custom Java path for instance '{}'.", id);
                } else {
                    inst.config.java_path = Some(path);
                    println!("Set custom Java path for instance '{}' to: {}", id, inst.config.java_path.as_ref().unwrap());
                }
                modified = true;
            }

            if let Some(ver) = java_version {
                if ver == 0 {
                    inst.config.java_version = None;
                    println!("Cleared custom JRE version for instance '{}'.", id);
                } else {
                    inst.config.java_version = Some(ver);
                    println!("Set custom JRE version for instance '{}' to: Java {}", id, ver);
                }
                modified = true;
            }

            if modified {
                inst.save()?;
                println!("Saved settings for instance '{}'.", id);
            } else {
                println!("No changes specified. Use `--java-path` or `--java-version`.");
            }
        }
        InstanceAction::ImportPack { path, id } => {
            let pack_path = std::path::PathBuf::from(&path);
            if !pack_path.exists() {
                return Err(format!("File not found: {}", path));
            }

            let custom_id = id.unwrap_or_else(|| {
                pack_path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("imported-pack")
                    .to_string()
            });

            println!("Importing modpack '{}' as instance '{}'...", path, custom_id);

            let (tx, mut rx) = tokio::sync::mpsc::channel::<ProgressUpdate>(100);

            let game_dir = config.game_dir.clone();
            let pack_path_clone = pack_path.clone();
            let id_clone = custom_id.clone();

            let handle = tokio::spawn(async move {
                Instance::import_mrpack(&game_dir, &pack_path_clone, &id_clone, &tx).await
            });

            // Print progress
            while let Some(update) = rx.recv().await {
                match update {
                    ProgressUpdate::Message(msg) => println!("  {}", msg),
                    ProgressUpdate::Started { total, message } => println!("  {} ({} files)", message, total),
                    ProgressUpdate::Progress { completed, total, current_file } => {
                        println!("  [{}/{}] {}", completed, total, current_file);
                    }
                    ProgressUpdate::Finished => break,
                    ProgressUpdate::Error(e) => {
                        eprintln!("  Error: {}", e);
                    }
                }
            }

            match handle.await {
                Ok(Ok(inst)) => {
                    println!("Successfully imported modpack as instance '{}'.", inst.id);
                    println!("  Version: {}", inst.config.version);
                    if let Some(mods) = &inst.config.mods {
                        println!("  Mods: {} declared", mods.len());
                    }
                }
                Ok(Err(e)) => return Err(format!("Import failed: {}", e)),
                Err(e) => return Err(format!("Import task panicked: {}", e)),
            }
        }
        InstanceAction::SearchPack { query } => {
            let api = ApiClient::new();
            println!("Searching Modrinth for modpacks matching '{}'...", query);
            let hits = api.search_modpacks(&query).await?;
            if hits.is_empty() {
                println!("No modpacks found.");
            } else {
                println!("{:<24} | {:<20} | {:<12} | {}", "Title", "ID/Slug", "Downloads", "Description");
                println!("{}", "-".repeat(80));
                for hit in hits {
                    let desc = if hit.description.len() > 40 {
                        format!("{}...", &hit.description[..37])
                    } else {
                        hit.description.clone()
                    };
                    println!(
                        "{:<24} | {:<20} | {:<12} | {}",
                        hit.title,
                        hit.project_id,
                        hit.downloads,
                        desc
                    );
                }
            }
        }
        InstanceAction::ExportPack { id, output_path } => {
            let instances = Instance::load_all(&config.game_dir);
            let inst = instances.iter().find(|i| i.id == id)
                .ok_or_else(|| format!("Instance '{}' not found.", id))?;

            let out_path = std::path::PathBuf::from(&output_path);
            println!("Exporting instance '{}' to '{}'...", id, out_path.display());
            inst.export_mrpack(&out_path)?;
            println!("Successfully exported modpack!");
        }
    }
    Ok(())
}

async fn handle_cli_launch(
    instance_id: Option<String>,
    username_override: Option<String>,
    force_online: bool,
    skip_downloads: bool,
) -> Result<(), String> {
    let mut config = Config::load();
    let api = ApiClient::new();

    let inst_id = instance_id.or(config.active_instance.clone()).ok_or("No instance selected and no active instance set. Use 'minecli instance create' or select one in the TUI.")?;
    let instance = Instance::load(&inst_id, config.game_dir.join("instances").join(&inst_id))?;
    let version_id = instance.config.version.clone();

    let account = if !force_online {
        let username = username_override
            .or_else(|| config.get_active_account().map(|a| a.username.clone()))
            .unwrap_or_else(|| "Player".to_string());

        let mut resolved_acc = None;
        for acc in &config.accounts {
            if acc.username == username && acc.account_type == AccountType::Offline {
                resolved_acc = Some(acc.clone());
                break;
            }
        }

        match resolved_acc {
            Some(acc) => acc,
            None => {
                let uuid = Uuid::new_v4().simple().to_string();
                let acc = Account {
                    uuid,
                    username,
                    account_type: AccountType::Offline,
                    microsoft_auth: None,
                };
                config.add_account(acc.clone());
                acc
            }
        }
    } else {
        let mut target_account = None;
        if let Some(ref acc) = config.get_active_account() {
            if acc.account_type == AccountType::Microsoft {
                target_account = Some((*acc).clone());
            }
        }
        if target_account.is_none() {
            target_account = config.accounts.iter().find(|a| a.account_type == AccountType::Microsoft).cloned();
        }
        if target_account.is_none() {
            if let Some(ref acc) = config.get_active_account() {
                if acc.account_type == AccountType::Offline {
                    target_account = Some((*acc).clone());
                }
            }
        }

        match target_account {
            Some(mut acc) => {
                if acc.account_type == AccountType::Microsoft {
                    if let Some(ref auth) = acc.microsoft_auth {
                        let is_expired = auth.expires_at.map(|exp| exp < chrono::Utc::now()).unwrap_or(true);
                        if is_expired {
                            println!("Session expired. Refreshing Microsoft tokens...");
                            match api.refresh_token(&auth.refresh_token).await {
                                Ok(token_res) => {
                                    match api.login_with_microsoft(&token_res.access_token).await {
                                        Ok(mc_res) => {
                                            let updated_auth = MicrosoftAuth {
                                                access_token: mc_res.access_token,
                                                refresh_token: token_res.refresh_token,
                                                expires_at: Some(chrono::Utc::now() + chrono::Duration::seconds(mc_res.expires_in as i64)),
                                            };
                                            acc.microsoft_auth = Some(updated_auth);
                                            config.add_account(acc.clone());
                                        }
                                        Err(e) => {
                                            return Err(format!("Failed to log in with refreshed token: {}", e));
                                        }
                                    }
                                }
                                Err(e) => {
                                    return Err(format!("Failed to refresh MS token (re-authentication required): {}", e));
                                }
                            }
                        }
                    }
                    println!("Logged in online as: {}", acc.username);
                    acc
                } else {
                    println!("Logged in offline as: {}", acc.username);
                    acc
                }
            }
            None => {
                println!("No account configured. Starting Microsoft Online Login...");
                let dev_code = api.request_device_code().await?;
                println!("To log in, open a web browser and navigate to:");
                println!("  \x1b[36m\x1b[4m{}\x1b[0m", dev_code.verification_uri);
                println!("Enter the code below to authorize this launcher:");
                println!("  \x1b[1m\x1b[32m{}\x1b[0m", dev_code.user_code);
                println!("Waiting for authentication...");

                let poll_interval = std::time::Duration::from_secs(dev_code.interval.max(1));
                let mut expires_in = dev_code.expires_in;
                let mut auth_account = None;

                while expires_in > 0 {
                    tokio::time::sleep(poll_interval).await;
                    expires_in = expires_in.saturating_sub(poll_interval.as_secs());

                    match api.poll_token(&dev_code.device_code).await {
                        Ok(Some(token_res)) => {
                            print!("Exchanging tokens with Mojang... ");
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                            
                            let mc_res = api.login_with_microsoft(&token_res.access_token).await?;
                            let profile = api.fetch_profile(&mc_res.access_token).await?;
                            
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
                            
                            println!("Success!");
                            println!("Logged in online as: {}", account.username);
                            config.add_account(account.clone());
                            auth_account = Some(account);
                            break;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            return Err(format!("Authentication failed: {}", e));
                        }
                    }
                }

                auth_account.ok_or("Authentication timed out or cancelled.")?
            }
        }
    };

    let version_json_path = config.game_dir
        .join("versions")
        .join(&version_id)
        .join(format!("{}.json", version_id));

    let jar_version_id = if version_json_path.exists() {
        let launcher = Launcher::new(config.clone());
        if let Ok(raw_details) = launcher.load_version_details_raw(&version_id) {
            raw_details.inheritsFrom.clone().unwrap_or_else(|| version_id.clone())
        } else {
            version_id.clone()
        }
    } else {
        version_id.clone()
    };

    let client_jar_path = config.game_dir
        .join("versions")
        .join(&jar_version_id)
        .join(format!("{}.jar", jar_version_id));

    if !skip_downloads && (!version_json_path.exists() || !client_jar_path.exists()) {
        download_version_files(&config, &version_id).await?;
    }

    println!("Preparing launch parameters...");
    let launcher = Launcher::new(config);
    launcher.launch(&instance, &account).await?;

    Ok(())
}
