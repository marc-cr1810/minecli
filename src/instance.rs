use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::collections::{HashMap, HashSet};
use serde::{Serialize, Deserialize};
use crate::downloader::ProgressUpdate;

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
                if let Err(_) = fs::hard_link(&cache_path, &target_path) {
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
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        let filename = entry.file_name().to_string_lossy().to_string();
                        if filename.ends_with(".jar") && !expected_filenames.contains(&filename) {
                            let _ = fs::remove_file(entry.path());
                        }
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
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
            io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
