use std::path::{Path, PathBuf};
use std::fs;

#[derive(Debug, Clone)]
pub struct CrashAnalysis {
    pub title: String,
    pub description: String,
    pub possible_solutions: Vec<String>,
}

pub fn analyze_crash(instance_path: &Path, latest_log_content: &str) -> Option<CrashAnalysis> {
    // 1. Try to read the latest crash report
    let crash_report_content = get_latest_crash_report(instance_path)
        .and_then(|path| fs::read_to_string(path).ok());

    // Combine contents for scanning, prioritizing the crash report if present
    let combined_content = if let Some(ref report) = crash_report_content {
        format!("{}\n\n=== LATEST LOG ===\n{}", report, latest_log_content)
    } else {
        latest_log_content.to_string()
    };

    // 2. Perform signature checking
    
    // Signature A: Java version mismatch
    if combined_content.contains("UnsupportedClassVersionError") 
        || combined_content.contains("has been compiled by a more recent version of the Java Runtime") 
    {
        return Some(CrashAnalysis {
            title: "Incompatible Java Version".to_string(),
            description: "The game exited because it was run with a JRE version that is older than what the game or one of your mods requires.".to_string(),
            possible_solutions: vec![
                "Open the instance settings (press 'E' in the TUI or use CLI 'edit') and set a newer Java version (e.g. 17 or 21).".to_string(),
                "If using a custom Java path, ensure it points to a modern Java installation.".to_string(),
            ],
        });
    }

    // Signature B: Out of Memory
    if combined_content.contains("OutOfMemoryError") 
        || combined_content.contains("GC overhead limit exceeded") 
        || combined_content.contains("java.lang.OutOfMemoryError: Java heap space") 
    {
        return Some(CrashAnalysis {
            title: "Out of Memory (Heap Exhausted)".to_string(),
            description: "The Java Virtual Machine ran out of allocated memory (RAM) while running the game.".to_string(),
            possible_solutions: vec![
                "Increase the maximum memory allocation for this instance (e.g., add '-Xmx4G' or '-Xmx6G' to your JVM arguments).".to_string(),
                "Close other memory-intensive applications running on your machine.".to_string(),
            ],
        });
    }

    // Signature C: Graphics Driver OpenGL failure
    if combined_content.contains("GLFW error 65542") 
        || combined_content.contains("WGL: The driver does not appear to support OpenGL") 
        || combined_content.contains("Pixel format not accelerated") 
        || combined_content.contains("org.lwjgl.LWJGLException: Pixel format not accelerated") 
    {
        return Some(CrashAnalysis {
            title: "Graphics Driver / OpenGL Error".to_string(),
            description: "Minecraft was unable to initialize OpenGL. This usually means the system lacks appropriate graphics drivers or hardware acceleration.".to_string(),
            possible_solutions: vec![
                "Update your graphics card drivers (Intel, AMD, or NVIDIA) to the latest version.".to_string(),
                "If you are running in a virtual machine or remote desktop container, ensure GPU acceleration is enabled and passed through.".to_string(),
            ],
        });
    }

    // Signature D: Mod Dependency Loader Error
    // Look for lines containing typical Fabric or Forge loader dependency errors
    for line in combined_content.lines() {
        if line.contains("requires") && (line.contains("version") || line.contains("of mod") || line.contains("any version of")) {
            return Some(CrashAnalysis {
                title: "Missing Mod Dependency".to_string(),
                description: format!("A mod dependency requirement was not satisfied: {}", line.trim()),
                possible_solutions: vec![
                    "Identify the missing mod mentioned in the message and download/add it to your instance.".to_string(),
                    "Ensure all dependencies are matching the version requirements.".to_string(),
                ],
            });
        }
    }

    // Signature E: ClassNotFound / NoSuchMethod mod conflicts
    if combined_content.contains("java.lang.ClassNotFoundException") 
        || combined_content.contains("java.lang.NoSuchMethodError") 
        || combined_content.contains("java.lang.NoClassDefFoundError") 
    {
        // Try to find the offending class/method if possible
        let context = combined_content.lines()
            .find(|l| l.contains("ClassNotFoundException") || l.contains("NoSuchMethodError") || l.contains("NoClassDefFoundError"))
            .unwrap_or("Mod linkage error detected");

        return Some(CrashAnalysis {
            title: "Mod Compatibility Conflict".to_string(),
            description: format!("A mod attempted to load a class or execute a method that does not exist in the current runtime: {}", context.trim()),
            possible_solutions: vec![
                "Check for mod updates to ensure they are fully compatible with your mod loader version and Minecraft version.".to_string(),
                "Remove or replace incompatible/outdated mods from this instance.".to_string(),
            ],
        });
    }

    // General fallback: if it's a generic crash but we have a crash report
    if crash_report_content.is_some() {
        return Some(CrashAnalysis {
            title: "Minecraft Client Crash".to_string(),
            description: "The game crashed and generated a detailed crash report file.".to_string(),
            possible_solutions: vec![
                format!("Inspect the crash report file under the instance's 'crash-reports' folder for more details."),
                "Check the latest log output for warning messages preceding the crash.".to_string(),
            ],
        });
    }

    None
}

fn get_latest_crash_report(instance_path: &Path) -> Option<PathBuf> {
    let reports_dir = instance_path.join("crash-reports");
    if !reports_dir.exists() {
        return None;
    }
    
    let entries = fs::read_dir(reports_dir).ok()?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.path().is_file() && e.path().extension().map(|ext| ext == "txt").unwrap_or(false))
        .map(|e| e.path())
        .collect();

    // Sort by modification time, newest first
    files.sort_by(|a, b| {
        let meta_a = a.metadata().and_then(|m| m.modified()).ok();
        let meta_b = b.metadata().and_then(|m| m.modified()).ok();
        meta_b.cmp(&meta_a)
    });

    files.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsupported_class_version() {
        let log = "java.lang.UnsupportedClassVersionError: net/minecraft/client/main/Main has been compiled by a more recent version of the Java Runtime";
        let path = Path::new("/tmp");
        let analysis = analyze_crash(path, log).unwrap();
        assert_eq!(analysis.title, "Incompatible Java Version");
    }

    #[test]
    fn test_out_of_memory() {
        let log = "java.lang.OutOfMemoryError: Java heap space\n\tat net.minecraft.client.renderer.texture.TextureAtlas.init(TextureAtlas.java:150)";
        let path = Path::new("/tmp");
        let analysis = analyze_crash(path, log).unwrap();
        assert_eq!(analysis.title, "Out of Memory (Heap Exhausted)");
    }
}
