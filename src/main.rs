mod config;
mod api;
mod downloader;
mod launcher;
mod tui;

use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::{Config, Account, AccountType, MicrosoftAuth};
use crate::api::ApiClient;
use crate::downloader::{Downloader, ProgressUpdate};
use crate::launcher::Launcher;

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
        /// Minecraft version ID (e.g., 1.20.4, 1.12.2)
        version: String,

        /// Offline username to run with (ignored if online is active)
        #[arg(short, long)]
        username: Option<String>,

        /// Force online login via Microsoft OAuth Device Code
        #[arg(short, long)]
        online: bool,

        /// Do not check or update files, launch immediately
        #[arg(short, long)]
        offline_mode: bool,
    },
    /// List all locally downloaded versions
    List,
    /// List all remote versions available from Mojang
    ListRemote,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    
    match cli.command {
        Some(Commands::Launch { version, username, online, offline_mode }) => {
            if let Err(e) = handle_cli_launch(version, username, online, offline_mode).await {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::List) => {
            let config = Config::load();
            let launcher = Launcher::new(config);
            let versions = launcher.get_available_local_versions();
            if versions.is_empty() {
                println!("No local versions downloaded. Run `minecli` to select and download one.");
            } else {
                println!("Downloaded Minecraft versions:");
                for v in versions {
                    println!(" - {}", v);
                }
            }
        }
        Some(Commands::ListRemote) => {
            println!("Fetching available Minecraft versions...");
            let api = ApiClient::new();
            match api.fetch_version_manifest().await {
                Ok(manifest) => {
                    println!("{:<18} | {:<10} | {}", "Version ID", "Type", "Release Time");
                    println!("{}", "-".repeat(50));
                    for v in manifest.versions.iter().take(40) {
                        println!("{:<18} | {:<10} | {}", v.id, v.r#type, v.releaseTime);
                    }
                    if manifest.versions.len() > 40 {
                        println!("... and {} more. Launch with `minecli launch <version>` to play.", manifest.versions.len() - 40);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to fetch remote versions: {}", e);
                    std::process::exit(1);
                }
            }
        }
        None => {
            // Run interactive TUI
            if let Err(e) = tui::run_tui().await {
                eprintln!("Launcher crashed: {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn handle_cli_launch(
    version_id: String,
    username_override: Option<String>,
    force_online: bool,
    skip_downloads: bool,
) -> Result<(), String> {
    let mut config = Config::load();
    let api = ApiClient::new();

    // 1. Resolve Account
    let account = if force_online {
        // Authenticate Online
        println!("Microsoft Online login requested.");
        let mut active_account = None;

        // Try to find cached Microsoft Account
        for acc in &config.accounts {
            if acc.account_type == AccountType::Microsoft {
                active_account = Some(acc.clone());
                break;
            }
        }

        match active_account {
            Some(mut acc) => {
                // Check if token is expired, refresh if needed
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
            }
            None => {
                // Perform Device Code OAuth flow in terminal
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
                        Ok(None) => {} // pending
                        Err(e) => {
                            return Err(format!("Authentication failed: {}", e));
                        }
                    }
                }

                auth_account.ok_or("Authentication timed out or cancelled.")?
            }
        }
    } else {
        // Resolve Offline Account
        let username = username_override
            .or_else(|| config.get_active_account().map(|a| a.username.clone()))
            .unwrap_or_else(|| "Player".to_string());

        // Find existing offline account
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
    };

    // 2. Download Game Files if needed
    let version_json_path = config.game_dir
        .join("versions")
        .join(&version_id)
        .join(format!("{}.json", version_id));

    if !skip_downloads && (!version_json_path.exists() || !config.game_dir.join("versions").join(&version_id).join(format!("{}.jar", version_id)).exists()) {
        println!("Version JSON/JAR not found locally. Preparing to download {}...", version_id);
        
        // Fetch Version Details
        let manifest = api.fetch_version_manifest().await?;
        let brief = manifest.versions.iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| format!("Minecraft version '{}' not found in Mojang version manifest.", version_id))?;

        let details = api.fetch_version_details(&brief.url).await?;
        
        // Save details locally
        if let Some(parent) = version_json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = serde_json::to_string_pretty(&details).map_err(|e| e.to_string())?;
        std::fs::write(&version_json_path, content).map_err(|e| e.to_string())?;

        // Download version assets & libraries
        let (tx, mut rx) = mpsc::channel::<ProgressUpdate>(100);
        let downloader = Downloader::new(tx);
        let game_dir = config.game_dir.clone();
        
        tokio::spawn(async move {
            let _ = downloader.download_version(&game_dir, &details).await;
        });

        // Simple CLI progress indicator
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
    }

    // 3. Launch Minecraft
    println!("Preparing launch parameters...");
    let launcher = Launcher::new(config);
    launcher.launch(&version_id, &account)?;

    Ok(())
}
