use axum::{extract::Query, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use shared::{GetState, models::user::GetPermissionManager};

use crate::settings::ContentInstallerSettings;

const CF_BASE: &str = "https://api.curseforge.com";
const CF_MINECRAFT_GAME_ID: u32 = 432;

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, String) {
    (status, msg.into())
}

async fn get_api_key(state: &GetState) -> Result<String, (StatusCode, String)> {
    let settings = state
        .settings
        .get()
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, format!("Settings error: {e}")))?;
    let ext = settings
        .find_extension_settings::<ContentInstallerSettings>()
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Extension settings not found"))?;
    if ext.curseforge_api_key.is_empty() {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "CurseForge API key not configured"));
    }
    Ok(ext.curseforge_api_key.to_string())
}

// ---- Search ----

#[derive(Deserialize)]
pub struct SearchParams {
    #[serde(rename = "searchFilter")]
    search_filter: Option<String>,
    #[serde(rename = "classId")]
    class_id: Option<u32>,
    #[serde(rename = "gameVersion")]
    game_version: Option<String>,
    #[serde(rename = "modLoaderType")]
    mod_loader_type: Option<u32>,
    #[serde(rename = "sortField")]
    sort_field: Option<u32>,
    #[serde(rename = "sortOrder")]
    sort_order: Option<String>,
    index: Option<u32>,
    #[serde(rename = "pageSize")]
    page_size: Option<u32>,
}

/// GET /content-installer/curseforge/search
pub async fn search(
    state: GetState,
    _permissions: GetPermissionManager,
    Query(params): Query<SearchParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let api_key = get_api_key(&state).await?;

    let mut url = format!("{CF_BASE}/v1/mods/search?gameId={CF_MINECRAFT_GAME_ID}");
    if let Some(ref q) = params.search_filter {
        url.push_str(&format!("&searchFilter={}", urlencoding::encode(q)));
    }
    if let Some(cid) = params.class_id {
        url.push_str(&format!("&classId={cid}"));
    }
    if let Some(ref gv) = params.game_version {
        url.push_str(&format!("&gameVersion={}", urlencoding::encode(gv)));
    }
    if let Some(mlt) = params.mod_loader_type {
        url.push_str(&format!("&modLoaderType={mlt}"));
    }
    if let Some(sf) = params.sort_field {
        url.push_str(&format!("&sortField={sf}"));
    }
    if let Some(ref so) = params.sort_order {
        url.push_str(&format!("&sortOrder={so}"));
    }
    url.push_str(&format!("&index={}", params.index.unwrap_or(0)));
    url.push_str(&format!("&pageSize={}", params.page_size.unwrap_or(20)));

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("x-api-key", &api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("CurseForge request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Failed to read response: {e}")))?;

    if !status.is_success() {
        return Err(err(
            StatusCode::BAD_GATEWAY,
            format!("CurseForge returned {status}: {body}"),
        ));
    }

    Ok((
        StatusCode::OK,
        [("content-type", "application/json")],
        body,
    ))
}

// ---- Get mod files ----

#[derive(Deserialize)]
pub struct FilesParams {
    #[serde(rename = "modId")]
    mod_id: u32,
    #[serde(rename = "gameVersion")]
    game_version: Option<String>,
    #[serde(rename = "modLoaderType")]
    mod_loader_type: Option<u32>,
    index: Option<u32>,
    #[serde(rename = "pageSize")]
    page_size: Option<u32>,
}

/// GET /content-installer/curseforge/files
pub async fn files(
    state: GetState,
    _permissions: GetPermissionManager,
    Query(params): Query<FilesParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let api_key = get_api_key(&state).await?;

    let mut url = format!("{CF_BASE}/v1/mods/{}/files?", params.mod_id);
    if let Some(ref gv) = params.game_version {
        url.push_str(&format!("gameVersion={}&", urlencoding::encode(gv)));
    }
    if let Some(mlt) = params.mod_loader_type {
        url.push_str(&format!("modLoaderType={mlt}&"));
    }
    url.push_str(&format!("index={}", params.index.unwrap_or(0)));
    url.push_str(&format!("&pageSize={}", params.page_size.unwrap_or(20)));

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("x-api-key", &api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("CurseForge request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Failed to read response: {e}")))?;

    if !status.is_success() {
        return Err(err(
            StatusCode::BAD_GATEWAY,
            format!("CurseForge returned {status}: {body}"),
        ));
    }

    Ok((
        StatusCode::OK,
        [("content-type", "application/json")],
        body,
    ))
}

// ---- Get mod description ----

#[derive(Deserialize)]
pub struct DescriptionParams {
    #[serde(rename = "modId")]
    mod_id: u32,
}

/// GET /content-installer/curseforge/description
pub async fn description(
    state: GetState,
    _permissions: GetPermissionManager,
    Query(params): Query<DescriptionParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let api_key = get_api_key(&state).await?;

    let url = format!("{CF_BASE}/v1/mods/{}/description", params.mod_id);

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("x-api-key", &api_key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("CurseForge request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, format!("Failed to read response: {e}")))?;

    if !status.is_success() {
        return Err(err(
            StatusCode::BAD_GATEWAY,
            format!("CurseForge returned {status}: {body}"),
        ));
    }

    Ok((
        StatusCode::OK,
        [("content-type", "application/json")],
        body,
    ))
}

// ---- Check if configured ----

/// GET /content-installer/curseforge/status
pub async fn status(
    state: GetState,
    _permissions: GetPermissionManager,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let has_key = get_api_key(&state).await.is_ok();
    Ok(axum::Json(serde_json::json!({ "configured": has_key })))
}
