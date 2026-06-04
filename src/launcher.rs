use std::collections::HashMap;
use std::fs;
use std::process::{Command, Stdio};

use crate::config::{Config, Account, AccountType};
use crate::api::{VersionDetails, Rule, ArgumentValue, ArgumentValueList};

pub struct Launcher {
    config: Config,
}

impl Launcher {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn get_available_local_versions(&self) -> Vec<String> {
        let versions_dir = self.config.game_dir.join("versions");
        if !versions_dir.exists() {
            return Vec::new();
        }
        let mut local_versions = Vec::new();
        if let Ok(entries) = fs::read_dir(versions_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let version_id = entry.file_name().to_string_lossy().to_string();
                    let json_path = entry.path().join(format!("{}.json", version_id));
                    if json_path.exists() {
                        local_versions.push(version_id);
                    }
                }
            }
        }
        local_versions.sort();
        local_versions
    }

    pub fn load_version_details(&self, version_id: &str) -> Result<VersionDetails, String> {
        let json_path = self.config.game_dir
            .join("versions")
            .join(version_id)
            .join(format!("{}.json", version_id));

        if !json_path.exists() {
            return Err(format!("Version details JSON does not exist for {}", version_id));
        }

        let content = fs::read_to_string(json_path)
            .map_err(|e| format!("Failed to read version details: {}", e))?;

        let details: VersionDetails = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse version JSON: {}", e))?;

        Ok(details)
    }

    fn build_classpath(&self, details: &VersionDetails) -> Result<String, String> {
        let mut classpath_entries = Vec::new();
        let libraries_dir = self.config.game_dir.join("libraries");

        for lib in &details.libraries {
            if let Some(ref rules) = lib.rules {
                if !Rule::evaluate(rules) {
                    continue;
                }
            }

            if let Some(ref art) = lib.downloads.artifact {
                let lib_path = libraries_dir.join(&art.path);
                classpath_entries.push(lib_path);
            }
        }

        // Add client jar itself
        let client_jar = self.config.game_dir
            .join("versions")
            .join(&details.id)
            .join(format!("{}.jar", details.id));
        classpath_entries.push(client_jar);

        let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
        let paths: Vec<String> = classpath_entries
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        Ok(paths.join(sep))
    }

    fn replace_placeholders(&self, template: &str, vars: &HashMap<&str, String>) -> String {
        let mut result = template.to_string();
        for (key, val) in vars {
            let placeholder = format!("${{{}}}", key);
            result = result.replace(&placeholder, val);
        }
        result
    }

    pub fn prepare_launch(&self, version_id: &str, account: &Account) -> Result<Command, String> {
        let details = self.load_version_details(version_id)?;

        let classpath = self.build_classpath(&details)?;
        let version_dir = self.config.game_dir.join("versions").join(version_id);
        let natives_dir = version_dir.join("natives");

        // Define template variables
        let mut vars = HashMap::new();
        vars.insert("auth_player_name", account.username.clone());
        vars.insert("version_name", version_id.to_string());
        vars.insert("game_directory", self.config.game_dir.to_string_lossy().to_string());
        vars.insert("assets_root", self.config.game_dir.join("assets").to_string_lossy().to_string());
        vars.insert("assets_index_name", details.assetIndex.id.clone());
        vars.insert("auth_uuid", account.uuid.clone());
        
        let token = if let Some(ref ms) = account.microsoft_auth {
            ms.access_token.clone()
        } else {
            "null".to_string()
        };
        vars.insert("auth_access_token", token);

        let user_type = match account.account_type {
            AccountType::Microsoft => "msa".to_string(),
            AccountType::Offline => "legacy".to_string(),
        };
        vars.insert("user_type", user_type);
        vars.insert("version_type", details.r#type.clone());
        vars.insert("natives_directory", natives_dir.to_string_lossy().to_string());
        vars.insert("classpath", classpath);
        vars.insert("user_properties", "{}".to_string());

        let mut jvm_args = Vec::new();
        let mut game_args = Vec::new();

        // 1. Process JVM Arguments
        if let Some(ref args) = details.arguments {
            // Modern arguments format
            for arg_val in &args.jvm {
                match arg_val {
                    ArgumentValue::Simple(s) => {
                        jvm_args.push(self.replace_placeholders(s, &vars));
                    }
                    ArgumentValue::Complex { rules, value } => {
                        if Rule::evaluate(rules) {
                            match value {
                                ArgumentValueList::Single(s) => {
                                    jvm_args.push(self.replace_placeholders(s, &vars));
                                }
                                ArgumentValueList::Many(list) => {
                                    for s in list {
                                        jvm_args.push(self.replace_placeholders(s, &vars));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // Legacy arguments format (use defaults for JVM)
            jvm_args.push(format!("-Djava.library.path={}", natives_dir.to_string_lossy()));
            jvm_args.push(format!("-Djna.tmpdir={}", natives_dir.to_string_lossy()));
            jvm_args.push(format!("-Dorg.lwjgl.system.SharedLibraryExtractPath={}", natives_dir.to_string_lossy()));
            jvm_args.push("-cp".to_string());
            jvm_args.push("${classpath}".to_string()); // Placeholder will be replaced below
        }

        // Add config/user JVM arguments (like -Xmx2G)
        for arg in &self.config.jvm_args {
            jvm_args.push(arg.clone());
        }

        // Apply placeholders on final jvm args list (in case classpath or others are still templated)
        jvm_args = jvm_args
            .into_iter()
            .map(|arg| self.replace_placeholders(&arg, &vars))
            .collect();

        // 2. Process Game Arguments
        if let Some(ref args) = details.arguments {
            // Modern arguments format
            for arg_val in &args.game {
                match arg_val {
                    ArgumentValue::Simple(s) => {
                        game_args.push(self.replace_placeholders(s, &vars));
                    }
                    ArgumentValue::Complex { rules, value } => {
                        if Rule::evaluate(rules) {
                            match value {
                                ArgumentValueList::Single(s) => {
                                    game_args.push(self.replace_placeholders(s, &vars));
                                }
                                ArgumentValueList::Many(list) => {
                                    for s in list {
                                        game_args.push(self.replace_placeholders(s, &vars));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else if let Some(ref legacy_args) = details.minecraftArguments {
            // Legacy arguments string
            for part in legacy_args.split_whitespace() {
                game_args.push(self.replace_placeholders(part, &vars));
            }
        }

        // 3. Construct command
        let mut cmd = Command::new(&self.config.java_path);
        cmd.current_dir(&self.config.game_dir);
        
        // Pass JVM args
        cmd.args(&jvm_args);
        
        // Main Class
        cmd.arg(&details.mainClass);

        // Pass game args
        cmd.args(&game_args);

        Ok(cmd)
    }

    pub fn launch(&self, version_id: &str, account: &Account) -> Result<(), String> {
        let mut cmd = self.prepare_launch(version_id, account)?;
        
        // Redirect stdout/stderr to parent process so standard terminal logging works
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        let mut child = cmd.spawn()
            .map_err(|e| format!("Failed to spawn Java process: {}. Is Java installed and configured correctly?", e))?;
        
        let status = child.wait()
            .map_err(|e| format!("Minecraft game process error: {}", e))?;

        if !status.success() {
            return Err(format!("Minecraft exited with non-zero code: {:?}", status.code()));
        }

        Ok(())
    }
}
