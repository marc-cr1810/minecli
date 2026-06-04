use std::path::{Path, PathBuf};
use std::fs;
use reqwest::Client;
use flate2::read::GzDecoder;
use tar::Archive;
use zip::ZipArchive;

pub async fn install_java_if_needed<P: AsRef<Path>, F>(
    game_dir: P,
    major_version: u32,
    mut log_fn: F,
) -> Result<PathBuf, String> 
where
    F: FnMut(String),
{
    let game_dir = game_dir.as_ref();
    let runtime_dir = game_dir.join("runtime").join(format!("java-{}", major_version));
    
    // Target binary subpath inside the unpacked JRE directory
    let java_exe_subpath = if cfg!(target_os = "windows") {
        Path::new("bin").join("java.exe")
    } else if cfg!(target_os = "macos") {
        Path::new("Contents").join("Home").join("bin").join("java")
    } else {
        Path::new("bin").join("java")
    };

    // If already installed and exists, use it
    if runtime_dir.exists()
        && let Some(exe_path) = find_java_executable(&runtime_dir, &java_exe_subpath) {
            return Ok(exe_path);
        }

    log_fn(format!("Installing Java {} (Adoptium JRE)... This may take a minute.", major_version));
    fs::create_dir_all(&runtime_dir).map_err(|e| format!("Failed to create runtime dir: {}", e))?;

    // Map current platform to Adoptium API parameters
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return Err("Unsupported OS for auto Java installation".to_string());
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x64"
    };

    let url = format!(
        "https://api.adoptium.net/v3/binary/latest/{}/ga/{}/{}/jre/hotspot/normal/eclipse",
        major_version, os, arch
    );

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client.get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to download Java JRE: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Failed to download Java JRE: HTTP status {}", response.status()));
    }

    let bytes = response.bytes().await.map_err(|e| format!("Failed to read Java JRE bytes: {}", e))?;
    
    // Extract archive depending on OS format (zip for Windows, tar.gz for Linux/macOS)
    if cfg!(target_os = "windows") {
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = ZipArchive::new(cursor).map_err(|e| format!("Invalid zip archive: {}", e))?;
        zip.extract(&runtime_dir).map_err(|e| format!("Failed to extract zip archive: {}", e))?;
    } else {
        let cursor = std::io::Cursor::new(bytes);
        let tar = GzDecoder::new(cursor);
        let mut archive = Archive::new(tar);
        archive.unpack(&runtime_dir).map_err(|e| format!("Failed to extract tar.gz: {}", e))?;
    }

    // Locate the executable within the extracted directory structure
    if let Some(exe_path) = find_java_executable(&runtime_dir, &java_exe_subpath) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(&exe_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&exe_path, perms);
            }
        }
        log_fn(format!("Java {} installed successfully to: {}", major_version, exe_path.display()));
        Ok(exe_path)
    } else {
        Err("Failed to find java executable after extraction".to_string())
    }
}

fn find_java_executable(dir: &Path, expected_subpath: &Path) -> Option<PathBuf> {
    let direct = dir.join(expected_subpath);
    if direct.exists() {
        return Some(direct);
    }
    
    // Check inside any wrapper directory (Adoptium JRE extracts to a subfolder)
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let p = entry.path().join(expected_subpath);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}
