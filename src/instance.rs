use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::collections::{HashMap, HashSet};
use serde::{Serialize, Deserialize};
use crate::downloader::ProgressUpdate;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstanceMod {
    pub filename: String,
    pub enabled: bool,
    pub metadata: ModMetadata,
}

// --- Modrinth mrpack index models ---
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct MrPackFileHashes {
    pub sha1: String,
    pub sha256: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct MrPackFile {
    pub path: String,
    pub hashes: MrPackFileHashes,
    pub downloads: Vec<String>,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct MrPackIndex {
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub name: String,
    #[serde(rename = "versionId")]
    pub version_id: String,
    pub summary: Option<String>,
    pub files: Vec<MrPackFile>,
    pub dependencies: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ModValue {
    Simple(String), // URL only
    Detailed {
        url: String,
        sha1: Option<String>,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct InstanceConfig {
    pub name: String,
    pub version: String,
    pub jvm_args: Option<Vec<String>>,
    pub pre_launch: Option<String>,
    pub post_exit: Option<String>,
    pub mods: Option<HashMap<String, ModValue>>,
    pub java_path: Option<String>,
    pub java_version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub id: String,
    pub path: PathBuf,
    pub config: InstanceConfig,
}

impl Instance {
    pub fn load(id: &str, path: PathBuf) -> Result<Self, String> {
        let config_path = path.join("instance.toml");
        if !config_path.exists() {
            return Err(format!("instance.toml not found in {}", path.display()));
        }
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("Failed to read instance.toml: {}", e))?;
        let config: InstanceConfig = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse instance.toml: {}", e))?;
        
        Ok(Self {
            id: id.to_string(),
            path,
            config,
        })
    }

    pub fn save(&self) -> Result<(), String> {
        let config_path = self.path.join("instance.toml");
        let content = toml::to_string_pretty(&self.config)
            .map_err(|e| format!("Failed to serialize instance config: {}", e))?;
        fs::write(config_path, content)
            .map_err(|e| format!("Failed to write instance.toml: {}", e))?;
        Ok(())
    }

    pub fn load_all(game_dir: &Path) -> Vec<Self> {
        let instances_dir = game_dir.join("instances");
        if !instances_dir.exists() {
            return Vec::new();
        }
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(instances_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let id = entry.file_name().to_string_lossy().to_string();
                    if let Ok(inst) = Self::load(&id, entry.path()) {
                        list.push(inst);
                    }
                }
            }
        }
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    pub fn create(game_dir: &Path, id: &str, name: &str, version: &str) -> Result<Self, String> {
        let instances_dir = game_dir.join("instances");
        let instance_path = instances_dir.join(id);
        if instance_path.exists() {
            return Err(format!("Instance directory '{}' already exists.", id));
        }

        fs::create_dir_all(&instance_path)
            .map_err(|e| format!("Failed to create instance folder: {}", e))?;

        let inst = Self {
            id: id.to_string(),
            path: instance_path,
            config: InstanceConfig {
                name: name.to_string(),
                version: version.to_string(),
                jvm_args: None,
                pre_launch: None,
                post_exit: None,
                mods: None,
                java_path: None,
                java_version: None,
            },
        };

        inst.save()?;
        Ok(inst)
    }

    pub fn delete(&self) -> Result<(), String> {
        if self.path.exists() {
            fs::remove_dir_all(&self.path)
                .map_err(|e| format!("Failed to delete instance files: {}", e))?;
        }
        Ok(())
    }

    pub fn backup(&self) -> Result<PathBuf, String> {
        let backups_dir = self.path.join("backups");
        fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
        let backup_filename = format!("backup_{}.zip", timestamp);
        let backup_path = backups_dir.join(&backup_filename);

        let file = File::create(&backup_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);

        // Backup saves
        let saves_dir = self.path.join("saves");
        if saves_dir.exists() {
            zip.add_directory("saves/", options).map_err(|e| e.to_string())?;
            zip_dir_recursive(&saves_dir, &self.path, &mut zip, options)?;
        }

        // Backup config
        let config_dir = self.path.join("config");
        if config_dir.exists() {
            zip.add_directory("config/", options).map_err(|e| e.to_string())?;
            zip_dir_recursive(&config_dir, &self.path, &mut zip, options)?;
        }

        // Backup options.txt
        let options_file = self.path.join("options.txt");
        if options_file.exists() {
            zip.start_file("options.txt", options).map_err(|e| e.to_string())?;
            let mut f = File::open(&options_file).map_err(|e| e.to_string())?;
            let mut buffer = Vec::new();
            f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
            zip.write_all(&buffer).map_err(|e| e.to_string())?;
        }

        zip.finish().map_err(|e| e.to_string())?;
        Ok(backup_path)
    }

    pub fn restore(&self, backup_name: &str) -> Result<(), String> {
        let backups_dir = self.path.join("backups");
        let backup_path = backups_dir.join(backup_name);
        if !backup_path.exists() {
            return Err(format!("Backup file not found: {}", backup_name));
        }

        // Rename current saves/config/options.txt for rollback safety
        let temp_saves = self.path.join("saves_old_backup");
        let temp_config = self.path.join("config_old_backup");
        let temp_options = self.path.join("options.txt_old_backup");

        let saves_dir = self.path.join("saves");
        let config_dir = self.path.join("config");
        let options_file = self.path.join("options.txt");

        if saves_dir.exists() {
            let _ = fs::rename(&saves_dir, &temp_saves);
        }
        if config_dir.exists() {
            let _ = fs::rename(&config_dir, &temp_config);
        }
        if options_file.exists() {
            let _ = fs::rename(&options_file, &temp_options);
        }

        // Unzip
        match unzip_to_dir(&backup_path, &self.path) {
            Ok(_) => {
                // Success! Delete temporary backups
                if temp_saves.exists() {
                    let _ = fs::remove_dir_all(temp_saves);
                }
                if temp_config.exists() {
                    let _ = fs::remove_dir_all(temp_config);
                }
                if temp_options.exists() {
                    let _ = fs::remove_file(temp_options);
                }
                Ok(())
            }
            Err(e) => {
                // Restore failed! Revert the temporary files
                if temp_saves.exists() {
                    let _ = fs::remove_dir_all(&saves_dir);
                    let _ = fs::rename(temp_saves, &saves_dir);
                }
                if temp_config.exists() {
                    let _ = fs::remove_dir_all(&config_dir);
                    let _ = fs::rename(temp_config, &config_dir);
                }
                if temp_options.exists() {
                    let _ = fs::remove_file(&options_file);
                    let _ = fs::rename(temp_options, &options_file);
                }
                Err(format!("Failed to restore backup: {}", e))
            }
        }
    }

    pub async fn sync_mods(
        &self,
        game_dir: &Path,
        progress_tx: tokio::sync::mpsc::Sender<ProgressUpdate>,
    ) -> Result<(), String> {
        match self.sync_mods_inner(game_dir, &progress_tx).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error(e.clone())).await;
                Err(e)
            }
        }
    }

    async fn sync_mods_inner(
        &self,
        game_dir: &Path,
        progress_tx: &tokio::sync::mpsc::Sender<ProgressUpdate>,
    ) -> Result<(), String> {
        let cache_dir = game_dir.join("cache").join("mods");
        let mods_dir = self.path.join("mods");
        fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

        let mods_map = match &self.config.mods {
            Some(m) => m,
            None => &HashMap::new(),
        };

        let downloader = crate::downloader::Downloader::new(progress_tx.clone());
        let mut expected_filenames = HashSet::new();

        let total = mods_map.len();
        if total > 0 {
            let _ = progress_tx.send(ProgressUpdate::Started {
                total,
                message: format!("Syncing mods for instance '{}'...", self.id),
            }).await;

            for (idx, (name, mod_val)) in mods_map.iter().enumerate() {
                let (url, sha1) = match mod_val {
                    ModValue::Simple(u) => (u.clone(), None),
                    ModValue::Detailed { url: u, sha1: s } => (u.clone(), s.clone()),
                };

                let ext = if url.contains(".jar") { "jar" } else { "jar" };
                let cache_filename = if let Some(ref s) = sha1 {
                    format!("{}.{}", s, ext)
                } else {
                    use sha1::{Sha1, Digest};
                    let mut hasher = Sha1::new();
                    hasher.update(url.as_bytes());
                    format!("{:x}.{}", hasher.finalize(), ext)
                };

                let cache_path = cache_dir.join(&cache_filename);
                let target_filename = if name.ends_with(".jar") { name.clone() } else { format!("{}.jar", name) };
                let target_path = mods_dir.join(&target_filename);

                expected_filenames.insert(target_filename.clone());

                let _ = progress_tx.send(ProgressUpdate::Progress {
                    completed: idx,
                    total,
                    current_file: target_filename.clone(),
                }).await;

                // Download file
                downloader.download_file(&url, &cache_path, sha1.as_deref().unwrap_or("")).await?;

                // Deduplicate: Link or copy
                if target_path.exists() {
                    let _ = fs::remove_file(&target_path);
                }
                if fs::hard_link(&cache_path, &target_path).is_err() {
                    fs::copy(&cache_path, &target_path)
                        .map_err(|e| format!("Failed to copy mod to instance mods: {}", e))?;
                }
            }

            let _ = progress_tx.send(ProgressUpdate::Progress {
                completed: total,
                total,
                current_file: "Sync Complete".to_string(),
            }).await;
        }

        // Cleanup: remove jars that are not declared
        if let Ok(entries) = fs::read_dir(&mods_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type()
                    && file_type.is_file() {
                        let filename = entry.file_name().to_string_lossy().to_string();
                        if filename.ends_with(".jar") && !expected_filenames.contains(&filename) {
                            let _ = fs::remove_file(entry.path());
                        }
                    }
            }
        }

        let _ = progress_tx.send(ProgressUpdate::Finished).await;
        Ok(())
    }

    pub fn list_backups(&self) -> Vec<String> {
        let backups_dir = self.path.join("backups");
        if !backups_dir.exists() {
            return Vec::new();
        }
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(backups_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let filename = entry.file_name().to_string_lossy().to_string();
                    if filename.starts_with("backup_") && filename.ends_with(".zip") {
                        list.push(filename);
                    }
                }
            }
        }
        list.sort();
        list.reverse(); // Newest backups first
        list
    }

    pub fn get_mods(&self) -> Result<Vec<InstanceMod>, String> {
        let mods_dir = self.path.join("mods");
        if !mods_dir.exists() {
            return Ok(Vec::new());
        }
        let mut list = Vec::new();
        if let Ok(entries) = fs::read_dir(mods_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type()
                    && file_type.is_file() {
                        let filename = entry.file_name().to_string_lossy().to_string();
                        let enabled = !filename.ends_with(".disabled");
                        if filename.ends_with(".jar") || filename.ends_with(".jar.disabled") {
                            let metadata = read_mod_metadata(&entry.path()).unwrap_or_else(|_| {
                                let clean_name = filename.strip_suffix(".disabled").unwrap_or(&filename).strip_suffix(".jar").unwrap_or(&filename);
                                ModMetadata {
                                    id: clean_name.to_string(),
                                    name: clean_name.to_string(),
                                    version: "unknown".to_string(),
                                    description: None,
                                }
                            });
                            list.push(InstanceMod {
                                filename,
                                enabled,
                                metadata,
                            });
                        }
                    }
            }
        }
        list.sort_by(|a, b| a.metadata.name.to_lowercase().cmp(&b.metadata.name.to_lowercase()));
        Ok(list)
    }

    pub fn enable_mod(&self, filename: &str) -> Result<(), String> {
        let mods_dir = self.path.join("mods");
        let old_path = mods_dir.join(filename);
        if !old_path.exists() {
            return Err(format!("Mod file '{}' not found.", filename));
        }
        if !filename.ends_with(".disabled") {
            return Ok(()); // Already enabled
        }
        let new_filename = filename.strip_suffix(".disabled").unwrap();
        let new_path = mods_dir.join(new_filename);
        std::fs::rename(&old_path, &new_path)
            .map_err(|e| format!("Failed to enable mod: {}", e))?;
        Ok(())
    }

    pub fn disable_mod(&self, filename: &str) -> Result<(), String> {
        let mods_dir = self.path.join("mods");
        let old_path = mods_dir.join(filename);
        if !old_path.exists() {
            return Err(format!("Mod file '{}' not found.", filename));
        }
        if filename.ends_with(".disabled") {
            return Ok(()); // Already disabled
        }
        let new_filename = format!("{}.disabled", filename);
        let new_path = mods_dir.join(new_filename);
        std::fs::rename(&old_path, &new_path)
            .map_err(|e| format!("Failed to disable mod: {}", e))?;
        Ok(())
    }

    pub async fn install_mod_from_url(
        &mut self,
        game_dir: &Path,
        filename: &str,
        url: &str,
        sha1: Option<&str>,
        save_to_toml: bool,
    ) -> Result<(), String> {
        let cache_dir = game_dir.join("cache").join("mods");
        let mods_dir = self.path.join("mods");
        fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

        let ext = if url.contains(".jar") { "jar" } else { "jar" };
        let cache_filename = if let Some(s) = sha1 {
            format!("{}.{}", s, ext)
        } else {
            use sha1::{Sha1, Digest};
            let mut hasher = Sha1::new();
            hasher.update(url.as_bytes());
            format!("{:x}.{}", hasher.finalize(), ext)
        };

        let cache_path = cache_dir.join(&cache_filename);
        let target_filename = if filename.ends_with(".jar") { filename.to_string() } else { format!("{}.jar", filename) };
        let target_path = mods_dir.join(&target_filename);

        // Download
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        let downloader = crate::downloader::Downloader::new(tx);
        downloader.download_file(url, &cache_path, sha1.unwrap_or("")).await?;

        // Link or copy
        if target_path.exists() {
            let _ = fs::remove_file(&target_path);
        }
        
        // Remove .disabled version if present
        let disabled_target_filename = format!("{}.disabled", target_filename);
        let disabled_target_path = mods_dir.join(&disabled_target_filename);
        if disabled_target_path.exists() {
            let _ = fs::remove_file(&disabled_target_path);
        }

        if fs::hard_link(&cache_path, &target_path).is_err() {
            fs::copy(&cache_path, &target_path)
                .map_err(|e| format!("Failed to copy mod to instance mods: {}", e))?;
        }

        if save_to_toml {
            let mut mods_map = self.config.mods.clone().unwrap_or_default();
            mods_map.insert(target_filename, ModValue::Detailed {
                url: url.to_string(),
                sha1: sha1.map(|s| s.to_string()),
            });
            self.config.mods = Some(mods_map);
            self.save()?;
        }

        Ok(())
    }

    pub fn remove_mod(&mut self, filename_or_id: &str, delete_from_toml: bool) -> Result<(), String> {
        let mods_dir = self.path.join("mods");
        let mods = self.get_mods()?;
        let target_mod = mods.iter().find(|m| {
            m.filename == filename_or_id
                || m.metadata.id == filename_or_id
                || m.metadata.name == filename_or_id
                || m.filename.strip_suffix(".disabled").unwrap_or(&m.filename).strip_suffix(".jar").unwrap_or(&m.filename) == filename_or_id
        });

        if let Some(m) = target_mod {
            let path = mods_dir.join(&m.filename);
            if path.exists() {
                fs::remove_file(&path).map_err(|e| format!("Failed to delete mod file: {}", e))?;
            }
            
            let disabled_filename = if m.filename.ends_with(".disabled") {
                m.filename.clone()
            } else {
                format!("{}.disabled", m.filename)
            };
            let disabled_path = mods_dir.join(&disabled_filename);
            if disabled_path.exists() {
                let _ = fs::remove_file(&disabled_path);
            }

            let enabled_filename = m.filename.strip_suffix(".disabled").unwrap_or(&m.filename).to_string();
            let enabled_path = mods_dir.join(&enabled_filename);
            if enabled_path.exists() {
                let _ = fs::remove_file(&enabled_path);
            }

            if delete_from_toml {
                if let Some(ref mut mods_map) = self.config.mods {
                    mods_map.remove(&enabled_filename);
                    mods_map.remove(&disabled_filename);
                    mods_map.remove(&m.filename);
                    self.save()?;
                }
            }
            Ok(())
        } else {
            Err(format!("Mod '{}' not found in instance.", filename_or_id))
        }
    }

    pub fn export_mrpack(&self, output_path: &Path) -> Result<(), String> {
        let file = File::create(output_path).map_err(|e| format!("Failed to create output file: {}", e))?;
        let mut zip = zip::ZipWriter::new(file);

        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);

        // 1. Dependencies
        let mut dependencies = HashMap::new();
        
        let (mc_ver, loader_name, loader_ver) = if self.config.version.starts_with("fabric-loader-") {
            let parts: Vec<&str> = self.config.version.split('-').collect();
            if parts.len() >= 4 {
                (parts[3].to_string(), Some("fabric-loader".to_string()), Some(parts[2].to_string()))
            } else {
                (self.config.version.clone(), None, None)
            }
        } else if self.config.version.starts_with("forge-") {
            let loader = self.config.version.strip_prefix("forge-").unwrap().to_string();
            (self.config.version.clone(), Some("forge".to_string()), Some(loader))
        } else if self.config.version.starts_with("neoforge-") {
            let loader = self.config.version.strip_prefix("neoforge-").unwrap().to_string();
            (self.config.version.clone(), Some("neoforge".to_string()), Some(loader))
        } else {
            (self.config.version.clone(), None, None)
        };

        dependencies.insert("minecraft".to_string(), mc_ver);
        if let (Some(l_name), Some(l_ver)) = (loader_name, loader_ver) {
            dependencies.insert(l_name, l_ver);
        }

        // 2. Add overrides/mods/
        let mods_dir = self.path.join("mods");
        if mods_dir.exists()
            && let Ok(entries) = fs::read_dir(&mods_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let filename = entry.file_name().to_string_lossy().to_string();
                        if !filename.ends_with(".disabled") {
                            let zip_path = format!("overrides/mods/{}", filename);
                            zip.start_file(zip_path, options).map_err(|e| e.to_string())?;
                            let mut f = File::open(&path).map_err(|e| e.to_string())?;
                            let mut buffer = Vec::new();
                            f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
                            zip.write_all(&buffer).map_err(|e| e.to_string())?;
                        }
                    }
                }
            }

        // 3. Add overrides/config/
        let config_dir = self.path.join("config");
        if config_dir.exists() {
            fn zip_config_recursive(
                current_dir: &Path,
                base_dir: &Path,
                writer: &mut zip::ZipWriter<File>,
                options: zip::write::FileOptions,
            ) -> Result<(), String> {
                for entry in fs::read_dir(current_dir).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    let rel_path = path.strip_prefix(base_dir).map_err(|e| e.to_string())?;
                    let name = format!("overrides/config/{}", rel_path.to_string_lossy());

                    if path.is_dir() {
                        writer.add_directory(&name, options).map_err(|e| e.to_string())?;
                        zip_config_recursive(&path, base_dir, writer, options)?;
                    } else {
                        writer.start_file(&name, options).map_err(|e| e.to_string())?;
                        let mut f = File::open(&path).map_err(|e| e.to_string())?;
                        let mut buffer = Vec::new();
                        f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
                        writer.write_all(&buffer).map_err(|e| e.to_string())?;
                    }
                }
                Ok(())
            }
            let _ = zip.add_directory("overrides/config/", options);
            zip_config_recursive(&config_dir, &config_dir, &mut zip, options)?;
        }

        // 4. Write modrinth.index.json
        zip.start_file("modrinth.index.json", options).map_err(|e| e.to_string())?;
        let index_json = serde_json::json!({
            "formatVersion": 1,
            "game": "minecraft",
            "name": self.config.name,
            "versionId": self.id,
            "files": [],
            "dependencies": dependencies
        });
        let index_str = serde_json::to_string_pretty(&index_json).map_err(|e| e.to_string())?;
        zip.write_all(index_str.as_bytes()).map_err(|e| e.to_string())?;

        zip.finish().map_err(|e| format!("Failed to finalize ZIP: {}", e))?;
        Ok(())
    }

    pub async fn import_mrpack(
        game_dir: &Path,
        pack_path: &Path,
        custom_id: &str,
        progress_tx: &tokio::sync::mpsc::Sender<ProgressUpdate>,
    ) -> Result<Self, String> {
        let file = File::open(pack_path).map_err(|e| e.to_string())?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

        // 1. Read modrinth.index.json
        let index: MrPackIndex = {
            let mut index_file = archive.by_name("modrinth.index.json")
                .map_err(|_| "Invalid .mrpack: missing modrinth.index.json".to_string())?;
            let mut index_content = String::new();
            index_file.read_to_string(&mut index_content).map_err(|e| e.to_string())?;
            serde_json::from_str(&index_content)
                .map_err(|e| format!("Failed to parse modrinth.index.json: {}", e))?
        };

        // Determine mod loader and version
        let mc_version = index.dependencies.get("minecraft")
            .ok_or_else(|| "Missing minecraft dependency in modpack".to_string())?
            .clone();

        // Target version ID for instance
        let version_id = if let Some(fabric_version) = index.dependencies.get("fabric-loader") {
            format!("fabric-loader-{}-{}", fabric_version, mc_version)
        } else if let Some(forge_version) = index.dependencies.get("forge") {
            format!("forge-{}", forge_version)
        } else if let Some(neoforge_version) = index.dependencies.get("neoforge") {
            format!("neoforge-{}", neoforge_version)
        } else {
            mc_version.clone()
        };

        // Create instance folder
        let instances_dir = game_dir.join("instances");
        let instance_path = instances_dir.join(custom_id);
        if instance_path.exists() {
            return Err(format!("Instance directory '{}' already exists", custom_id));
        }
        fs::create_dir_all(&instance_path).map_err(|e| e.to_string())?;

        // 2. Extract overrides/
        let _ = progress_tx.send(ProgressUpdate::Message("Extracting overrides...".to_string())).await;
        let archive_len = archive.len();
        for i in 0..archive_len {
            let mut zip_file = archive.by_index(i).map_err(|e| e.to_string())?;
            let name = zip_file.name().to_string();
            if name.starts_with("overrides/") {
                let rel_path = name.strip_prefix("overrides/").unwrap();
                if rel_path.is_empty() {
                    continue;
                }
                let dest_path = instance_path.join(rel_path);
                if zip_file.is_dir() {
                    fs::create_dir_all(&dest_path).map_err(|e| e.to_string())?;
                } else {
                    if let Some(parent) = dest_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    let mut outfile = File::create(&dest_path).map_err(|e| e.to_string())?;
                    io::copy(&mut zip_file, &mut outfile).map_err(|e| e.to_string())?;
                }
            }
        }

        // 3. Download pack files
        let client = reqwest::Client::new();
        let total_files = index.files.len();
        let _ = progress_tx.send(ProgressUpdate::Started {
            total: total_files,
            message: format!("Downloading {} modpack files...", total_files),
        }).await;

        let mut instance_mods = HashMap::new();

        for (idx, pack_file) in index.files.into_iter().enumerate() {
            let dest_path = instance_path.join(&pack_file.path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }

            let filename = dest_path.file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("download")
                .to_string();

            let mut downloaded = false;
            for url in &pack_file.downloads {
                let _ = progress_tx.send(ProgressUpdate::Progress {
                    completed: idx + 1,
                    total: total_files,
                    current_file: filename.clone(),
                }).await;

                if crate::downloader::Downloader::verify_sha1(&dest_path, &pack_file.hashes.sha1) {
                    downloaded = true;
                    break;
                }

                if let Ok(res) = client.get(url).send().await
                    && res.status().is_success()
                        && let Ok(bytes) = res.bytes().await
                            && fs::write(&dest_path, &bytes).is_ok()
                                && crate::downloader::Downloader::verify_sha1(&dest_path, &pack_file.hashes.sha1) {
                                    downloaded = true;
                                    break;
                                }
            }

            if !downloaded {
                return Err(format!("Failed to download file: {}", pack_file.path));
            }

            if pack_file.path.starts_with("mods/")
                && let Some(first_url) = pack_file.downloads.first() {
                    instance_mods.insert(filename, ModValue::Detailed {
                        url: first_url.clone(),
                        sha1: Some(pack_file.hashes.sha1.clone()),
                    });
                }
        }

        // Create the instance configuration
        let inst = Self {
            id: custom_id.to_string(),
            path: instance_path,
            config: InstanceConfig {
                name: index.name,
                version: version_id,
                jvm_args: None,
                pre_launch: None,
                post_exit: None,
                mods: Some(instance_mods),
                java_path: None,
                java_version: None,
            },
        };

        inst.save()?;
        let _ = progress_tx.send(ProgressUpdate::Finished).await;

        Ok(inst)
    }

    pub fn get_game_version_and_loader(&self, game_dir: &Path) -> (String, Option<String>) {
        let version_id = &self.config.version;
        let mut loader = None;
        if version_id.to_lowercase().contains("fabric") {
            loader = Some("fabric".to_string());
        } else if version_id.to_lowercase().contains("neoforge") {
            loader = Some("neoforge".to_string());
        } else if version_id.to_lowercase().contains("forge") {
            loader = Some("forge".to_string());
        }

        // Try to load version JSON to find inheritsFrom
        let json_path = game_dir
            .join("versions")
            .join(version_id)
            .join(format!("{}.json", version_id));

        if json_path.exists() {
            if let Ok(content) = fs::read_to_string(json_path) {
                if let Ok(details) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(inherits) = details.get("inheritsFrom").and_then(|v| v.as_str()) {
                        return (inherits.to_string(), loader);
                    }
                }
            }
        }

        // Fallback: if fabric-loader-x.y.z-1.a.b, try to get last part
        if version_id.starts_with("fabric-loader-") {
            let parts: Vec<&str> = version_id.split('-').collect();
            if parts.len() >= 4 {
                return (parts[3].to_string(), loader);
            }
        }

        (version_id.clone(), loader)
    }
}

