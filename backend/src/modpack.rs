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

// ─── Remote client-only mod exclusion list ──────────────────
// Fetched from GitHub at runtime so the list can be updated without rebuilding.
// Falls back to a minimal hardcoded list if the fetch fails.

const EXCLUSION_LIST_URL: &str =
    "https://raw.githubusercontent.com/regrave/content-installer/main/client-only-mods.json";

/// Minimal fallback for when the remote list can't be fetched.
const FALLBACK_PATTERNS: &[&str] = &[
    "optifine", "sodium", "iris", "oculus", "rubidium", "embeddium",
    "entityculling", "fpsreducer", "skinlayers3d", "notenoughanimations",
    "ambientsounds", "fancymenu", "drippyloadingscreen", "blur",
    "modmenu", "controlling", "betterf3", "mousetweaks", "freecam",
    "litematica", "minihud", "tweakeroo", "citresewn", "continuity",
    "chatheads", "reauth", "physicsmod", "xaerosminimap", "xaerosworldmap",
    "roughlyenoughitems", "emi", "legendarytooltips", "betterthirdperson",
    "dynamiclights", "ryoamiclights", "immediatelyfast", "reforgium",
];

#[derive(serde::Deserialize)]
struct ExclusionList {
    excludes: Vec<String>,
}

/// Fetch the client-only mod exclusion list from GitHub.
/// Returns the remote list on success, or the hardcoded fallback on failure.
pub async fn fetch_exclusion_list(http_client: &reqwest::Client) -> Vec<String> {
    match http_client
        .get(EXCLUSION_LIST_URL)
        .header("User-Agent", "IR77-ContentInstaller/1.0.0")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<ExclusionList>().await {
                Ok(list) => {
                    tracing::info!("Loaded {} client-only mod patterns from remote list", list.excludes.len());
                    list.excludes
                }
                Err(e) => {
                    tracing::warn!("Failed to parse remote exclusion list: {e}, using fallback");
                    FALLBACK_PATTERNS.iter().map(|s| s.to_string()).collect()
                }
            }
        }
        Ok(resp) => {
            tracing::warn!("Remote exclusion list returned {}, using fallback", resp.status());
            FALLBACK_PATTERNS.iter().map(|s| s.to_string()).collect()
        }
        Err(e) => {
            tracing::warn!("Failed to fetch remote exclusion list: {e}, using fallback");
            FALLBACK_PATTERNS.iter().map(|s| s.to_string()).collect()
        }
    }
}

/// Check if a filename matches the client-only exclusion list.
pub fn is_known_client_only(filename: &str, exclusion_list: &[String]) -> bool {
    let name_lower = filename
        .rsplit('/')
        .next()
        .unwrap_or(filename)
        .trim_end_matches(".jar")
        .trim_end_matches(".zip")
        .to_lowercase();
    exclusion_list.iter().any(|pattern| {
        name_lower.starts_with(pattern.as_str())
            || name_lower.contains(&format!("-{pattern}"))
            || name_lower.contains(&format!("_{pattern}"))
    })
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

    // Check META-INF/mods.toml (Forge/NeoForge mods)
    if let Ok(mut file) = archive.by_name("META-INF/mods.toml") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let lower = contents.to_lowercase();

            // side="CLIENT" on any dependency means explicitly client-only
            if lower.contains("side=\"client\"") || lower.contains("side = \"client\"") {
                // Make sure there's no side="BOTH" or side="SERVER" that would indicate mixed
                let has_both = lower.contains("side=\"both\"") || lower.contains("side = \"both\"");
                let has_server = lower.contains("side=\"server\"") || lower.contains("side = \"server\"");
                if !has_both && !has_server {
                    return true;
                }
            }

            // displayTest="IGNORE_ALL_VERSION" is the strongest client-only signal
            // for Forge 1.20.1 (before the explicit clientSideOnly field in 1.20.4).
            // This tells the server "don't check if I'm installed on the client" which
            // almost always means the mod is client-only or client-optional.
            if lower.contains("displaytest=\"ignore_all_version\"")
                || lower.contains("displaytest = \"ignore_all_version\"")
                || lower.contains("displaytest=\"ignore_server_only\"")
                || lower.contains("displaytest = \"ignore_server_only\"")
            {
                return true;
            }
        }
    }

    // Check META-INF/neoforge.mods.toml (NeoForge mods)
    if let Ok(mut file) = archive.by_name("META-INF/neoforge.mods.toml") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let lower = contents.to_lowercase();
            if lower.contains("side=\"client\"") || lower.contains("side = \"client\"") {
                let has_both = lower.contains("side=\"both\"") || lower.contains("side = \"both\"");
                let has_server = lower.contains("side=\"server\"") || lower.contains("side = \"server\"");
                if !has_both && !has_server {
                    return true;
                }
            }
            if lower.contains("displaytest=\"ignore_all_version\"")
                || lower.contains("displaytest = \"ignore_all_version\"")
                || lower.contains("displaytest=\"ignore_server_only\"")
                || lower.contains("displaytest = \"ignore_server_only\"")
            {
                return true;
            }
        }
    }

    false
}

// ─── CurseForge Manifest Types ──────────────────────────────

#[derive(Deserialize)]
pub struct CfManifest {
    pub name: String,
    pub files: Vec<CfManifestFile>,
    pub minecraft: CfManifestMinecraft,
    #[serde(default = "default_overrides")]
    pub overrides: String,
}

fn default_overrides() -> String {
    "overrides".to_string()
}

#[derive(Deserialize)]
pub struct CfManifestFile {
    #[serde(rename = "projectID")]
    pub project_id: u32,
    #[serde(rename = "fileID")]
    pub file_id: u32,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct CfManifestMinecraft {
    pub version: String,
    #[serde(default, rename = "modLoaders")]
    pub mod_loaders: Vec<CfManifestModLoader>,
}

#[derive(Deserialize)]
pub struct CfManifestModLoader {
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}

impl CfManifest {
    /// Get the primary mod loader ID (e.g. "forge-47.3.0", "fabric-0.16.0", "neoforge-21.1.77")
    pub fn primary_loader(&self) -> Option<&str> {
        self.minecraft
            .mod_loaders
            .iter()
            .find(|l| l.primary)
            .or(self.minecraft.mod_loaders.first())
            .map(|l| l.id.as_str())
    }

    /// Determine loader type from the loader ID string
    pub fn loader_type(&self) -> &str {
        let loader = self.primary_loader().unwrap_or("");
        if loader.starts_with("forge-") {
            "FORGE"
        } else if loader.starts_with("neoforge-") {
            "NEOFORGE"
        } else if loader.starts_with("fabric-") {
            "FABRIC"
        } else if loader.starts_with("quilt-") {
            "QUILT"
        } else {
            "UNKNOWN"
        }
    }
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
