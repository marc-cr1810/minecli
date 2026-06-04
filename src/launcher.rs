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

    pub async fn prepare_launch(
        &self,
        instance: &crate::instance::Instance,
        account: &Account,
        log_tx: &Option<tokio::sync::mpsc::Sender<String>>,
    ) -> Result<Command, String> {
        let version_id = &instance.config.version;
        let details = self.load_version_details(version_id)?;

        let classpath = self.build_classpath(&details)?;
        let version_dir = self.config.game_dir.join("versions").join(version_id);
        let natives_dir = version_dir.join("natives");

        // Define template variables
        let mut vars = HashMap::new();
        vars.insert("auth_player_name", account.username.clone());
        vars.insert("version_name", version_id.to_string());
        vars.insert("game_directory", instance.path.to_string_lossy().to_string());
        vars.insert("assets_root", self.config.game_dir.join("assets").to_string_lossy().to_string());
        vars.insert("assets_index_name", details.assetIndex.id.clone());
        vars.insert("auth_uuid", format_uuid_with_hyphens(&account.uuid));
        
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
        vars.insert("resolution_width", "854".to_string());
        vars.insert("resolution_height", "480".to_string());
        vars.insert("clientid", "minecli".to_string());
        let xuid = if let Some(ref ms) = account.microsoft_auth {
            extract_xuid_from_token(&ms.access_token).unwrap_or_else(|| "dummy".to_string())
        } else {
            "dummy".to_string()
        };
        vars.insert("auth_xuid", xuid);
        vars.insert("launcher_name", "minecli".to_string());
        vars.insert("launcher_version", "0.1.0".to_string());

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
                                    let replaced = self.replace_placeholders(s, &vars);
                                    if !replaced.contains("${") {
                                        jvm_args.push(replaced);
                                    }
                                }
                                ArgumentValueList::Many(list) => {
                                    let mut replaced_list = Vec::new();
                                    let mut has_unreplaced = false;
                                    for s in list {
                                        let replaced = self.replace_placeholders(s, &vars);
                                        if replaced.contains("${") {
                                            has_unreplaced = true;
                                            break;
                                        }
                                        replaced_list.push(replaced);
                                    }
                                    if !has_unreplaced {
                                        jvm_args.extend(replaced_list);
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
        let custom_jvm = instance.config.jvm_args.as_ref().unwrap_or(&self.config.jvm_args);
        for arg in custom_jvm {
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
                                    let replaced = self.replace_placeholders(s, &vars);
                                    if !replaced.contains("${") {
                                        game_args.push(replaced);
                                    }
                                }
                                ArgumentValueList::Many(list) => {
                                    let mut replaced_list = Vec::new();
                                    let mut has_unreplaced = false;
                                    for s in list {
                                        let replaced = self.replace_placeholders(s, &vars);
                                        if replaced.contains("${") {
                                            has_unreplaced = true;
                                            break;
                                        }
                                        replaced_list.push(replaced);
                                    }
                                    if !has_unreplaced {
                                        game_args.extend(replaced_list);
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

        let log_info = |msg: String| {
            if let Some(ref tx) = *log_tx {
                let _ = tx.try_send(msg);
            } else {
                println!("{}", msg);
            }
        };

        // 3. Construct command
        let java_version;
        let java_exe = if let Some(ref inst_java_path_str) = instance.config.java_path {
            let path = std::path::PathBuf::from(inst_java_path_str);
            if path.exists() {
                log_info(format!("Using instance custom Java path: {}", inst_java_path_str));
                java_version = instance.config.java_version.unwrap_or(21);
                path
            } else {
                return Err(format!("Custom instance Java path does not exist: {}", inst_java_path_str));
            }
        } else if self.config.java_path != std::path::Path::new("java") && self.config.java_path.exists() {
            log_info(format!("Using global custom Java path: {}", self.config.java_path.display()));
            java_version = instance.config.java_version.unwrap_or(21);
            std::path::PathBuf::from(&self.config.java_path)
        } else {
            let jv = instance.config.java_version
                .or_else(|| details.javaVersion.as_ref().map(|jv| jv.majorVersion))
                .unwrap_or(17);
            java_version = jv;
            let log_tx_clone = log_tx.clone();
            crate::java::install_java_if_needed(&self.config.game_dir, jv, move |msg| {
                if let Some(ref tx) = log_tx_clone {
                    let _ = tx.try_send(msg);
                } else {
                    println!("{}", msg);
                }
            }).await?
        };

        // Filter out incompatible JVM arguments for Java versions < 22
        let mut final_jvm_args = jvm_args.clone();
        if java_version < 22 {
            final_jvm_args.retain(|arg| arg != "--sun-misc-unsafe-memory-access=allow");
        }

        let mut cmd = Command::new(java_exe);
        cmd.current_dir(&instance.path);
        
        // Pass JVM args
        cmd.args(&final_jvm_args);
        
        // Main Class
        cmd.arg(&details.mainClass);

        // Pass game args
        cmd.args(&game_args);

        Ok(cmd)
    }

    pub async fn launch(&self, instance: &crate::instance::Instance, account: &Account) -> Result<(), String> {
        self.launch_with_logs(instance, account, None).await
    }

    pub async fn launch_with_logs(
        &self, 
        instance: &crate::instance::Instance, 
        account: &Account,
        log_tx: Option<tokio::sync::mpsc::Sender<String>>
    ) -> Result<(), String> {
        let interpolate = |cmd_str: &str| -> String {
            cmd_str
                .replace("${game_directory}", &instance.path.to_string_lossy())
                .replace("${version_id}", &instance.config.version)
        };

        let log_info = |msg: String| {
            if let Some(ref tx) = log_tx {
                let _ = tx.try_send(msg);
            } else {
                println!("{}", msg);
            }
        };

        // Run pre-launch hook if present
        if let Some(ref pre_cmd) = instance.config.pre_launch {
            if !pre_cmd.trim().is_empty() {
                let interpolated = interpolate(pre_cmd);
                log_info(format!("Running pre-launch hook: {}", interpolated));
                let status = run_hook_command(&interpolated, &instance.path)?;
                if !status.success() {
                    return Err(format!("Pre-launch hook exited with failure code: {:?}", status.code()));
                }
            }
        }

        let mut cmd = self.prepare_launch(instance, account, &log_tx).await?;
        
        let status = if let Some(tx) = log_tx.clone() {
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            log_info(format!("Launch command: {:?}", cmd));
            let mut child = cmd.spawn()
                .map_err(|e| format!("Failed to spawn Java process: {}. Is Java installed and configured correctly?", e))?;

            let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
            let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

            use std::io::{BufRead, BufReader};
            let tx_out = tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    let _ = tx_out.blocking_send(line);
                }
            });

            let tx_err = tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let _ = tx_err.blocking_send(line);
                }
            });

            child.wait()
                .map_err(|e| format!("Minecraft game process error: {}", e))?
        } else {
            // Redirect stdout/stderr to parent process so standard terminal logging works
            cmd.stdout(Stdio::inherit());
            cmd.stderr(Stdio::inherit());

            println!("Launch command: {:?}", cmd);
            let mut child = cmd.spawn()
                .map_err(|e| format!("Failed to spawn Java process: {}. Is Java installed and configured correctly?", e))?;
            
            child.wait()
                .map_err(|e| format!("Minecraft game process error: {}", e))?
        };

        // Run post-exit hook if present
        if let Some(ref post_cmd) = instance.config.post_exit {
            if !post_cmd.trim().is_empty() {
                let interpolated = interpolate(post_cmd);
                log_info(format!("Running post-exit hook: {}", interpolated));
                if let Err(e) = run_hook_command(&interpolated, &instance.path) {
                    log_info(format!("Warning: post-exit hook failed: {}", e));
                }
            }
        }

        if !status.success() {
            if log_tx.is_none() {
                // Read latest.log
                let log_path = instance.path.join("logs").join("latest.log");
                if let Ok(content) = std::fs::read_to_string(&log_path) {
                    if let Some(analysis) = crate::crash_analyzer::analyze_crash(&instance.path, &content) {
                        println!("\n==================================================");
                        println!("[!] CRASH DIAGNOSTICS DETECTED:");
                        println!("Title: {}", analysis.title);
                        println!("Cause: {}", analysis.description);
                        println!("Solutions:");
                        for sol in &analysis.possible_solutions {
                            println!("  - {}", sol);
                        }
                        println!("==================================================\n");
                    }
                }
            }
            return Err(format!("Minecraft exited with non-zero code: {:?}", status.code()));
        }

        Ok(())
    }
}