pub fn read_mod_metadata(jar_path: &Path) -> Result<ModMetadata, String> {
    let file = File::open(jar_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    // Try fabric.mod.json
    if let Ok(mut fabric_file) = archive.by_name("fabric.mod.json") {
        let mut content = String::new();
        if fabric_file.read_to_string(&mut content).is_ok()
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                let id = json["id"].as_str().unwrap_or("").to_string();
                let name = json["name"].as_str().map(|s| s.to_string()).unwrap_or_else(|| id.clone());
                let version = json["version"].as_str().unwrap_or("unknown").to_string();
                let description = json["description"].as_str().map(|s| s.to_string());
                if !id.is_empty() {
                    return Ok(ModMetadata { id, name, version, description });
                }
            }
    }

    // Try mods.toml or neoforge.mods.toml under META-INF
    for name in &["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Ok(mut toml_file) = archive.by_name(name) {
            let mut content = String::new();
            if toml_file.read_to_string(&mut content).is_ok()
                && let Ok(toml_val) = toml::from_str::<toml::Value>(&content)
                    && let Some(mods_array) = toml_val.get("mods").and_then(|m| m.as_array())
                        && let Some(first_mod) = mods_array.first() {
                            let id = first_mod.get("modId").and_then(|v| v.as_str())
                                .or_else(|| first_mod.get("id").and_then(|v| v.as_str()))
                                .unwrap_or("")
                                .to_string();
                            let name = first_mod.get("displayName").and_then(|v| v.as_str())
                                .or_else(|| first_mod.get("name").and_then(|v| v.as_str()))
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| id.clone());
                            let version = first_mod.get("version").and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string();
                            let description = first_mod.get("description").and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            if !id.is_empty() {
                                return Ok(ModMetadata { id, name, version, description });
                            }
                        }
        }
    }

    let filename = jar_path.file_name().and_then(|s| s.to_str()).unwrap_or("unknown.jar");
    let clean_name = filename.strip_suffix(".disabled").unwrap_or(filename).strip_suffix(".jar").unwrap_or(filename);
    Ok(ModMetadata {
        id: clean_name.to_string(),
        name: clean_name.to_string(),
        version: "unknown".to_string(),
        description: None,
    })
}

