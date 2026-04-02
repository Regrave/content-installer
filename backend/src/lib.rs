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
        builder.add_client_server_api_router(|router| {
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
    /// "plugins" or "mods"
    directory: String,
}

/// POST: Download a plugin/mod file to the server
async fn install_content(
    state: GetState,
    permissions: GetPermissionManager,
    mut server: GetServer,
    Query(params): Query<InstallParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    permissions
        .has_server_permission("files.create")
        .map_err(|_| err(StatusCode::FORBIDDEN, "Missing files.create permission"))?;

    if !ALLOWED_DOMAINS.iter().any(|d| params.url.starts_with(d)) {
        return Err(err(StatusCode::BAD_REQUEST, "URL domain not allowed"));
    }

    // Validate directory is plugins, mods, or a datapacks path
    let is_datapacks = params.directory.ends_with("/datapacks");
    if params.directory != "plugins" && params.directory != "mods" && !is_datapacks {
        return Err(err(StatusCode::BAD_REQUEST, "Directory must be 'plugins', 'mods', or '<world>/datapacks'"));
    }
    // Prevent path traversal in datapacks path
    if is_datapacks && params.directory.contains("..") {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid directory path"));
    }

    // Sanitize filename
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

    // Ensure target directory exists by creating it (Wings ignores if exists)
    let _ = wings
        .post_servers_server_files_create_directory(
            server.uuid,
            &wings_api::servers_server_files_create_directory::post::RequestBody {
                root: "/".into(),
                name: params.directory.clone().into(),
            },
        )
        .await;

    // Delete existing file with same name if present
    let _ = wings
        .post_servers_server_files_delete(
            server.uuid,
            &wings_api::servers_server_files_delete::post::RequestBody {
                root: format!("/{}", params.directory).into(),
                files: vec![filename.clone().into()],
            },
        )
        .await;

    // Pull the file
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
    mut server: GetServer,
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
    /// "plugins" or "mods"
    directory: String,
}

/// POST: Remove a plugin/mod file
async fn remove_content(
    state: GetState,
    permissions: GetPermissionManager,
    mut server: GetServer,
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