fn run_hook_command(command_str: &str, cwd: &std::path::Path) -> Result<std::process::ExitStatus, String> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd.exe");
        c.arg("/C").arg(command_str);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command_str);
        c
    };
    cmd.current_dir(cwd);
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    cmd.status().map_err(|e| format!("Failed to run hook command: {}", e))
}

fn format_uuid_with_hyphens(uuid: &str) -> String {
    let clean = uuid.replace('-', "");
    if clean.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &clean[0..8],
            &clean[8..12],
            &clean[12..16],
            &clean[16..20],
            &clean[20..32]
        )
    } else {
        uuid.to_string()
    }
}

fn extract_xuid_from_token(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() > 1 {
        let payload_b64 = parts[1];
        // Decode base64url manually
        let mut s = payload_b64.replace('-', "+").replace('_', "/");
        while s.len() % 4 != 0 {
            s.push('=');
        }
        
        let mut table = [0u8; 256];
        for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
            table[c as usize] = i as u8;
        }
        
        let bytes = s.as_bytes();
        let len = bytes.len();
        if len % 4 == 0 {
            let mut out = Vec::new();
            let mut i = 0;
            while i < len {
                let b0 = table[bytes[i] as usize] as u32;
                let b1 = table[bytes[i+1] as usize] as u32;
                let b2 = if bytes[i+2] == b'=' { 0 } else { table[bytes[i+2] as usize] as u32 };
                let b3 = if bytes[i+3] == b'=' { 0 } else { table[bytes[i+3] as usize] as u32 };
                
                let triple = (b0 << 18) | (b1 << 12) | (b2 << 6) | b3;
                
                out.push(((triple >> 16) & 0xff) as u8);
                if bytes[i+2] != b'=' {
                    out.push(((triple >> 8) & 0xff) as u8);
                }
                if bytes[i+3] != b'=' {
                    out.push((triple & 0xff) as u8);
                }
                i += 4;
            }
            if let Ok(json_str) = String::from_utf8(out) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let Some(xuid) = val.get("xuid") {
                        return xuid.as_str().map(|s| s.to_string());
                    }
                }
            }
        }
    }
    None
}
