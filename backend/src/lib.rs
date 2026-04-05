mod curseforge;
mod modpack;
mod settings;

use axum::{extract::Query, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use shared::{
    GetState,
    extensions::{Extension, ExtensionRouteBuilder},
    models::{
        server::GetServer,
        user::GetPermissionManager,
    },
    State,
};
use std::sync::Arc;

#[derive(Default)]
pub struct ExtensionStruct;

#[async_trait::async_trait]
impl Extension for ExtensionStruct {
    async fn initialize(&mut self, _state: State) {}

    async fn settings_deserializer(
        &self,
        _state: State,
    ) -> shared::extensions::settings::ExtensionSettingsDeserializer {
        Arc::new(settings::ContentInstallerSettingsDeserializer)
    }

    async fn initialize_router(
        &mut self,
        _state: State,
        builder: ExtensionRouteBuilder,
    ) -> ExtensionRouteBuilder {
        let progress = modpack::create_progress_map();
        let progress_install = progress.clone();
        let progress_status = progress.clone();

        builder
            .add_client_server_api_router(move |router| {
                let pi = progress_install.clone();
                let ps = progress_status.clone();
                router
                    .route(
                        "/content-installer/install",
                        axum::routing::post(install_content),
                    )
                    .route(
                        "/content-installer/install/status",
                        axum::routing::get(install_status),
                    )
                    .route(
                        "/content-installer/remove",
                        axum::routing::post(remove_content),
                    )
                    .route(
                        "/content-installer/curseforge/search",
                        axum::routing::get(curseforge::search),
                    )
                    .route(
                        "/content-installer/curseforge/files",
                        axum::routing::get(curseforge::files),
                    )
                    .route(
                        "/content-installer/curseforge/status",
                        axum::routing::get(curseforge::status),
                    )
                    .route(
                        "/content-installer/curseforge/description",
                        axum::routing::get(curseforge::description),
                    )
                    .route(
                        "/content-installer/modpack/install",
                        axum::routing::post({
                            let pi = pi.clone();
                            move |state, perms, server, query| {
                                modpack_install(state, perms, server, query, pi.clone())
                            }
                        }),
                    )
                    .route(
                        "/content-installer/modpack/cf-install",
                        axum::routing::post({
                            let pi2 = pi.clone();
                            move |state, perms, server, query| {
                                cf_modpack_install(state, perms, server, query, pi2.clone())
                            }
                        }),
                    )
                    .route(
                        "/content-installer/modpack/status",
                        axum::routing::get(move |server| {
                            modpack_status(server, ps.clone())
                        }),
                    )
            })
            .add_admin_api_router(|router| {
                router
                    .route(
                        "/content-installer/settings",
                        axum::routing::get(admin_get_settings),
                    )
                    .route(
                        "/content-installer/settings",
                        axum::routing::put(admin_put_settings),
                    )
            })
    }
}

// ---- Admin settings endpoints ----

async fn admin_get_settings(
    state: GetState,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let settings = state
        .settings
        .get()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;
    let ext = settings
        .find_extension_settings::<settings::ContentInstallerSettings>()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Settings not found"))?;
    // Mask the API key for display
    let masked = if ext.curseforge_api_key.is_empty() {
        String::new()
    } else {
        let key = &ext.curseforge_api_key;
        if key.len() > 8 {
            format!("{}...{}", &key[..4], &key[key.len()-4..])
        } else {
            "*".repeat(key.len())
        }
    };
    Ok(axum::Json(serde_json::json!({
        "curseforge_configured": !ext.curseforge_api_key.is_empty(),
        "curseforge_api_key_masked": masked,
    })))
}

#[derive(Deserialize)]
struct PutSettingsBody {
    curseforge_api_key: Option<String>,
}

async fn admin_put_settings(
    state: GetState,
    axum::Json(body): axum::Json<PutSettingsBody>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut settings = state
        .settings
        .get_mut()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;
    let ext = settings
        .find_mut_extension_settings::<settings::ContentInstallerSettings>()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Settings not found"))?;

    if let Some(key) = body.curseforge_api_key {
        ext.curseforge_api_key = key.into();
    }

    settings
        .save()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Save failed: {e}")))?;

    Ok(axum::Json(serde_json::json!({ "success": true })))
}

const ALLOWED_DOMAINS: &[&str] = &[
    "https://cdn.modrinth.com/",
    "https://cdn-raw.modrinth.com/",
    "https://edge.forgecdn.net/",
    "https://mediafilez.forgecdn.net/",
    "https://media.forgecdn.net/",
];

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, String) {
    (status, msg.into())
}

#[derive(Deserialize)]
struct InstallParams {
    url: String,
    filename: String,
    directory: String,
}

