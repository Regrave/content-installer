mod modpack;

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

#[derive(Default)]
pub struct ExtensionStruct;

#[async_trait::async_trait]
impl Extension for ExtensionStruct {
    async fn initialize(&mut self, _state: State) {}

    async fn initialize_router(
        &mut self,
        _state: State,
        builder: ExtensionRouteBuilder,
    ) -> ExtensionRouteBuilder {
        let progress = modpack::create_progress_map();
        let progress_install = progress.clone();
        let progress_status = progress.clone();

        builder.add_client_server_api_router(move |router| {
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
                    "/content-installer/modpack/install",
                    axum::routing::post(move |state, perms, server, query| {
                        modpack_install(state, perms, server, query, pi.clone())
                    }),
                )
                .route(
                    "/content-installer/modpack/status",
                    axum::routing::get(move |server| {
                        modpack_status(server, ps.clone())
                    }),
                )
        })
    }
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
    /// MCJars server type for the loader (e.g. "FABRIC", "FORGE", "NEOFORGE")
    #[serde(default)]
    loader_type: Option<String>,
    /// MCJars build URL for the loader jar (from version chooser MCJars API)
    #[serde(default)]
    loader_url: Option<String>,
    /// Filename for the loader jar
    #[serde(default = "default_jar_filename")]
    loader_filename: String,
    /// Whether the loader is a zip install (Forge/NeoForge)
    #[serde(default)]
    loader_unzip: bool,
}

fn default_jar_filename() -> String {
    "server.jar".to_string()
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

    // Step 6: Download mod files
    let server_files = index.server_files();
    let total = server_files.len() as u32;

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

    for (i, file) in server_files.iter().enumerate() {
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

    // Step 8: Write eula.txt
    let _ = wings
        .post_servers_server_files_write(
            server_uuid,
            "/eula.txt",
            "eula=true\n".into(),
        )
        .await;

    // Step 9: Write .mcvc-type.json marker
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

    // Step 10: Clean up temp files
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
