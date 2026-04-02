use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

// ─── Mrpack Index Types ──────────────────────────────────────

#[derive(Deserialize)]
pub struct MrpackIndex {
    pub name: String,
    pub files: Vec<MrpackFile>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct MrpackFile {
    pub path: String,
    #[serde(default)]
    pub env: Option<MrpackEnv>,
    pub downloads: Vec<String>,
}

#[derive(Deserialize)]
pub struct MrpackEnv {
    pub server: Option<String>,
}

impl MrpackFile {
    /// Returns true if this file should be installed on a server
    pub fn is_server_side(&self) -> bool {
        match &self.env {
            Some(env) => env.server.as_deref() != Some("unsupported"),
            None => true, // No env means required on both sides
        }
    }
}

impl MrpackIndex {
    /// Get the Minecraft version from dependencies
    pub fn minecraft_version(&self) -> Option<&str> {
        self.dependencies.get("minecraft").map(|s| s.as_str())
    }

    /// Get server-side files only
    pub fn server_files(&self) -> Vec<&MrpackFile> {
        self.files.iter().filter(|f| f.is_server_side()).collect()
    }
}

// ─── Path Validation (CVE-2023-25307 prevention) ─────────────

/// Validate that a file path from the mrpack index is safe.
/// Rejects path traversal, absolute paths, and drive letters.
pub fn validate_path(path: &str) -> bool {
    // Reject empty paths
    if path.is_empty() {
        return false;
    }
    // Reject absolute paths
    if path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    // Reject Windows drive letters (C:, D:, etc.)
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return false;
    }
    // Reject path traversal
    for component in path.split(['/', '\\']) {
        if component == ".." {
            return false;
        }
    }
    true
}

// ─── Installation Progress Tracking ──────────────────────────

#[derive(Clone, Serialize)]
pub struct ModpackProgress {
    pub state: String,        // "preparing", "downloading_mods", "applying_overrides", "installing_loader", "done", "error"
    pub total_files: u32,
    pub downloaded_files: u32,
    pub current_file: String,
    pub error: Option<String>,
}

impl Default for ModpackProgress {
    fn default() -> Self {
        Self {
            state: "preparing".to_string(),
            total_files: 0,
            downloaded_files: 0,
            current_file: String::new(),
            error: None,
        }
    }
}

/// Global progress tracker keyed by server UUID
pub type ProgressMap = Arc<Mutex<HashMap<uuid::Uuid, ModpackProgress>>>;

pub fn create_progress_map() -> ProgressMap {
    Arc::new(Mutex::new(HashMap::new()))
}

// ─── JAR metadata inspection ─────────────────────────────────

/// Check if a jar file is client-only by inspecting its metadata.
/// Reads the jar as a zip and checks:
///   - fabric.mod.json: "environment": "client" (Fabric/Quilt mods)
///   - META-INF/mods.toml: side="CLIENT" or displayTest="IGNORE_ALL_VERSION" (Forge mods)
///   - quilt.mod.json: "environment": "client" (Quilt mods)
pub fn is_client_only_jar(jar_bytes: &[u8]) -> bool {
    use std::io::{Cursor, Read};

    let reader = Cursor::new(jar_bytes);
    let mut archive = match zip::ZipArchive::new(reader) {
        Ok(a) => a,
        Err(_) => return false,
    };

    // Check fabric.mod.json (Fabric mods)
    if let Ok(mut file) = archive.by_name("fabric.mod.json") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                if json["environment"].as_str() == Some("client") {
                    return true;
                }
            }
        }
    }

    // Check quilt.mod.json (Quilt mods)
    if let Ok(mut file) = archive.by_name("quilt.mod.json") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                if json["quilt_loader"]["environment"].as_str() == Some("client") {
                    return true;
                }
            }
        }
    }

    // Check META-INF/mods.toml (Forge mods)
    if let Ok(mut file) = archive.by_name("META-INF/mods.toml") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let lower = contents.to_lowercase();
            // side="CLIENT" means explicitly client-only
            if lower.contains("side=\"client\"") || lower.contains("side = \"client\"") {
                return true;
            }
            // displayTest="IGNORE_ALL_VERSION" generally means no server component
            if lower.contains("displaytest=\"ignore_all_version\"")
                || lower.contains("displaytest = \"ignore_all_version\"")
            {
                // Only flag as client-only if there's also a hint it's client-side
                // (some server mods use IGNORE_ALL_VERSION too)
                if lower.contains("client") && !lower.contains("server") {
                    return true;
                }
            }
        }
    }

    false
}

// ─── Allowed download domains for mrpack files ───────────────

const ALLOWED_MRPACK_DOMAINS: &[&str] = &[
    "cdn.modrinth.com",
    "cdn-raw.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
    "objects.githubusercontent.com",
];

/// Validate that a download URL is from an allowed domain
pub fn validate_download_url(url: &str) -> bool {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host_str() {
            return ALLOWED_MRPACK_DOMAINS.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")));
        }
    }
    false
}

// ─── Protected paths (never overwritten on updates) ──────────

pub const PROTECTED_PATHS: &[&str] = &[
    "world",
    "world_nether",
    "world_the_end",
    "server.properties",
    "whitelist.json",
    "banned-ips.json",
    "banned-players.json",
    "ops.json",
    "eula.txt",
    ".mcvc-type.json",
];

/// Check if a path should be protected from overwrite
pub fn is_protected_path(path: &str) -> bool {
    let normalized = path.trim_start_matches('/');
    PROTECTED_PATHS.iter().any(|p| {
        normalized == *p || normalized.starts_with(&format!("{p}/"))
    })
}
