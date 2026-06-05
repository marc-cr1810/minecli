use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::io::{Read, Write};
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Local};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WorldInfo {
    pub name: String,
    pub folder_name: String,
    pub last_played: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct AssetInfo {
    pub filename: String,
    pub size_bytes: u64,
    pub enabled: bool,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ScreenshotInfo {
    pub filename: String,
    pub size_bytes: u64,
    pub created: String,
}

// Helper to recursively get directory size
pub fn get_dir_size(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    if path.is_file() {
        return path.metadata().map(|m| m.len()).unwrap_or(0);
    }
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += get_dir_size(&p);
            } else {
                total += p.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

// Helper to format system time to string
fn format_system_time(time: std::time::SystemTime) -> String {
    let dt: DateTime<Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

pub fn list_worlds(instance_path: &Path) -> Result<Vec<WorldInfo>, String> {
    let saves_dir = instance_path.join("saves");
    if !saves_dir.exists() {
        return Ok(Vec::new());
    }

    let mut worlds = Vec::new();
    let entries = fs::read_dir(&saves_dir).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let folder_name = entry.file_name().to_string_lossy().to_string();
            
            // Fallback modification time
            let mut mod_time = path.metadata().and_then(|m| m.modified()).unwrap_or_else(|_| std::time::SystemTime::now());
            
            // Check level.dat for more accurate play date
            let level_dat = path.join("level.dat");
            if level_dat.exists() {
                if let Ok(meta) = level_dat.metadata() {
                    if let Ok(m) = meta.modified() {
                        mod_time = m;
                    }
                }
            }

            let last_played = format_system_time(mod_time);
            let size_bytes = get_dir_size(&path);

            worlds.push(WorldInfo {
                name: folder_name.clone(), // User-friendly name matches directory
                folder_name,
                last_played,
                size_bytes,
            });
        }
    }

    // Sort: newest first
    worlds.sort_by(|a, b| b.last_played.cmp(&a.last_played));
    Ok(worlds)
}

pub fn delete_world(instance_path: &Path, name: &str) -> Result<(), String> {
    let world_dir = instance_path.join("saves").join(name);
    if !world_dir.exists() {
        return Err(format!("World '{}' does not exist.", name));
    }
    fs::remove_dir_all(&world_dir).map_err(|e| format!("Failed to delete world: {}", e))?;
    Ok(())
}

pub fn backup_world(instance_path: &Path, name: &str) -> Result<PathBuf, String> {
    let world_dir = instance_path.join("saves").join(name);
    if !world_dir.exists() {
        return Err(format!("World '{}' does not exist.", name));
    }

    let backups_dir = instance_path.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let backup_filename = format!("world_backup_{}_{}.zip", name, timestamp);
    let backup_path = backups_dir.join(&backup_filename);

    let file = File::create(&backup_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    zip.add_directory(name, options).map_err(|e| e.to_string())?;
    zip_dir_recursive(&world_dir, &world_dir.parent().unwrap(), &mut zip, options)?;
    
    zip.finish().map_err(|e| e.to_string())?;
    Ok(backup_path)
}

fn zip_dir_recursive(
    current_dir: &Path,
    base_dir: &Path,
    writer: &mut zip::ZipWriter<File>,
    options: zip::write::FileOptions,
) -> Result<(), String> {
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

fn get_resource_pack_description(path: &Path) -> Option<String> {
    let contents = if path.is_file() {
        let file = File::open(path).ok()?;
        let mut archive = zip::ZipArchive::new(file).ok()?;
        let mut pack_mcmeta = archive.by_name("pack.mcmeta").ok()?;
        let mut contents = String::new();
        pack_mcmeta.read_to_string(&mut contents).ok()?;
        contents
    } else {
        let meta_path = path.join("pack.mcmeta");
        if meta_path.exists() {
            fs::read_to_string(meta_path).ok()?
        } else {
            return None;
        }
    };

    #[derive(Deserialize)]
    struct PackMcMeta {
        pack: PackInfo,
    }
    #[derive(Deserialize)]
    struct PackInfo {
        description: serde_json::Value,
    }

    let parsed: PackMcMeta = serde_json::from_str(&contents).ok()?;
    match parsed.pack.description {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Object(obj) => {
            if let Some(serde_json::Value::String(text)) = obj.get("text") {
                Some(text.clone())
            } else {
                Some(serde_json::Value::Object(obj).to_string())
            }
        }
        serde_json::Value::Array(arr) => {
            let joined: String = arr.iter().filter_map(|val| {
                if let serde_json::Value::String(s) = val {
                    Some(s.as_str())
                } else if let serde_json::Value::Object(obj) = val {
                    obj.get("text").and_then(|v| v.as_str())
                } else {
                    None
                }
            }).collect();
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

fn list_assets_in_dir(dir_path: &Path) -> Result<Vec<AssetInfo>, String> {
    if !dir_path.exists() {
        return Ok(Vec::new());
    }

    let mut assets = Vec::new();
    let entries = fs::read_dir(dir_path).map_err(|e| e.to_string())?;
    let is_resourcepack = dir_path.file_name().and_then(|n| n.to_str()) == Some("resourcepacks");

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = entry.file_name().to_string_lossy().to_string();
        
        let size_bytes = if path.is_file() {
            path.metadata().map(|m| m.len()).unwrap_or(0)
        } else {
            get_dir_size(&path)
        };

        let enabled = !filename.ends_with(".disabled");
        let description = if is_resourcepack {
            get_resource_pack_description(&path)
        } else {
            None
        };

        assets.push(AssetInfo {
            filename,
            size_bytes,
            enabled,
            description,
        });
    }

    assets.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()));
    Ok(assets)
}

fn toggle_asset_in_dir(dir_path: &Path, filename: &str) -> Result<(), String> {
    let old_path = dir_path.join(filename);
    if !old_path.exists() {
        return Err(format!("Asset file '{}' not found.", filename));
    }

    let new_filename = if filename.ends_with(".disabled") {
        filename.strip_suffix(".disabled").unwrap().to_string()
    } else {
        format!("{}.disabled", filename)
    };

    let new_path = dir_path.join(new_filename);
    fs::rename(&old_path, &new_path).map_err(|e| format!("Failed to rename asset: {}", e))?;
    Ok(())
}

pub fn list_resourcepacks(instance_path: &Path) -> Result<Vec<AssetInfo>, String> {
    list_assets_in_dir(&instance_path.join("resourcepacks"))
}

pub fn toggle_resourcepack(instance_path: &Path, filename: &str) -> Result<(), String> {
    toggle_asset_in_dir(&instance_path.join("resourcepacks"), filename)
}

pub fn list_shaderpacks(instance_path: &Path) -> Result<Vec<AssetInfo>, String> {
    list_assets_in_dir(&instance_path.join("shaderpacks"))
}

pub fn toggle_shaderpack(instance_path: &Path, filename: &str) -> Result<(), String> {
    toggle_asset_in_dir(&instance_path.join("shaderpacks"), filename)
}

pub fn list_screenshots(instance_path: &Path) -> Result<Vec<ScreenshotInfo>, String> {
    let screenshots_dir = instance_path.join("screenshots");
    if !screenshots_dir.exists() {
        return Ok(Vec::new());
    }

    let mut screenshots = Vec::new();
    let entries = fs::read_dir(&screenshots_dir).map_err(|e| e.to_string())?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let filename = entry.file_name().to_string_lossy().to_string();
            let size_bytes = path.metadata().map(|m| m.len()).unwrap_or(0);
            let created_time = path.metadata().and_then(|m| m.created().or_else(|_| m.modified()))
                .unwrap_or_else(|_| std::time::SystemTime::now());
            let created = format_system_time(created_time);

            screenshots.push(ScreenshotInfo {
                filename,
                size_bytes,
                created,
            });
        }
    }

    // Sort: newest first
    screenshots.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(screenshots)
}

pub fn delete_screenshot(instance_path: &Path, filename: &str) -> Result<(), String> {
    let file_path = instance_path.join("screenshots").join(filename);
    if !file_path.exists() {
        return Err(format!("Screenshot '{}' not found.", filename));
    }
    fs::remove_file(&file_path).map_err(|e| format!("Failed to delete screenshot: {}", e))?;
    Ok(())
}

pub fn detect_shader_support(instance_path: &Path) -> bool {
    let mods_dir = instance_path.join("mods");
    if !mods_dir.exists() {
        return false;
    }
    if let Ok(entries) = fs::read_dir(mods_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let filename = entry.file_name().to_string_lossy().to_lowercase();
                if filename.contains("iris") || filename.contains("oculus") || filename.contains("optifine") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_temp_instance() -> (PathBuf, PathBuf) {
        let temp_dir = std::env::temp_dir().join(format!("minecli_test_inst_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();
        (temp_dir.clone(), temp_dir)
    }

    #[test]
    fn test_format_system_time() {
        let now = std::time::SystemTime::now();
        let formatted = format_system_time(now);
        assert!(formatted.contains('-') && formatted.contains(':'));
    }

    #[test]
    fn test_worlds() {
        let (inst_path, _) = setup_temp_instance();
        let saves_dir = inst_path.join("saves");
        
        // No saves initially
        let list = list_worlds(&inst_path).unwrap();
        assert!(list.is_empty());

        // Create a world
        let world1_dir = saves_dir.join("World1");
        fs::create_dir_all(&world1_dir).unwrap();
        
        // Write level.dat
        fs::write(world1_dir.join("level.dat"), b"dummy level data").unwrap();
        fs::write(world1_dir.join("some_region.mca"), b"data").unwrap();

        let list = list_worlds(&inst_path).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].folder_name, "World1");
        assert_eq!(list[0].name, "World1");
        assert!(list[0].size_bytes > 0);

        // Delete world
        delete_world(&inst_path, "World1").unwrap();
        let list = list_worlds(&inst_path).unwrap();
        assert!(list.is_empty());

        let _ = fs::remove_dir_all(&inst_path);
    }

    #[test]
    fn test_backup_world() {
        let (inst_path, _) = setup_temp_instance();
        let saves_dir = inst_path.join("saves");
        let world_dir = saves_dir.join("WorldToBackup");
        fs::create_dir_all(&world_dir).unwrap();
        fs::write(world_dir.join("level.dat"), b"dummy").unwrap();

        let backup_path = backup_world(&inst_path, "WorldToBackup").unwrap();
        assert!(backup_path.exists());
        assert!(backup_path.file_name().unwrap().to_string_lossy().starts_with("world_backup_WorldToBackup_"));

        let _ = fs::remove_dir_all(&inst_path);
    }

    #[test]
    fn test_resourcepacks() {
        let (inst_path, _) = setup_temp_instance();
        let packs_dir = inst_path.join("resourcepacks");
        fs::create_dir_all(&packs_dir).unwrap();

        fs::write(packs_dir.join("pack1.zip"), b"zip data").unwrap();

        let list = list_resourcepacks(&inst_path).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].filename, "pack1.zip");
        assert!(list[0].enabled);

        // Toggle disabled
        toggle_resourcepack(&inst_path, "pack1.zip").unwrap();
        let list = list_resourcepacks(&inst_path).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].filename, "pack1.zip.disabled");
        assert!(!list[0].enabled);

        // Toggle enabled
        toggle_resourcepack(&inst_path, "pack1.zip.disabled").unwrap();
        let list = list_resourcepacks(&inst_path).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].filename, "pack1.zip");
        assert!(list[0].enabled);

        let _ = fs::remove_dir_all(&inst_path);
    }

    #[test]
    fn test_screenshots() {
        let (inst_path, _) = setup_temp_instance();
        let ss_dir = inst_path.join("screenshots");
        fs::create_dir_all(&ss_dir).unwrap();

        fs::write(ss_dir.join("shot.png"), b"png data").unwrap();

        let list = list_screenshots(&inst_path).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].filename, "shot.png");

        delete_screenshot(&inst_path, "shot.png").unwrap();
        let list = list_screenshots(&inst_path).unwrap();
        assert!(list.is_empty());

        let _ = fs::remove_dir_all(&inst_path);
    }

    #[test]
    fn test_shader_detection() {
        let (inst_path, _) = setup_temp_instance();
        assert!(!detect_shader_support(&inst_path));

        let mods_dir = inst_path.join("mods");
        fs::create_dir_all(&mods_dir).unwrap();

        fs::write(mods_dir.join("iris-mc1.20-1.6.4.jar"), b"jar data").unwrap();
        assert!(detect_shader_support(&inst_path));

        let _ = fs::remove_dir_all(&inst_path);
    }
}