/// POST: Download a plugin/mod file to the server
async fn install_content(
    state: GetState,
    permissions: GetPermissionManager,
    server: GetServer,
    Query(params): Query<InstallParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    permissions
        .has_server_permission("files.create")
        .map_err(|_| err(StatusCode::FORBIDDEN, "Missing files.create permission"))?;

    if !ALLOWED_DOMAINS.iter().any(|d| params.url.starts_with(d)) {
        return Err(err(StatusCode::BAD_REQUEST, "URL domain not allowed"));
    }

    let is_datapacks = params.directory.ends_with("/datapacks");
    if params.directory != "plugins" && params.directory != "mods" && !is_datapacks {
        return Err(err(StatusCode::BAD_REQUEST, "Directory must be 'plugins', 'mods', or '<world>/datapacks'"));
    }
    if is_datapacks && params.directory.contains("..") {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid directory path"));
    }

    let filename = params.filename
        .replace('/', "")
        .replace('\\', "")
        .replace("..", "");
    if filename.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid filename"));
    }

    let node = server
        .node
        .fetch_cached(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let wings = node
        .api_client(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let _ = wings
        .post_servers_server_files_create_directory(
            server.uuid,
            &wings_api::servers_server_files_create_directory::post::RequestBody {
                root: "/".into(),
                name: params.directory.clone().into(),
            },
        )
        .await;

    let _ = wings
        .post_servers_server_files_delete(
            server.uuid,
            &wings_api::servers_server_files_delete::post::RequestBody {
                root: format!("/{}", params.directory).into(),
                files: vec![filename.clone().into()],
            },
        )
        .await;

    let pull_result = wings
        .post_servers_server_files_pull(
            server.uuid,
            &wings_api::servers_server_files_pull::post::RequestBody {
                root: format!("/{}", params.directory).into(),
                url: params.url.into(),
                file_name: Some(filename.into()),
                use_header: false,
                foreground: false,
            },
        )
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Wings pull failed: {e:?}")))?;

    let identifier = match pull_result {
        wings_api::servers_server_files_pull::post::Response::Accepted(r) => Some(r.identifier),
        wings_api::servers_server_files_pull::post::Response::Ok(_) => None,
    };

    Ok(axum::Json(serde_json::json!({
        "success": true,
        "identifier": identifier,
    })))
}

/// GET: Check download progress
async fn install_status(
    state: GetState,
    _permissions: GetPermissionManager,
    server: GetServer,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let node = server
        .node
        .fetch_cached(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let wings = node
        .api_client(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let pulls = wings
        .get_servers_server_files_pull(server.uuid)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:?}")))?;

    if let Some(dl) = pulls.downloads.first() {
        Ok(axum::Json(serde_json::json!({
            "state": "downloading",
            "progress": dl.progress,
            "total": dl.total,
        })))
    } else {
        Ok(axum::Json(serde_json::json!({ "state": "done" })))
    }
}

#[derive(Deserialize)]
struct RemoveParams {
    filename: String,
    directory: String,
}

/// POST: Remove a plugin/mod file
async fn remove_content(
    state: GetState,
    permissions: GetPermissionManager,
    server: GetServer,
    Query(params): Query<RemoveParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    permissions
        .has_server_permission("files.delete")
        .map_err(|_| err(StatusCode::FORBIDDEN, "Missing files.delete permission"))?;

    let is_datapacks = params.directory.ends_with("/datapacks");
    if params.directory != "plugins" && params.directory != "mods" && !is_datapacks {
        return Err(err(StatusCode::BAD_REQUEST, "Directory must be 'plugins', 'mods', or '<world>/datapacks'"));
    }
    if is_datapacks && params.directory.contains("..") {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid directory path"));
    }

    let filename = params.filename
        .replace('/', "")
        .replace('\\', "")
        .replace("..", "");
    if filename.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid filename"));
    }

    let node = server
        .node
        .fetch_cached(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let wings = node
        .api_client(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    wings
        .post_servers_server_files_delete(
            server.uuid,
            &wings_api::servers_server_files_delete::post::RequestBody {
                root: format!("/{}", params.directory).into(),
                files: vec![filename.into()],
            },
        )
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Wings delete failed: {e:?}")))?;

    Ok(axum::Json(serde_json::json!({ "success": true })))
}

// ─── Modpack Installation ────────────────────────────────────

#[derive(Deserialize)]
struct ModpackInstallParams {
    /// URL to the .mrpack file on cdn.modrinth.com
    mrpack_url: String,
    /// Whether to wipe the server first
    #[serde(default)]
    clean_install: bool,
}

/// POST: Install a modpack from an mrpack URL
async fn modpack_install(
    state: GetState,
    permissions: GetPermissionManager,
    server: GetServer,
    Query(params): Query<ModpackInstallParams>,
    progress_map: modpack::ProgressMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    permissions
        .has_server_permission("files.create")
        .map_err(|_| err(StatusCode::FORBIDDEN, "Missing files.create permission"))?;

    // Validate mrpack URL is from Modrinth CDN
    if !params.mrpack_url.starts_with("https://cdn.modrinth.com/") {
        return Err(err(StatusCode::BAD_REQUEST, "mrpack URL must be from cdn.modrinth.com"));
    }

    // Initialize progress
    {
        let mut map = progress_map.lock().await;
        map.insert(server.uuid, modpack::ModpackProgress::default());
    }

    let node = server
        .node
        .fetch_cached(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let wings = node
        .api_client(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let server_uuid = server.uuid;
    let pm = progress_map.clone();

    // Spawn the installation as a background task
    tokio::spawn(async move {
        let result = run_modpack_install(wings, server_uuid, params, pm.clone()).await;
        if let Err(e) = result {
            let mut map = pm.lock().await;
            if let Some(prog) = map.get_mut(&server_uuid) {
                prog.state = "error".to_string();
                prog.error = Some(e);
            }
        }
    });

    Ok(axum::Json(serde_json::json!({ "success": true })))
}

struct LoaderJar {
    url: String,
    is_zip: bool,
}

/// Resolve the loader server jar from the mrpack dependencies.
/// Fabric/Quilt: meta API gives a single ready-to-run jar (direct download).
/// Forge/NeoForge: MCJars provides zip bundles that need decompression.
async fn resolve_loader_jar(dependencies: &std::collections::HashMap<String, String>, mc_version: &str) -> Option<LoaderJar> {
    // Fabric: direct jar from meta API
    if let Some(loader_ver) = dependencies.get("fabric-loader") {
        return Some(LoaderJar {
            url: format!("https://meta.fabricmc.net/v2/versions/loader/{mc_version}/{loader_ver}/1.0.1/server/jar"),
            is_zip: false,
        });
    }

    // Quilt: direct jar from meta API
    if let Some(loader_ver) = dependencies.get("quilt-loader") {
        return Some(LoaderJar {
            url: format!("https://meta.quiltmc.org/v3/versions/loader/{mc_version}/{loader_ver}/0.10.3/server/jar"),
            is_zip: false,
        });
    }

    // NeoForge: query MCJars for the zip URL
    if dependencies.contains_key("neoforge") {
        if let Ok(resp) = reqwest::get(format!("https://versions.mcjars.app/api/v2/builds/NEOFORGE/{mc_version}")).await {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(zip_url) = data["builds"][0]["zipUrl"].as_str() {
                    return Some(LoaderJar { url: zip_url.to_string(), is_zip: true });
                }
            }
        }
    }

    // Forge: query MCJars for the zip URL
    if dependencies.contains_key("forge") {
        if let Ok(resp) = reqwest::get(format!("https://versions.mcjars.app/api/v2/builds/FORGE/{mc_version}")).await {
            if let Ok(data) = resp.json::<serde_json::Value>().await {
                if let Some(zip_url) = data["builds"][0]["zipUrl"].as_str() {
                    return Some(LoaderJar { url: zip_url.to_string(), is_zip: true });
                }
            }
        }
    }

    None
}

/// Background task that runs the full modpack installation
async fn run_modpack_install(
    wings: wings_api::client::WingsClient,
    server_uuid: uuid::Uuid,
    params: ModpackInstallParams,
    progress_map: modpack::ProgressMap,
) -> Result<(), String> {
    let update_progress = |state: &str, downloaded: u32, total: u32, current: &str| {
        let pm = progress_map.clone();
        let state = state.to_string();
        let current = current.to_string();
        async move {
            let mut map = pm.lock().await;
            if let Some(prog) = map.get_mut(&server_uuid) {
                prog.state = state;
                prog.total_files = total;
                prog.downloaded_files = downloaded;
                prog.current_file = current;
            }
        }
    };

    // Step 1: Clean install if requested
    if params.clean_install {
        update_progress("preparing", 0, 0, "Wiping server files...").await;
        let entries = wings
            .get_servers_server_files_list_directory(server_uuid, "/")
            .await
            .map_err(|e| format!("Failed to list files: {e:?}"))?;

        let files: Vec<compact_str::CompactString> = entries.iter().map(|e| e.name.clone()).collect();
        if !files.is_empty() {
            let _ = wings
                .post_servers_server_files_delete(
                    server_uuid,
                    &wings_api::servers_server_files_delete::post::RequestBody {
                        root: "/".into(),
                        files,
                    },
                )
                .await;
        }
    }

    // Step 2: Download mrpack to server
    update_progress("preparing", 0, 0, "Downloading modpack...").await;
    wings
        .post_servers_server_files_pull(
            server_uuid,
            &wings_api::servers_server_files_pull::post::RequestBody {
                root: "/".into(),
                url: params.mrpack_url.into(),
                file_name: Some("_mrpack_install.zip".into()),
                use_header: false,
                foreground: true,
            },
        )
        .await
        .map_err(|e| format!("Failed to download mrpack: {e:?}"))?;

    // Step 3: Decompress mrpack to temp directory
    update_progress("preparing", 0, 0, "Extracting modpack...").await;
    let _ = wings
        .post_servers_server_files_create_directory(
            server_uuid,
            &wings_api::servers_server_files_create_directory::post::RequestBody {
                root: "/".into(),
                name: "_mrpack_temp".into(),
            },
        )
        .await;

    wings
        .post_servers_server_files_decompress(
            server_uuid,
            &wings_api::servers_server_files_decompress::post::RequestBody {
                root: "/_mrpack_temp".into(),
                file: "/_mrpack_install.zip".into(),
                foreground: true,
            },
        )
        .await
        .map_err(|e| format!("Failed to decompress mrpack: {e:?}"))?;

    // Step 4: Read modrinth.index.json from the server
    update_progress("preparing", 0, 0, "Reading modpack index...").await;
    let mut index_data = wings
        .get_servers_server_files_contents(
            server_uuid,
            "/_mrpack_temp/modrinth.index.json",
            false,
            10_000_000, // 10MB max
        )
        .await
        .map_err(|e| format!("Failed to read modrinth.index.json: {e:?}"))?;

    // Read the async stream to a string
    let mut index_bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut index_data, &mut index_bytes)
        .await
        .map_err(|e| format!("Failed to read index data: {e}"))?;

    let index: modpack::MrpackIndex = serde_json::from_slice(&index_bytes)
        .map_err(|e| format!("Failed to parse modrinth.index.json: {e}"))?;

    // Step 5: Apply overrides (copy from temp to server root)
    update_progress("applying_overrides", 0, 0, "Applying config overrides...").await;

    // Move overrides/ contents to root
    let override_entries = wings
        .get_servers_server_files_list_directory(server_uuid, "/_mrpack_temp/overrides")
        .await
        .unwrap_or_default();

    for entry in &override_entries {
        if !modpack::is_protected_path(&entry.name) {
            let _ = wings
                .put_servers_server_files_rename(
                    server_uuid,
                    &wings_api::servers_server_files_rename::put::RequestBody {
                        root: "/".into(),
                        files: vec![wings_api::servers_server_files_rename::put::RequestBodyFiles {
                            from: format!("_mrpack_temp/overrides/{}", entry.name).into(),
                            to: entry.name.clone(),
                        }],
                    },
                )
                .await;
        }
    }

    // Move server-overrides/ contents to root (layered on top)
    let server_override_entries = wings
        .get_servers_server_files_list_directory(server_uuid, "/_mrpack_temp/server-overrides")
        .await
        .unwrap_or_default();

    for entry in &server_override_entries {
        if !modpack::is_protected_path(&entry.name) {
            let _ = wings
                .put_servers_server_files_rename(
                    server_uuid,
                    &wings_api::servers_server_files_rename::put::RequestBody {
                        root: "/".into(),
                        files: vec![wings_api::servers_server_files_rename::put::RequestBodyFiles {
                            from: format!("_mrpack_temp/server-overrides/{}", entry.name).into(),
                            to: entry.name.clone(),
                        }],
                    },
                )
                .await;
        }
    }

    // Step 6: Filter out client-only mods by checking Modrinth project metadata.
    // Modpack authors often don't set the env field in the mrpack, but the individual
    // projects on Modrinth DO have correct server_side fields.
    update_progress("preparing", 0, 0, "Checking mod compatibility...").await;

    let server_files = index.server_files();

    // Extract project IDs from download URLs (format: cdn.modrinth.com/data/{PROJECT_ID}/versions/...)
    let mut file_project_ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut all_project_ids: Vec<String> = Vec::new();
    for file in &server_files {
        for url in &file.downloads {
            if let Some(start) = url.find("cdn.modrinth.com/data/") {
                let rest = &url[start + 22..];
                if let Some(end) = rest.find('/') {
                    let pid = rest[..end].to_string();
                    file_project_ids.insert(file.path.clone(), pid.clone());
                    if !all_project_ids.contains(&pid) {
                        all_project_ids.push(pid);
                    }
                }
            }
        }
    }

    // Fetch the client-only mod exclusion list from GitHub
    let http_client = reqwest::Client::new();
    let exclusion_list = modpack::fetch_exclusion_list(&http_client).await;

    // Batch fetch project metadata to check server_side field (80 per request)
    let mut client_only_projects: std::collections::HashSet<String> = std::collections::HashSet::new();
    for chunk in all_project_ids.chunks(80) {
        let ids_json = serde_json::to_string(chunk).unwrap_or_default();
        if let Ok(resp) = http_client
            .get(format!("https://api.modrinth.com/v2/projects?ids={}", urlencoding::encode(&ids_json)))
            .header("User-Agent", "IR77-ContentInstaller/1.0.0")
            .send()
            .await
        {
            if let Ok(projects) = resp.json::<Vec<serde_json::Value>>().await {
                for p in &projects {
                    let server = p["server_side"].as_str().unwrap_or("unknown");
                    let client = p["client_side"].as_str().unwrap_or("unknown");

                    // Skip if server_side is unsupported, OR if server is unknown but client is required
                    // (a mod that's definitely needed client-side but unknown server-side is almost certainly client-only)
                    let is_client_only = server == "unsupported"
                        || (server == "unknown" && client == "required");

                    if is_client_only {
                        if let Some(id) = p["id"].as_str() {
                            client_only_projects.insert(id.to_string());
                            tracing::info!("Skipping client-only mod: {} ({}) [server={}, client={}]",
                                p["title"].as_str().unwrap_or("?"), id, server, client);
                        }
                    }
                }
            }
        }
    }

    // Filter to server-compatible files only
    let installable_files: Vec<&modpack::MrpackFile> = server_files
        .into_iter()
        .filter(|f| {
            // Layer 1: Modrinth project metadata
            if let Some(pid) = file_project_ids.get(&f.path) {
                if client_only_projects.contains(pid) {
                    return false;
                }
            }
            // Layer 2: Known client-only mod filename patterns
            if modpack::is_known_client_only(&f.path, &exclusion_list) {
                let filename = f.path.rsplit('/').next().unwrap_or(&f.path);
                tracing::info!("Skipping known client-only mod (filename match): {}", filename);
                return false;
            }
            true
        })
        .collect();

    let total = installable_files.len() as u32;
    tracing::info!("Installing {total} mods ({} client-only skipped)", client_only_projects.len());

    // Ensure mods/ directory exists
    let _ = wings
        .post_servers_server_files_create_directory(
            server_uuid,
            &wings_api::servers_server_files_create_directory::post::RequestBody {
                root: "/".into(),
                name: "mods".into(),
            },
        )
        .await;

    for (i, file) in installable_files.iter().enumerate() {
        // Validate path
        if !modpack::validate_path(&file.path) {
            tracing::warn!("Skipping file with invalid path: {}", file.path);
            continue;
        }

        // Get first valid download URL
        let download_url = file.downloads.iter()
            .find(|u| modpack::validate_download_url(u))
            .ok_or_else(|| format!("No valid download URL for {}", file.path))?;

        let filename = file.path.rsplit('/').next().unwrap_or(&file.path);
        let dir = if file.path.contains('/') {
            file.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("/")
        } else {
            "/"
        };

        update_progress("downloading_mods", i as u32, total, filename).await;

        // Ensure parent directory exists
        if dir != "/" && !dir.is_empty() {
            let _ = wings
                .post_servers_server_files_create_directory(
                    server_uuid,
                    &wings_api::servers_server_files_create_directory::post::RequestBody {
                        root: "/".into(),
                        name: dir.into(),
                    },
                )
                .await;
        }

        // Download the file
        let root = if dir == "/" || dir.is_empty() {
            "/".to_string()
        } else {
            format!("/{dir}")
        };

        wings
            .post_servers_server_files_pull(
                server_uuid,
                &wings_api::servers_server_files_pull::post::RequestBody {
                    root: root.into(),
                    url: download_url.clone().into(),
                    file_name: Some(filename.into()),
                    use_header: false,
                    foreground: true,
                },
            )
            .await
            .map_err(|e| format!("Failed to download {}: {e:?}", file.path))?;
    }

    // Step 7: Auto-install loader based on mrpack dependencies
    update_progress("installing_loader", total, total, "Installing server loader...").await;

    let mc_version = index.minecraft_version().unwrap_or("1.21.1");
    let loader_jar = resolve_loader_jar(&index.dependencies, mc_version).await;
    tracing::info!("Loader jar resolved: {:?}", loader_jar.as_ref().map(|j| (&j.url, j.is_zip)));

    if let Some(jar) = &loader_jar {
        if jar.is_zip {
            // Forge/NeoForge: download zip, decompress, clean up
            wings
                .post_servers_server_files_pull(
                    server_uuid,
                    &wings_api::servers_server_files_pull::post::RequestBody {
                        root: "/".into(),
                        url: jar.url.clone().into(),
                        file_name: Some("_loader_install.zip".into()),
                        use_header: false,
                        foreground: true,
                    },
                )
                .await
                .map_err(|e| format!("Failed to download loader: {e:?}"))?;

            let _ = wings
                .post_servers_server_files_decompress(
                    server_uuid,
                    &wings_api::servers_server_files_decompress::post::RequestBody {
                        root: "/".into(),
                        file: "_loader_install.zip".into(),
                        foreground: true,
                    },
                )
                .await;

            let _ = wings
                .post_servers_server_files_delete(
                    server_uuid,
                    &wings_api::servers_server_files_delete::post::RequestBody {
                        root: "/".into(),
                        files: vec!["_loader_install.zip".into()],
                    },
                )
                .await;
        } else {
            // Fabric/Quilt: single jar download
            tracing::info!("Pulling loader jar from: {}", jar.url);
            let pull_res = wings
                .post_servers_server_files_pull(
                    server_uuid,
                    &wings_api::servers_server_files_pull::post::RequestBody {
                        root: "/".into(),
                        url: jar.url.clone().into(),
                        file_name: Some("server.jar".into()),
                        use_header: false,
                        foreground: true,
                    },
                )
                .await;
            tracing::info!("Loader pull result: {:?}", pull_res.is_ok());
            pull_res.map_err(|e| format!("Failed to download loader jar: {e:?}"))?;
        }
    }

    // Step 8: Post-download scan of ALL jars in mods/ for client-only mods.
    // This catches mods that slipped through earlier layers due to bad Modrinth
    // metadata, missing env fields, or mods introduced via overrides.
    // Uses: filename exclusion list, JAR metadata inspection, and Modrinth hash lookup.
    update_progress("installing_loader", total, total, "Scanning for client-only mods...").await;

    let mods_entries = wings
        .get_servers_server_files_list_directory(server_uuid, "/mods")
        .await
        .unwrap_or_default();

    let mut jars_to_remove: Vec<String> = Vec::new();
    for entry in &mods_entries {
        if !entry.name.ends_with(".jar") { continue; }

        // Check filename against hardcoded exclusion list
        if modpack::is_known_client_only(entry.name.as_str(), &exclusion_list) {
            tracing::info!("Removing known client-only mod (filename match): {}", entry.name);
            jars_to_remove.push(entry.name.to_string());
            continue;
        }

        // Read the jar to inspect metadata and hash it
        if let Ok(mut file_data) = wings
            .get_servers_server_files_contents(server_uuid, &format!("/mods/{}", entry.name), true, 50_000_000)
            .await
        {
            let mut bytes = Vec::new();
            if tokio::io::AsyncReadExt::read_to_end(&mut file_data, &mut bytes).await.is_ok() {
                // JAR metadata inspection (fabric.mod.json, mods.toml, quilt.mod.json)
                if modpack::is_client_only_jar(&bytes) {
                    tracing::info!("Removing client-only mod (JAR metadata): {}", entry.name);
                    jars_to_remove.push(entry.name.to_string());
                    continue;
                }

                // Modrinth hash lookup for mods we don't already know about
                let hash = sha1_smol::Sha1::from(&bytes).digest().to_string();
                if let Ok(resp) = http_client
                    .get(format!("https://api.modrinth.com/v2/version_file/{hash}?algorithm=sha1"))
                    .header("User-Agent", "IR77-ContentInstaller/1.0.0")
                    .send()
                    .await
                {
                    if resp.status().is_success() {
                        if let Ok(version_data) = resp.json::<serde_json::Value>().await {
                            let project_id = version_data["project_id"].as_str().unwrap_or("");
                            if client_only_projects.contains(project_id) {
                                tracing::info!("Removing client-only mod (Modrinth hash, known project): {} (project {})", entry.name, project_id);
                                jars_to_remove.push(entry.name.to_string());
                            } else if !project_id.is_empty() && !file_project_ids.values().any(|v| v == project_id) {
                                // Only fetch project metadata if we didn't already check this project
                                if let Ok(proj_resp) = http_client
                                    .get(format!("https://api.modrinth.com/v2/project/{project_id}"))
                                    .header("User-Agent", "IR77-ContentInstaller/1.0.0")
                                    .send()
                                    .await
                                {
                                    if let Ok(proj) = proj_resp.json::<serde_json::Value>().await {
                                        let server = proj["server_side"].as_str().unwrap_or("unknown");
                                        let client = proj["client_side"].as_str().unwrap_or("unknown");
                                        if server == "unsupported" || (server == "unknown" && client == "required") {
                                            tracing::info!("Removing client-only mod (Modrinth project): {} [server={}, client={}]", entry.name, server, client);
                                            jars_to_remove.push(entry.name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Delete client-only mods
    if !jars_to_remove.is_empty() {
        tracing::info!("Removing {} client-only mods from final scan", jars_to_remove.len());
        let _ = wings
            .post_servers_server_files_delete(
                server_uuid,
                &wings_api::servers_server_files_delete::post::RequestBody {
                    root: "/mods".into(),
                    files: jars_to_remove.iter().map(|s| compact_str::CompactString::from(s.as_str())).collect(),
                },
            )
            .await;
    }

    // Step 9: Write eula.txt
    let _ = wings
        .post_servers_server_files_write(
            server_uuid,
            "/eula.txt",
            "eula=true\n".into(),
        )
        .await;

    // Step 10: Write .mcvc-type.json marker
    {
        let loader_type = if index.dependencies.contains_key("fabric-loader") { "FABRIC" }
            else if index.dependencies.contains_key("quilt-loader") { "QUILT" }
            else if index.dependencies.contains_key("neoforge") { "NEOFORGE" }
            else if index.dependencies.contains_key("forge") { "FORGE" }
            else { "UNKNOWN" };
        let marker = serde_json::json!({
            "type": loader_type,
            "version": mc_version,
            "modpack": index.name,
            "installedAt": chrono::Utc::now().to_rfc3339(),
        });
        let _ = wings
            .post_servers_server_files_write(
                server_uuid,
                "/.mcvc-type.json",
                serde_json::to_string(&marker).unwrap_or_default().into(),
            )
            .await;
    }

    // Step 11: Clean up temp files
    update_progress("done", total, total, "Cleaning up...").await;
    let _ = wings
        .post_servers_server_files_delete(
            server_uuid,
            &wings_api::servers_server_files_delete::post::RequestBody {
                root: "/".into(),
                files: vec!["_mrpack_temp".into(), "_mrpack_install.zip".into()],
            },
        )
        .await;

    // Mark as done
    {
        let mut map = progress_map.lock().await;
        if let Some(prog) = map.get_mut(&server_uuid) {
            prog.state = "done".to_string();
            prog.downloaded_files = total;
            prog.current_file = String::new();
        }
    }

    Ok(())
}

// ─── CurseForge Modpack Installation ─────────────────────────

#[derive(Deserialize)]
struct CfModpackInstallParams {
    /// CurseForge CDN URL to the modpack zip
    zip_url: String,
    #[serde(default)]
    clean_install: bool,
}

async fn cf_modpack_install(
    state: GetState,
    permissions: GetPermissionManager,
    server: GetServer,
    Query(params): Query<CfModpackInstallParams>,
    progress_map: modpack::ProgressMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    permissions
        .has_server_permission("files.create")
        .map_err(|_| err(StatusCode::FORBIDDEN, "Missing files.create permission"))?;

    if !params.zip_url.starts_with("https://edge.forgecdn.net/")
        && !params.zip_url.starts_with("https://mediafilez.forgecdn.net/")
        && !params.zip_url.starts_with("https://media.forgecdn.net/")
    {
        return Err(err(StatusCode::BAD_REQUEST, "URL must be from CurseForge CDN"));
    }

    // Get CF API key from settings
    let cf_api_key = {
        let settings_guard = state
            .settings
            .get()
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;
        let ext = settings_guard
            .find_extension_settings::<settings::ContentInstallerSettings>()
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Settings not found"))?;
        if ext.curseforge_api_key.is_empty() {
            return Err(err(StatusCode::SERVICE_UNAVAILABLE, "CurseForge API key not configured"));
        }
        ext.curseforge_api_key.to_string()
    };

    {
        let mut map = progress_map.lock().await;
        map.insert(server.uuid, modpack::ModpackProgress::default());
    }

    let node = server
        .node
        .fetch_cached(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let wings = node
        .api_client(&state.database)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let server_uuid = server.uuid;
    let pm = progress_map.clone();

    tokio::spawn(async move {
        let result = run_cf_modpack_install(wings, server_uuid, params, cf_api_key, pm.clone()).await;
        if let Err(e) = result {
            let mut map = pm.lock().await;
            if let Some(prog) = map.get_mut(&server_uuid) {
                prog.state = "error".to_string();
                prog.error = Some(e);
            }
        }
    });

    Ok(axum::Json(serde_json::json!({ "success": true })))
}

async fn run_cf_modpack_install(
    wings: wings_api::client::WingsClient,
    server_uuid: uuid::Uuid,
    params: CfModpackInstallParams,
    cf_api_key: String,
    progress_map: modpack::ProgressMap,
) -> Result<(), String> {
    let update_progress = |state: &str, downloaded: u32, total: u32, current: &str| {
        let pm = progress_map.clone();
        let state = state.to_string();
        let current = current.to_string();
        async move {
            let mut map = pm.lock().await;
            if let Some(prog) = map.get_mut(&server_uuid) {
                prog.state = state;
                prog.total_files = total;
                prog.downloaded_files = downloaded;
                prog.current_file = current;
            }
        }
    };

    let http_client = reqwest::Client::new();

    // Step 1: Clean install
    if params.clean_install {
        update_progress("preparing", 0, 0, "Wiping server files...").await;
        let entries = wings
            .get_servers_server_files_list_directory(server_uuid, "/")
            .await
            .map_err(|e| format!("Failed to list files: {e:?}"))?;
        let files: Vec<compact_str::CompactString> = entries.iter().map(|e| e.name.clone()).collect();
        if !files.is_empty() {
            let _ = wings
                .post_servers_server_files_delete(
                    server_uuid,
                    &wings_api::servers_server_files_delete::post::RequestBody {
                        root: "/".into(),
                        files,
                    },
                )
                .await;
        }
    }

    // Step 2: Download modpack zip
    update_progress("preparing", 0, 0, "Downloading modpack...").await;
    wings
        .post_servers_server_files_pull(
            server_uuid,
            &wings_api::servers_server_files_pull::post::RequestBody {
                root: "/".into(),
                url: params.zip_url.into(),
                file_name: Some("_cf_modpack.zip".into()),
                use_header: false,
                foreground: true,
            },
        )
        .await
        .map_err(|e| format!("Failed to download modpack: {e:?}"))?;

    // Step 3: Extract
    update_progress("preparing", 0, 0, "Extracting modpack...").await;
    let _ = wings
        .post_servers_server_files_create_directory(
            server_uuid,
            &wings_api::servers_server_files_create_directory::post::RequestBody {
                root: "/".into(),
                name: "_cf_temp".into(),
            },
        )
        .await;

    wings
        .post_servers_server_files_decompress(
            server_uuid,
            &wings_api::servers_server_files_decompress::post::RequestBody {
                root: "/_cf_temp".into(),
                file: "/_cf_modpack.zip".into(),
                foreground: true,
            },
        )
        .await
        .map_err(|e| format!("Failed to decompress modpack: {e:?}"))?;

    // Step 4: Read manifest.json
    update_progress("preparing", 0, 0, "Reading manifest...").await;
    let mut manifest_data = wings
        .get_servers_server_files_contents(
            server_uuid,
            "/_cf_temp/manifest.json",
            false,
            10_000_000,
        )
        .await
        .map_err(|e| format!("Failed to read manifest.json: {e:?}"))?;

    let mut manifest_bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut manifest_data, &mut manifest_bytes)
        .await
        .map_err(|e| format!("Failed to read manifest data: {e}"))?;

    let manifest: modpack::CfManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("Failed to parse manifest.json: {e}"))?;

    // Step 5: Apply overrides
    update_progress("applying_overrides", 0, 0, "Applying config overrides...").await;
    let overrides_path = format!("/_cf_temp/{}", manifest.overrides);
    let override_entries = wings
        .get_servers_server_files_list_directory(server_uuid, &overrides_path)
        .await
        .unwrap_or_default();

    for entry in &override_entries {
        if !modpack::is_protected_path(&entry.name) {
            let _ = wings
                .put_servers_server_files_rename(
                    server_uuid,
                    &wings_api::servers_server_files_rename::put::RequestBody {
                        root: "/".into(),
                        files: vec![wings_api::servers_server_files_rename::put::RequestBodyFiles {
                            from: format!("{}/{}", overrides_path.trim_start_matches('/'), entry.name).into(),
                            to: entry.name.clone(),
                        }],
                    },
                )
                .await;
        }
    }

    // Step 6: Resolve and download mods from CurseForge API
    update_progress("preparing", 0, 0, "Checking mod compatibility...").await;

    let exclusion_list = modpack::fetch_exclusion_list(&http_client).await;

    // Batch resolve files from CF API (we need download URLs)
    // Process in chunks of 50 to avoid API limits
    let required_files: Vec<&modpack::CfManifestFile> = manifest.files.iter().filter(|f| f.required).collect();
    let total = required_files.len() as u32;

    // Ensure mods/ directory exists
    let _ = wings
        .post_servers_server_files_create_directory(
            server_uuid,
            &wings_api::servers_server_files_create_directory::post::RequestBody {
                root: "/".into(),
                name: "mods".into(),
            },
        )
        .await;

    let mut downloaded = 0u32;
    let mut client_only_skipped = 0u32;

    for (i, cf_file) in required_files.iter().enumerate() {
        // Fetch file info from CurseForge API
        let file_url = format!(
            "https://api.curseforge.com/v1/mods/{}/files/{}",
            cf_file.project_id, cf_file.file_id
        );
        let file_resp = http_client
            .get(&file_url)
            .header("x-api-key", &cf_api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch file info for project {}: {e}", cf_file.project_id))?;

        if !file_resp.status().is_success() {
            tracing::warn!("CurseForge API returned {} for project {} file {}", file_resp.status(), cf_file.project_id, cf_file.file_id);
            continue;
        }

        let file_data: serde_json::Value = file_resp.json().await
            .map_err(|e| format!("Failed to parse file info: {e}"))?;

        let file_info = &file_data["data"];
        let filename = file_info["fileName"].as_str().unwrap_or("unknown.jar");
        let download_url = file_info["downloadUrl"].as_str();

        // Check if file is a mod (goes to mods/) based on path heuristic
        let is_mod = filename.ends_with(".jar");

        // Check client-only by filename pattern
        if is_mod && modpack::is_known_client_only(filename, &exclusion_list) {
            tracing::info!("Skipping known client-only mod: {}", filename);
            client_only_skipped += 1;
            continue;
        }

        // Check client-only via CurseForge project metadata
        if is_mod {
            let proj_url = format!("https://api.curseforge.com/v1/mods/{}", cf_file.project_id);
            if let Ok(proj_resp) = http_client
                .get(&proj_url)
                .header("x-api-key", &cf_api_key)
                .header("Accept", "application/json")
                .send()
                .await
            {
                if let Ok(proj_data) = proj_resp.json::<serde_json::Value>().await {
                    // CurseForge classId 6 = mods, check if it's a client-side mod
                    // We can't reliably determine this from CF API alone, so rely on
                    // Modrinth cross-check and filename patterns. The post-install JAR
                    // scan will catch remaining client-only mods.
                }
            }
        }

        update_progress("downloading_mods", downloaded, total, filename).await;

        let Some(url) = download_url else {
            // Mod author disabled third-party distribution
            tracing::warn!("No download URL for {} (project {}), skipping", filename, cf_file.project_id);
            continue;
        };

        // Download the file
        wings
            .post_servers_server_files_pull(
                server_uuid,
                &wings_api::servers_server_files_pull::post::RequestBody {
                    root: "/mods".into(),
                    url: url.into(),
                    file_name: Some(filename.into()),
                    use_header: false,
                    foreground: true,
                },
            )
            .await
            .map_err(|e| format!("Failed to download {}: {e:?}", filename))?;

        downloaded += 1;
    }

    tracing::info!("Downloaded {downloaded} mods, skipped {client_only_skipped} client-only");

    // Step 7: Install loader
    update_progress("installing_loader", downloaded, total, "Installing server loader...").await;

    let mc_version = &manifest.minecraft.version;
    let loader_jar = resolve_loader_jar(
        &{
            let mut deps = std::collections::HashMap::new();
            if let Some(loader_id) = manifest.primary_loader() {
                if let Some(ver) = loader_id.strip_prefix("forge-") {
                    deps.insert("forge".to_string(), ver.to_string());
                } else if let Some(ver) = loader_id.strip_prefix("neoforge-") {
                    deps.insert("neoforge".to_string(), ver.to_string());
                } else if let Some(ver) = loader_id.strip_prefix("fabric-") {
                    deps.insert("fabric-loader".to_string(), ver.to_string());
                } else if let Some(ver) = loader_id.strip_prefix("quilt-") {
                    deps.insert("quilt-loader".to_string(), ver.to_string());
                }
            }
            deps
        },
        mc_version,
    )
    .await;

    if let Some(jar) = &loader_jar {
        if jar.is_zip {
            wings
                .post_servers_server_files_pull(
                    server_uuid,
                    &wings_api::servers_server_files_pull::post::RequestBody {
                        root: "/".into(),
                        url: jar.url.clone().into(),
                        file_name: Some("_loader_install.zip".into()),
                        use_header: false,
                        foreground: true,
                    },
                )
                .await
                .map_err(|e| format!("Failed to download loader: {e:?}"))?;

            let _ = wings
                .post_servers_server_files_decompress(
                    server_uuid,
                    &wings_api::servers_server_files_decompress::post::RequestBody {
                        root: "/".into(),
                        file: "_loader_install.zip".into(),
                        foreground: true,
                    },
                )
                .await;

            let _ = wings
                .post_servers_server_files_delete(
                    server_uuid,
                    &wings_api::servers_server_files_delete::post::RequestBody {
                        root: "/".into(),
                        files: vec!["_loader_install.zip".into()],
                    },
                )
                .await;
        } else {
            wings
                .post_servers_server_files_pull(
                    server_uuid,
                    &wings_api::servers_server_files_pull::post::RequestBody {
                        root: "/".into(),
                        url: jar.url.clone().into(),
                        file_name: Some("server.jar".into()),
                        use_header: false,
                        foreground: true,
                    },
                )
                .await
                .map_err(|e| format!("Failed to download loader jar: {e:?}"))?;
        }
    }

    // Step 8: Post-install JAR scan for client-only mods
    update_progress("installing_loader", downloaded, total, "Scanning for client-only mods...").await;

    let mods_entries = wings
        .get_servers_server_files_list_directory(server_uuid, "/mods")
        .await
        .unwrap_or_default();

    let mut jars_to_remove: Vec<String> = Vec::new();
    for entry in &mods_entries {
        if !entry.name.ends_with(".jar") { continue; }

        if modpack::is_known_client_only(entry.name.as_str(), &exclusion_list) {
            tracing::info!("Removing known client-only mod (post-scan): {}", entry.name);
            jars_to_remove.push(entry.name.to_string());
            continue;
        }

        if let Ok(mut file_data) = wings
            .get_servers_server_files_contents(server_uuid, &format!("/mods/{}", entry.name), true, 50_000_000)
            .await
        {
            let mut bytes = Vec::new();
            if tokio::io::AsyncReadExt::read_to_end(&mut file_data, &mut bytes).await.is_ok() {
                if modpack::is_client_only_jar(&bytes) {
                    tracing::info!("Removing client-only mod (JAR metadata): {}", entry.name);
                    jars_to_remove.push(entry.name.to_string());
                }
            }
        }
    }

    if !jars_to_remove.is_empty() {
        tracing::info!("Removing {} client-only mods from post-scan", jars_to_remove.len());
        let _ = wings
            .post_servers_server_files_delete(
                server_uuid,
                &wings_api::servers_server_files_delete::post::RequestBody {
                    root: "/mods".into(),
                    files: jars_to_remove.iter().map(|s| compact_str::CompactString::from(s.as_str())).collect(),
                },
            )
            .await;
    }

    // Step 9: Write eula.txt
    let _ = wings
        .post_servers_server_files_write(server_uuid, "/eula.txt", "eula=true\n".into())
        .await;

    // Step 10: Write .mcvc-type.json marker
    {
        let marker = serde_json::json!({
            "type": manifest.loader_type(),
            "version": mc_version,
            "modpack": manifest.name,
            "source": "curseforge",
            "installedAt": chrono::Utc::now().to_rfc3339(),
        });
        let _ = wings
            .post_servers_server_files_write(
                server_uuid,
                "/.mcvc-type.json",
                serde_json::to_string(&marker).unwrap_or_default().into(),
            )
            .await;
    }

    // Step 11: Clean up
    update_progress("done", downloaded, total, "Cleaning up...").await;
    let _ = wings
        .post_servers_server_files_delete(
            server_uuid,
            &wings_api::servers_server_files_delete::post::RequestBody {
                root: "/".into(),
                files: vec!["_cf_temp".into(), "_cf_modpack.zip".into()],
            },
        )
        .await;

    {
        let mut map = progress_map.lock().await;
        if let Some(prog) = map.get_mut(&server_uuid) {
            prog.state = "done".to_string();
            prog.downloaded_files = downloaded;
            prog.current_file = String::new();
        }
    }

    Ok(())
}

/// GET: Check modpack installation progress
async fn modpack_status(
    server: GetServer,
    progress_map: modpack::ProgressMap,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let map = progress_map.lock().await;
    if let Some(progress) = map.get(&server.uuid) {
        Ok(axum::Json(serde_json::to_value(progress).unwrap()))
    } else {
        Ok(axum::Json(serde_json::json!({ "state": "idle" })))
    }
}
