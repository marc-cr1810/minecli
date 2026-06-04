use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use directories::BaseDirs;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum AccountType {
    Offline,
    Microsoft,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MicrosoftAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Account {
    pub uuid: String,
    pub username: String,
    pub account_type: AccountType,
    pub microsoft_auth: Option<MicrosoftAuth>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub game_dir: PathBuf,
    pub java_path: PathBuf,
    pub jvm_args: Vec<String>,
    pub accounts: Vec<Account>,
    pub active_account_uuid: Option<String>,
    pub selected_version: Option<String>,
    pub active_instance: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let game_dir = Self::default_game_dir();
        Self {
            game_dir,
            java_path: PathBuf::from("java"),
            jvm_args: vec!["-Xmx2G".to_string()],
            accounts: Vec::new(),
            active_account_uuid: None,
            selected_version: None,
            active_instance: None,
        }
    }
}

impl Config {
    pub fn default_game_dir() -> PathBuf {
        if let Some(base_dirs) = BaseDirs::new() {
            base_dirs.home_dir().join(".minecraft")
        } else {
            PathBuf::from(".minecraft")
        }
    }

    pub fn config_path() -> Option<PathBuf> {
        BaseDirs::new().map(|base_dirs| {
            base_dirs.config_dir().join("minecli").join("config.json")
        })
    }

    pub fn load() -> Self {
        let mut config = if let Some(path) = Self::config_path() {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(cfg) = serde_json::from_str::<Config>(&content) {
                        cfg
                    } else {
                        Config::default()
                    }
                } else {
                    Config::default()
                }
            } else {
                Config::default()
            }
        } else {
            Config::default()
        };
        config.migrate_and_initialize();
        config
    }

    pub fn migrate_and_initialize(&mut self) {
        let instances_dir = self.game_dir.join("instances");
        let _ = fs::create_dir_all(&instances_dir);

        if self.active_instance.is_none() {
            let list = crate::instance::Instance::load_all(&self.game_dir);
            if let Some(first) = list.first() {
                self.active_instance = Some(first.id.clone());
            } else {
                let version = self.selected_version.clone().unwrap_or_else(|| "1.21.1".to_string());
                if let Ok(inst) = crate::instance::Instance::create(&self.game_dir, "default", "Default Profile", &version) {
                    self.active_instance = Some(inst.id);
                }
            }
            let _ = self.save();
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or("Could not locate config directory")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_active_account(&self) -> Option<&Account> {
        let uuid = self.active_account_uuid.as_ref()?;
        self.accounts.iter().find(|acc| &acc.uuid == uuid)
    }

    pub fn add_account(&mut self, account: Account) {
        // Remove existing account with the same UUID or username
        self.accounts.retain(|acc| acc.uuid != account.uuid);
        self.active_account_uuid = Some(account.uuid.clone());
        self.accounts.push(account);
        let _ = self.save();
    }

    pub fn remove_account(&mut self, uuid: &str) {
        self.accounts.retain(|acc| acc.uuid != uuid);
        if self.active_account_uuid.as_deref() == Some(uuid) {
            self.active_account_uuid = self.accounts.first().map(|acc| acc.uuid.clone());
        }
        let _ = self.save();
    }
}
