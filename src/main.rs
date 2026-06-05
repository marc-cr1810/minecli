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

use crossterm::style::Stylize;

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
    /// Search Modrinth for compatible mods
    SearchMod {
        /// Instance ID to check compatibility against
        id: String,
        /// Search query
        query: String,
    },
    /// Add/install a mod to an instance
    AddMod {
        /// Instance ID
        id: String,
        /// Modrinth mod ID or slug
        mod_id: String,
        /// Skip downloading dependencies
        #[arg(long)]
        no_deps: bool,
        /// Save mod to instance.toml for future declarative sync
        #[arg(long, default_value_t = true)]
        save: bool,
    },
    /// Remove/delete a mod from an instance
    RemoveMod {
        /// Instance ID
        id: String,
        /// Filename or Modrinth ID of the mod to remove
        filename_or_id: String,
    },
    /// Check for and apply updates to installed mods
    UpdateMods {
        /// Instance ID
        id: String,
        /// Do not prompt, automatically apply all updates
        #[arg(short, long)]
        yes: bool,
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
                eprintln!("{}: {}", "Error".red().bold(), e);
                std::process::exit(1);
            }
        }
        Some(Commands::Instance { action }) => {
            if let Err(e) = handle_instance_command(action).await {
                eprintln!("{}: {}", "Error".red().bold(), e);
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
                    println!("{}", "No local versions match the search filter.".yellow());
                } else {
                    println!("{}", "No local versions downloaded. Run `minecli` to select and download one.".yellow());
                }
            } else {
                println!("{}", "Downloaded Minecraft versions:".cyan().bold());
                for v in versions {
                    println!("  {} {}", "•".cyan(), v);
                }
            }
        }
        Some(Commands::ListRemote { release, snapshot, search, limit }) => {
            println!("{}", "Fetching available Minecraft versions...".cyan());
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
                        println!("{}", "No remote versions match the specified filters.".yellow());
                    } else {
                        let col1 = format!("{:<18}", "Version ID");
                        let col2 = format!("{:<10}", "Type");
                        println!("{} | {} | {}", col1.cyan().bold(), col2.cyan().bold(), "Release Time".cyan().bold());
                        println!("{}", "-".repeat(50).dim());
                        for v in &display_list {
                            let id_padded = format!("{:<18}", v.id);
                            let type_padded = format!("{:<10}", v.r#type);
                            let type_styled = if v.r#type == "release" {
                                type_padded.green()
                            } else {
                                type_padded.yellow()
                            };
                            println!("{} | {} | {}", id_padded.bold(), type_styled, v.releaseTime.clone().dim());
                        }
                        if total_count > print_limit {
                            println!("{}", format!("... and {} more. Launch with `minecli launch <version>` to play.", total_count - print_limit).italic().dim());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{}: Failed to fetch remote versions: {}", "Error".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
        Some(Commands::Download { version }) => {
            let config = Config::load();
            if let Err(e) = download_version_files(&config, &version).await {
                eprintln!("{}: {}", "Error".red().bold(), e);
                std::process::exit(1);
            }
        }
        Some(Commands::Accounts { action }) => {
            if let Err(e) = handle_accounts_command(action).await {
                eprintln!("{}: {}", "Error".red().bold(), e);
                std::process::exit(1);
            }
        }
        Some(Commands::Settings { action }) => {
            if let Err(e) = handle_settings_command(action) {
                eprintln!("{}: {}", "Error".red().bold(), e);
                std::process::exit(1);
            }
        }
        None => {
            if let Err(e) = tui::run_tui().await {
                eprintln!("{} crashed: {}", "Launcher".red().bold(), e);
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
    } else if version_id.starts_with("fabric-loader-") {
        let rest = version_id.strip_prefix("fabric-loader-").unwrap();
        let (loader_ver, game_ver) = rest.split_once('-')
            .ok_or_else(|| format!("Invalid Fabric version ID format: {}", version_id))?;
        println!("{} (Loader: {}, Game: {})...", "Fetching Fabric profile".cyan(), loader_ver.yellow(), game_ver.yellow());
        api.fetch_fabric_profile(game_ver, loader_ver).await?
    } else if version_id.starts_with("forge-") {
        let loader_ver = version_id.strip_prefix("forge-").unwrap();
        println!("{} (Version: {})...", "Fetching Forge profile".cyan(), loader_ver.yellow());
        api.fetch_forge_profile(loader_ver).await?
    } else if version_id.starts_with("neoforge-") {
        let loader_ver = version_id.strip_prefix("neoforge-").unwrap();
        println!("{} (Version: {})...", "Fetching NeoForge profile".cyan(), loader_ver.yellow());
        api.fetch_neoforge_profile(loader_ver).await?
    } else {
        println!("{} {}...", "Fetching details for Minecraft version".cyan(), version_id.yellow());
        let manifest = api.fetch_version_manifest().await?;
        let brief = manifest.versions.iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| format!("Minecraft version '{}' not found in Mojang manifest.", version_id))?;

        api.fetch_version_details(&brief.url).await?
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
                println!("\n{} {}", "→".yellow().bold(), message.bold());
            }
            ProgressUpdate::Progress { completed, total, current_file } => {
                print!(
                    "\r[{}] Downloading: {}                     ",
                    format!("{}/{}", completed, total).cyan(),
                    current_file
                );
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            ProgressUpdate::Message(msg) => {
                println!("\n{} {}", "✔".green().bold(), msg.green());
            }
            ProgressUpdate::Finished => {
                println!("\n{} {}", "✔".green().bold(), "Download and integrity checks complete!".green().bold());
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
                println!("{}", "No accounts configured. Use the `accounts add-offline` subcommand or TUI.".yellow());
            } else {
                println!("{}", "Configured Accounts:".cyan().bold());
                for acc in &config.accounts {
                    let is_active = config.active_account_uuid.as_deref() == Some(&acc.uuid);
                    let active_marker = if is_active { " (ACTIVE)".green().bold().to_string() } else { "".to_string() };
                    let acc_type = match acc.account_type {
                        AccountType::Offline => "Offline".dim(),
                        AccountType::Microsoft => "Microsoft".magenta(),
                    };
                    let bullet = if is_active { "•".green() } else { "•".dim() };
                    println!("  {} {} [{}]{}", bullet, acc.username.clone().bold(), acc_type, active_marker);
                }
            }
        }
        AccountAction::Select { name_or_uuid } => {
            let acc = config.accounts.iter().find(|a| a.username == name_or_uuid || a.uuid == name_or_uuid).cloned();
            if let Some(account) = acc {
                config.active_account_uuid = Some(account.uuid.clone());
                config.save()?;
                println!("Set active account to: {} ({})", account.username.green().bold(), account.uuid.dim());
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
            println!("{} added offline profile for: {}", "Successfully".green().bold(), username.green());
        }
        AccountAction::Add => {
            let api = ApiClient::new();
            let dev_code = api.request_device_code().await?;
            println!("{}", "To log in, open a web browser and navigate to:".cyan());
            println!("  {}", dev_code.verification_uri.cyan().underlined());
            println!("{}", "Enter the code below to authorize this launcher:".cyan());
            println!("  {}", dev_code.user_code.green().bold());
            println!("{}", "Waiting for authentication...".yellow().italic());

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
                        
                        println!("{}", "Success!".green().bold());
                        println!("Logged in online as: {}", account.username.clone().green().bold());
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
                println!("Removed account profile: {}", account.username.yellow());
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
            println!("{}", "Launcher Settings:".cyan().bold());
            println!("  {:<22} {}", "Game Directory:".bold(), config.game_dir.display());
            println!("  {:<22} {}", "Java Executable Path:".bold(), config.java_path.display());
            println!("  {:<22} {}", "JVM Extra Arguments:".bold(), config.jvm_args.join(" "));
        }
        SettingsAction::SetGameDir { path } => {
            config.game_dir = std::path::PathBuf::from(path.clone());
            config.save()?;
            println!("{} set game directory to: {}", "Successfully".green().bold(), path);
        }
        SettingsAction::SetJava { path } => {
            config.java_path = std::path::PathBuf::from(path.clone());
            config.save()?;
            println!("{} set Java executable path to: {}", "Successfully".green().bold(), path);
        }
        SettingsAction::SetJvmArgs { args } => {
            config.jvm_args = args.split_whitespace().map(|s| s.to_string()).collect();
            config.save()?;
            println!("{} set JVM arguments to: {}", "Successfully".green().bold(), args);
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
        println!("{} {}...", "Fetching Fabric profile for loader".cyan(), loader_ver.clone().yellow());
        let profile = api.fetch_fabric_profile(game_version, &loader_ver).await?;
        
        let version_dir = game_dir.join("versions").join(&version_id);
        std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;
        let json_path = version_dir.join(format!("{}.json", version_id));
        let json_str = serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())?;
        std::fs::write(&json_path, json_str).map_err(|e| e.to_string())?;

        Ok(version_id)
    } else if loader_lower == "forge" || loader_lower == "neoforge" {
        let is_neoforge = loader_lower == "neoforge";
        println!("{} {} version index...", "Fetching".cyan(), if is_neoforge { "NeoForge" } else { "Forge" });
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

        println!(
            "{} {} profile for version {}...",
            "Fetching".cyan(),
            if is_neoforge { "NeoForge" } else { "Forge" },
            selected_ver.clone().yellow()
        );
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
                println!("{}", "No instances configured. Use `minecli instance create` or TUI.".yellow());
            } else {
                println!("{}", "Available Instances:".cyan().bold());
                for inst in list {
                    let is_active = config.active_instance.as_ref() == Some(&inst.id);
                    let active_marker = if is_active { " (ACTIVE)".green().bold().to_string() } else { "".to_string() };
                    let bullet = if is_active { "•".green() } else { "•".dim() };
                    println!("  {} {} [{}] [Version: {}]{}", bullet, inst.config.name.bold(), inst.id.dim(), inst.config.version.yellow(), active_marker);
                }
            }
        }
        InstanceAction::Create { id, version, name, loader, loader_version } => {
            let name_str = name.unwrap_or_else(|| format!("{} Profile", id));
            let api = ApiClient::new();
            
            let resolved_version = if let Some(ref l) = loader {
                resolve_and_setup_loader(&api, &config.game_dir, &version, l, loader_version).await?
            } else {
                println!("Validating Minecraft version '{}'...", version.clone().yellow());
                let manifest = api.fetch_version_manifest().await?;
                if !manifest.versions.iter().any(|v| v.id == version) {
                    return Err(format!("Version '{}' not found in Mojang version manifest.", version));
                }
                version.clone()
            };

            let inst = Instance::create(&config.game_dir, &id, &name_str, &resolved_version)?;
            config.active_instance = Some(inst.id.clone());
            config.save()?;
            
            println!("Created instance '{}' ({} - {}) and set it as active.", id.clone().green().bold(), name_str, resolved_version.yellow());
            println!("Run `minecli launch {}` to play.", id.green());
        }
        InstanceAction::Delete { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            inst.delete()?;
            if config.active_instance.as_ref() == Some(&id) {
                config.active_instance = None;
                config.save()?;
            }
            println!("Deleted instance '{}'.", id.yellow());
        }
        InstanceAction::Backup { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Creating backup of instance '{}'...", id.cyan());
            let path = inst.backup()?;
            println!("{} created at: {}", "Backup".green().bold(), path.display().to_string().cyan());
        }
        InstanceAction::Restore { id, backup } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Restoring backup '{}' for instance '{}'...", backup.clone().cyan(), id.clone().cyan());
            inst.restore(&backup)?;
            println!("{}", "Instance restored successfully!".green().bold());
        }
        InstanceAction::ListBackups { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let backups = inst.list_backups();
            if backups.is_empty() {
                println!("No backups found for instance '{}'.", id.yellow());
            } else {
                println!("Available backups for instance '{}':", id.cyan().bold());
                for backup in backups {
                    println!("  {} {}", "•".cyan(), backup);
                }
            }
        }
        InstanceAction::ListMods { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let mods = inst.get_mods()?;
            if mods.is_empty() {
                println!("No mods found for instance '{}'.", id.yellow());
            } else {
                println!("Mods for instance '{}':", id.cyan().bold());
                for m in mods {
                    let status = if m.enabled { "ENABLED".green().bold() } else { "DISABLED".dim() };
                    let bullet = if m.enabled { "•".green() } else { "•".dim() };
                    let filename_padded = format!("{:<35}", m.filename);
                    println!("  {} {} [{}] (Version: {})", bullet, filename_padded.bold(), status, m.metadata.version.yellow());
                }
            }
        }
        InstanceAction::EnableMod { id, filename } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            inst.enable_mod(&filename)?;
            println!("Enabled mod '{}' in instance '{}'.", filename.green(), id.green());
        }
        InstanceAction::DisableMod { id, filename } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            inst.disable_mod(&filename)?;
            println!("Disabled mod '{}' in instance '{}'.", filename.yellow(), id.yellow());
        }
        InstanceAction::Sync { id } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Syncing mods for instance '{}'...", id.cyan());
            let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
            let game_dir = config.game_dir.clone();
            tokio::spawn(async move {
                let _ = inst.sync_mods(&game_dir, tx).await;
            });
            while let Some(update) = rx.recv().await {
                match update {
                    ProgressUpdate::Started { total, message } => {
                        println!("Sync started: {} (Total: {})", message.bold(), total.to_string().yellow());
                    }
                    ProgressUpdate::Progress { completed, total, current_file } => {
                        print!(
                            "\r[{}] Syncing: {}                             ",
                            format!("{}/{}", completed, total).cyan(),
                            current_file
                        );
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                    ProgressUpdate::Message(msg) => {
                        println!("\n{}", msg.green());
                    }
                    ProgressUpdate::Finished => {
                        println!("\n{}", "Mod sync completed successfully!".green().bold());
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
                    println!("Cleared custom Java path for instance '{}'.", id.clone().cyan());
                } else {
                    inst.config.java_path = Some(path);
                    println!("Set custom Java path for instance '{}' to: {}", id.clone().cyan(), inst.config.java_path.as_ref().unwrap().clone().green());
                }
                modified = true;
            }

            if let Some(ver) = java_version {
                if ver == 0 {
                    inst.config.java_version = None;
                    println!("Cleared custom JRE version for instance '{}'.", id.clone().cyan());
                } else {
                    inst.config.java_version = Some(ver);
                    println!("Set custom JRE version for instance '{}' to: Java {}", id.clone().cyan(), ver.to_string().green());
                }
                modified = true;
            }

            if modified {
                inst.save()?;
                println!("Saved settings for instance '{}'.", id.clone().green());
            } else {
                println!("{}", "No changes specified. Use `--java-path` or `--java-version`.".yellow());
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

            println!("Importing modpack '{}' as instance '{}'...", path.cyan(), custom_id.clone().green());

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
                    ProgressUpdate::Message(msg) => println!("  {}", msg.green()),
                    ProgressUpdate::Started { total, message } => println!("  {} ({} files)", message.bold(), total.to_string().yellow()),
                    ProgressUpdate::Progress { completed, total, current_file } => {
                        println!("  [{}] {}", format!("{}/{}", completed, total).cyan(), current_file);
                    }
                    ProgressUpdate::Finished => break,
                    ProgressUpdate::Error(e) => {
                        eprintln!("  {}: {}", "Error".red().bold(), e);
                    }
                }
            }

            match handle.await {
                Ok(Ok(inst)) => {
                    println!("{} imported modpack as instance '{}'.", "Successfully".green().bold(), inst.id.green().bold());
                    println!("  Version: {}", inst.config.version.yellow());
                    if let Some(mods) = &inst.config.mods {
                        println!("  Mods: {} declared", mods.len().to_string().cyan());
                    }
                }
                Ok(Err(e)) => return Err(format!("Import failed: {}", e)),
                Err(e) => return Err(format!("Import task panicked: {}", e)),
            }
        }
        InstanceAction::SearchPack { query } => {
            let api = ApiClient::new();
            println!("Searching Modrinth for modpacks matching '{}'...", query.clone().cyan());
            let hits = api.search_modpacks(&query).await?;
            if hits.is_empty() {
                println!("{}", "No modpacks found.".yellow());
            } else {
                let col1 = format!("{:<24}", "Title");
                let col2 = format!("{:<20}", "ID/Slug");
                let col3 = format!("{:<12}", "Downloads");
                println!("{} | {} | {} | {}", col1.cyan().bold(), col2.cyan().bold(), col3.cyan().bold(), "Description".cyan().bold());
                println!("{}", "-".repeat(100).dim());
                for hit in hits {
                    let desc = if hit.description.len() > 40 {
                        format!("{}...", &hit.description[..37])
                    } else {
                        hit.description.clone()
                    };
                    let title_padded = format!("{:<24}", hit.title);
                    let id_padded = format!("{:<20}", hit.project_id);
                    let downloads_padded = format!("{:<12}", hit.downloads);
                    println!(
                        "{} | {} | {} | {}",
                        title_padded.bold(),
                        id_padded.dim(),
                        downloads_padded.green(),
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
            println!("Exporting instance '{}' to '{}'...", id.cyan(), out_path.display().to_string().cyan());
            inst.export_mrpack(&out_path)?;
            println!("{}", "Successfully exported modpack!".green().bold());
        }
        InstanceAction::SearchMod { id, query } => {
            let inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let (game_version, loader) = inst.get_game_version_and_loader(&config.game_dir);
            
            println!(
                "Searching Modrinth for mods matching '{}' compatible with Minecraft {} ({})...",
                query.clone().cyan(),
                game_version.clone().yellow(),
                loader.as_deref().unwrap_or("vanilla").yellow()
            );
            
            let api = ApiClient::new();
            let hits = api.search_mods(&query, Some(&game_version), loader.as_deref()).await?;
            
            if hits.is_empty() {
                println!("{}", "No compatible mods found.".yellow());
            } else {
                let col1 = format!("{:<24}", "Title");
                let col2 = format!("{:<20}", "ID/Slug");
                let col3 = format!("{:<12}", "Downloads");
                println!("{} | {} | {} | {}", col1.cyan().bold(), col2.cyan().bold(), col3.cyan().bold(), "Description".cyan().bold());
                println!("{}", "-".repeat(100).dim());
                for hit in hits {
                    let desc = if hit.description.len() > 40 {
                        format!("{}...", &hit.description[..37])
                    } else {
                        hit.description.clone()
                    };
                    let title_padded = format!("{:<24}", hit.title);
                    let id_padded = format!("{:<20}", hit.project_id);
                    let downloads_padded = format!("{:<12}", hit.downloads);
                    println!(
                        "{} | {} | {} | {}",
                        title_padded.bold(),
                        id_padded.dim(),
                        downloads_padded.green(),
                        desc
                    );
                }
            }
        }
        InstanceAction::AddMod { id, mod_id, no_deps, save } => {
            let mut inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let (game_version, loader) = inst.get_game_version_and_loader(&config.game_dir);
            let api = ApiClient::new();
            
            let mut installed_projects = std::collections::HashSet::new();
            // Populate installed_projects with already installed mods' IDs and slugs to avoid re-downloads
            if let Ok(existing_mods) = inst.get_mods() {
                for m in existing_mods {
                    installed_projects.insert(m.metadata.id.clone());
                    installed_projects.insert(m.metadata.name.clone());
                }
            }

            install_mod_recursive(
                &api,
                &config.game_dir,
                &mut inst,
                &mod_id,
                &game_version,
                loader.as_deref(),
                no_deps,
                save,
                &mut installed_projects,
            ).await?;

            println!("{}", "Finished adding mods!".green().bold());
        }
        InstanceAction::RemoveMod { id, filename_or_id } => {
            let mut inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            println!("Deleting mod '{}' from instance '{}'...", filename_or_id.clone().cyan(), id.cyan());
            inst.remove_mod(&filename_or_id, true)?;
            println!("{}", "Mod removed successfully!".green().bold());
        }
        InstanceAction::UpdateMods { id, yes } => {
            let mut inst = Instance::load(&id, config.game_dir.join("instances").join(&id))?;
            let (game_version, loader) = inst.get_game_version_and_loader(&config.game_dir);
            let mods = inst.get_mods()?;
            
            if mods.is_empty() {
                println!("No mods installed in instance '{}'.", id.yellow());
                return Ok(());
            }

            println!("Checking for updates for {} mods...", mods.len().to_string().cyan());
            let api = ApiClient::new();
            let mut updates = Vec::new();
            
            for m in &mods {
                let project_id_or_slug = m.metadata.id.clone();
                if let Ok(versions) = api.fetch_modpack_versions(&project_id_or_slug).await {
                    let compatible_version = versions.into_iter().find(|v| {
                        let matches_game = v.game_versions.contains(&game_version);
                        let matches_loader = match loader.as_deref() {
                            Some(l) => v.loaders.iter().any(|loader_name| loader_name.to_lowercase() == l.to_lowercase()),
                            None => true,
                        };
                        matches_game && matches_loader
                    });

                    if let Some(latest_ver) = compatible_version {
                        if latest_ver.version_number != m.metadata.version {
                            updates.push((m.clone(), latest_ver));
                        }
                    }
                }
            }

            if updates.is_empty() {
                println!("{}", "All mods are up to date!".green().bold());
                return Ok(());
            }

            println!("\nUpdates available:");
            for (local_mod, remote_ver) in &updates {
                println!(
                    "  • {}: {} -> {}",
                    local_mod.metadata.name.clone().bold(),
                    local_mod.metadata.version.clone().red(),
                    remote_ver.version_number.clone().green()
                );
            }

            let apply_updates = if yes {
                true
            } else {
                print!("\nApply all updates? [Y/n]: ");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input).is_ok()
                    && (input.trim().is_empty() || input.trim().to_lowercase().starts_with('y'))
            };

            if apply_updates {
                for (local_mod, remote_ver) in updates {
                    if let Some(file) = remote_ver.files.iter().find(|f| f.primary || f.filename.ends_with(".jar"))
                        .or_else(|| remote_ver.files.first()) {
                            println!("Updating {}...", local_mod.metadata.name.clone().cyan());
                            let _ = inst.remove_mod(&local_mod.filename, false);
                            if let Err(e) = inst.install_mod_from_url(&config.game_dir, &file.filename, &file.url, None, true).await {
                                println!("Warning: Failed to update mod {}: {}", local_mod.metadata.name.clone(), e);
                            } else {
                                println!("Updated {} to {}!", local_mod.metadata.name.clone().green(), remote_ver.version_number.clone().yellow());
                            }
                        }
                }
                println!("{}", "Finished applying updates!".green().bold());
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
        if let Some(acc) = config.get_active_account()
            && acc.account_type == AccountType::Microsoft {
                target_account = Some(acc.clone());
            }
        if target_account.is_none() {
            target_account = config.accounts.iter().find(|a| a.account_type == AccountType::Microsoft).cloned();
        }
        if target_account.is_none()
            && let Some(acc) = config.get_active_account()
                && acc.account_type == AccountType::Offline {
                    target_account = Some(acc.clone());
                }

        match target_account {
            Some(mut acc) => {
                if acc.account_type == AccountType::Microsoft {
                    if let Some(ref auth) = acc.microsoft_auth {
                        let is_expired = auth.expires_at.map(|exp| exp < chrono::Utc::now()).unwrap_or(true);
                        if is_expired {
                            println!("{}", "Session expired. Refreshing Microsoft tokens...".yellow());
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
                    println!("Logged in online as: {}", acc.username.clone().green().bold());
                    acc
                } else {
                    println!("Logged in offline as: {}", acc.username.clone().green().bold());
                    acc
                }
            }
            None => {
                println!("{}", "No account configured. Starting Microsoft Online Login...".yellow());
                let dev_code = api.request_device_code().await?;
                println!("{}", "To log in, open a web browser and navigate to:".cyan());
                println!("  {}", dev_code.verification_uri.cyan().underlined());
                println!("{}", "Enter the code below to authorize this launcher:".cyan());
                println!("  {}", dev_code.user_code.green().bold());
                println!("{}", "Waiting for authentication...".yellow().italic());

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
                            
                            println!("{}", "Success!".green().bold());
                            println!("Logged in online as: {}", account.username.clone().green().bold());
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

    println!("{}", "Preparing launch parameters...".cyan());
    let launcher = Launcher::new(config);
    launcher.launch(&instance, &account).await?;

    Ok(())
}

async fn install_mod_recursive(
    api: &ApiClient,
    game_dir: &std::path::Path,
    inst: &mut Instance,
    project_id_or_slug: &str,
    game_version: &str,
    loader: Option<&str>,
    no_deps: bool,
    save: bool,
    installed_projects: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    if installed_projects.contains(project_id_or_slug) {
        return Ok(());
    }

    println!("Resolving version for mod '{}'...", project_id_or_slug.cyan());
    
    let versions = api.fetch_modpack_versions(project_id_or_slug).await?;
    let compatible_version = versions.into_iter().find(|v| {
        let matches_game = v.game_versions.contains(&game_version.to_string());
        let matches_loader = match loader {
            Some(l) => v.loaders.iter().any(|loader_name| loader_name.to_lowercase() == l.to_lowercase()),
            None => true,
        };
        matches_game && matches_loader
    });

    let ver = match compatible_version {
        Some(v) => v,
        None => {
            return Err(format!(
                "No compatible version of mod '{}' found for Minecraft {} and loader '{}'.",
                project_id_or_slug,
                game_version,
                loader.unwrap_or("vanilla")
            ));
        }
    };

    let file = ver.files.iter().find(|f| f.primary || f.filename.ends_with(".jar"))
        .or_else(|| ver.files.first())
        .ok_or_else(|| format!("No file found in version {} for mod {}", ver.name, project_id_or_slug))?;

    println!("Downloading {} (version: {})...", file.filename.clone().green(), ver.version_number.clone().yellow());
    
    inst.install_mod_from_url(game_dir, &file.filename, &file.url, None, save).await?;
    
    installed_projects.insert(project_id_or_slug.to_string());
    if let Ok(project) = api.fetch_project(project_id_or_slug).await {
        installed_projects.insert(project.id);
        installed_projects.insert(project.slug);
    }

    if !no_deps {
        for dep in &ver.dependencies {
            if dep.dependency_type == "required" {
                if let Some(ref dep_project_id) = dep.project_id {
                    if installed_projects.contains(dep_project_id) {
                        continue;
                    }
                    
                    let dep_name = match api.fetch_project(dep_project_id).await {
                        Ok(p) => p.title,
                        Err(_) => dep_project_id.clone(),
                    };
                    
                    println!("Installing required dependency: {}", dep_name.clone().yellow().bold());
                    
                    if let Err(e) = Box::pin(install_mod_recursive(
                        api,
                        game_dir,
                        inst,
                        dep_project_id,
                        game_version,
                        loader,
                        no_deps,
                        save,
                        installed_projects,
                    )).await {
                        println!("Warning: Failed to install dependency {}: {}", dep_name, e);
                    }
                }
            }
        }
    }

    Ok(())
}