fn zip_dir_recursive(
    current_dir: &Path,
    base_dir: &Path,
    writer: &mut zip::ZipWriter<File>,
    options: zip::write::FileOptions,
) -> Result<(), String> {
    if !current_dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = path.strip_prefix(base_dir)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .to_string();

        if path.is_dir() {
            writer.add_directory(&name, options).map_err(|e| e.to_string())?;
            zip_dir_recursive(&path, base_dir, writer, options)?;
        } else {
            writer.start_file(&name, options).map_err(|e| e.to_string())?;
            let mut f = File::open(&path).map_err(|e| e.to_string())?;
            let mut buffer = Vec::new();
            f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
            writer.write_all(&buffer).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn unzip_to_dir(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => dest_dir.join(path),
            None => continue,
        };

        if (*file.name()).ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = outpath.parent()
                && !p.exists() {
                    fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
            let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
            io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_read_mod_metadata_fabric() {
        let temp_path = std::env::temp_dir().join("minecli_test_fabric_mod.jar");
        {
            let file = File::create(&temp_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("fabric.mod.json", zip::write::FileOptions::default()).unwrap();
            zip.write_all(br#"{
                "id": "testmod",
                "name": "Test Mod",
                "version": "1.0.0",
                "description": "A nice fabric mod"
            }"#).unwrap();
            zip.finish().unwrap();
        }

        let meta = read_mod_metadata(&temp_path).unwrap();
        assert_eq!(meta.id, "testmod");
        assert_eq!(meta.name, "Test Mod");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.description, Some("A nice fabric mod".to_string()));

        let _ = fs::remove_file(temp_path);
    }

    #[test]
    fn test_read_mod_metadata_forge() {
        let temp_path = std::env::temp_dir().join("minecli_test_forge_mod.jar");
        {
            let file = File::create(&temp_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("META-INF/mods.toml", zip::write::FileOptions::default()).unwrap();
            zip.write_all(br#"
[[mods]]
modId = "testforge"
displayName = "Test Forge Mod"
version = "2.3.4"
description = "A forge mod"
"#).unwrap();
            zip.finish().unwrap();
        }

        let meta = read_mod_metadata(&temp_path).unwrap();
        assert_eq!(meta.id, "testforge");
        assert_eq!(meta.name, "Test Forge Mod");
        assert_eq!(meta.version, "2.3.4");
        assert_eq!(meta.description, Some("A forge mod".to_string()));

        let _ = fs::remove_file(temp_path);
    }
}
