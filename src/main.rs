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

    println!("Fetching details for Minecraft version {}...", version_id);
    let manifest = api.fetch_version_manifest().await?;
    let brief = manifest.versions.iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| format!("Minecraft version '{}' not found in Mojang manifest.", version_id))?;

    let details = api.fetch_version_details(&brief.url).await?;

    if let Some(parent) = version_json_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(&details).map_err(|e| e.to_string())?;
    std::fs::write(&version_json_path, content).map_err(|e| e.to_string())?;

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
        InstanceAction::Create { id, version, name } => {
            let name_str = name.unwrap_or_else(|| format!("{} Profile", id));
            
            let api = ApiClient::new();
            println!("Validating Minecraft version '{}'...", version);
            let manifest = api.fetch_version_manifest().await?;
            if !manifest.versions.iter().any(|v| v.id == version) {
                return Err(format!("Version '{}' not found in Mojang version manifest.", version));
            }

            let inst = Instance::create(&config.game_dir, &id, &name_str, &version)?;
            config.active_instance = Some(inst.id.clone());
            config.save()?;
            
            println!("Created instance '{}' ({} - {}) and set it as active.", id, name_str, version);
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

    if !skip_downloads && (!version_json_path.exists() || !config.game_dir.join("versions").join(&version_id).join(format!("{}.jar", version_id)).exists()) {
        download_version_files(&config, &version_id).await?;
    }

    println!("Preparing launch parameters...");
    let launcher = Launcher::new(config);
    launcher.launch(&instance, &account).await?;

    Ok(())
}
