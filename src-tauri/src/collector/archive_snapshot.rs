use super::cdp::CdpPage;
use crate::sync::SyncSourceV1;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSnapshotOptions {
    pub source: SyncSourceV1,
    pub discover_remote_folders: bool,
    pub requested_remote_folder_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawArchiveItem {
    pub aweme_id: Value,
    pub aweme_type: Option<Value>,
    pub remote_created_at: Option<Value>,
    pub href: String,
    pub source_position: i64,
    pub author_name: Option<Value>,
    pub item_title: Option<Value>,
    pub desc: Option<Value>,
    pub cover_urls: Option<Value>,
    pub like_count: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawRemoteFolder {
    pub remote_folder_id: Value,
    pub name: Value,
    pub source_position: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawArchiveSnapshot {
    pub kind: String,
    pub items: Vec<RawArchiveItem>,
    pub remote_folders: Vec<RawRemoteFolder>,
    pub active_remote_folder_id: Option<Value>,
    pub active_remote_folder_name: Option<Value>,
    pub dom_card_count: i64,
    pub dom_folder_count: i64,
    pub scroll_top: f64,
    pub scroll_height: f64,
    pub client_height: f64,
    pub no_more_visible: bool,
    pub loading_visible: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArchiveWheelTarget {
    pub x: f64,
    pub y: f64,
    pub record_count: usize,
    pub scroll_top: f64,
    pub scroll_height: f64,
    pub client_height: f64,
    pub no_more_visible: bool,
    pub loading_visible: bool,
    pub can_scroll: bool,
    pub list_found: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteFolderPageSnapshot {
    pub kind: String,
    pub folders: Vec<RawRemoteFolder>,
    pub dom_folder_count: i64,
    pub scroll_top: f64,
    pub scroll_height: f64,
    pub client_height: f64,
    pub no_more_visible: bool,
    pub loading_visible: bool,
    pub can_scroll: bool,
    pub anchor_x: f64,
    pub anchor_y: f64,
}

pub async fn read_raw_archive_snapshot(
    page: &CdpPage,
    options: &ArchiveSnapshotOptions,
) -> Result<RawArchiveSnapshot, String> {
    let argument =
        serde_json::to_value(options).map_err(|_| "snapshot_encode_failed".to_string())?;
    let value: Value = page
        .call_function(archive_snapshot_script(), argument)
        .await?;
    if let Some(error) = value.get("error") {
        return Err(error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("archive_snapshot_failed")
            .to_string());
    }
    serde_json::from_value(value).map_err(|_| "archive_snapshot_decode_failed".to_string())
}

pub fn archive_snapshots_stable(first: &RawArchiveSnapshot, second: &RawArchiveSnapshot) -> bool {
    first == second && !first.loading_visible && !second.loading_visible
}

pub fn archive_bottom_confirmed(first: &RawArchiveSnapshot, second: &RawArchiveSnapshot) -> bool {
    archive_snapshots_stable(first, second)
        && first.no_more_visible
        && second.no_more_visible
        && first.scroll_top + first.client_height >= first.scroll_height - 1.0
        && second.scroll_top + second.client_height >= second.scroll_height - 1.0
}

pub async fn read_archive_wheel_target(
    page: &CdpPage,
    source: SyncSourceV1,
) -> Result<ArchiveWheelTarget, String> {
    read_archive_wheel_target_with_deadline(page, source, super::deadline::Deadline::after(Duration::from_secs(60))).await
}

pub async fn read_archive_wheel_target_with_deadline(
    page: &CdpPage,
    source: SyncSourceV1,
    deadline: super::deadline::Deadline,
) -> Result<ArchiveWheelTarget, String> {
    let argument = serde_json::to_value(source).map_err(|_| "snapshot_encode_failed".to_string())?;
    let value: Value = page
        .call_function_with_deadline(
            include_str!("../../collector-scripts/archive-wheel-target.js"),
            argument,
            deadline,
        )
        .await?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    serde_json::from_value(value).map_err(|_| "archive_scroll_target_invalid".to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FavoriteNavigationTarget {
    x: f64,
    y: f64,
}

async fn click_favorite_navigation(page: &CdpPage, view: &str) -> Result<(), String> {
    let value: Value = page
        .call_function(
            include_str!("../../collector-scripts/favorite-navigation-target.js"),
            Value::String(view.to_string()),
        )
        .await?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    let target: FavoriteNavigationTarget = serde_json::from_value(value)
        .map_err(|_| "favorite_navigation_target_invalid".to_string())?;
    page.dispatch_left_click(target.x, target.y).await
}

pub async fn click_favorite_tab(page: &CdpPage) -> Result<(), String> {
    click_favorite_navigation(page, "favorites").await
}

pub async fn click_favorite_folders_tab(page: &CdpPage) -> Result<(), String> {
    click_favorite_navigation(page, "folders").await
}

pub async fn read_remote_folder_page_snapshot(
    page: &CdpPage,
) -> Result<RemoteFolderPageSnapshot, String> {
    read_remote_folder_page_snapshot_with_deadline(
        page,
        super::deadline::Deadline::after(Duration::from_secs(60)),
    )
    .await
}

pub async fn read_remote_folder_page_snapshot_with_deadline(
    page: &CdpPage,
    deadline: super::deadline::Deadline,
) -> Result<RemoteFolderPageSnapshot, String> {
    let value: Value = page
        .call_function_with_deadline(
            include_str!("../../collector-scripts/remote-folder-snapshot.js"),
            Value::Null,
            deadline,
        )
        .await?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    serde_json::from_value(value).map_err(|_| "remote_folder_snapshot_decode_failed".to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemoteFolderClickTarget {
    x: f64,
    y: f64,
}

pub async fn select_remote_folder(page: &CdpPage, requested_id: &str) -> Result<(), String> {
    select_remote_folder_request(
        page,
        Value::String(requested_id.to_string()),
        requested_id,
    )
    .await
}

pub async fn select_remote_folder_with_name(
    page: &CdpPage,
    requested_id: &str,
    requested_name: &str,
) -> Result<(), String> {
    select_remote_folder_request(
        page,
        serde_json::json!({
            "requestedRemoteFolderId": requested_id,
            "requestedRemoteFolderName": requested_name,
        }),
        requested_id,
    )
    .await
}

async fn select_remote_folder_request(
    page: &CdpPage,
    request: Value,
    requested_id: &str,
) -> Result<(), String> {
    let value: Value = page
        .call_function(
            include_str!("../../collector-scripts/select-remote-folder.js"),
            request,
        )
        .await?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    let target: RemoteFolderClickTarget = serde_json::from_value(value)
        .map_err(|_| "remote_folder_click_target_invalid".to_string())?;
    page.dispatch_left_click(target.x, target.y).await?;

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match read_raw_archive_snapshot(
            page,
            &ArchiveSnapshotOptions {
                source: SyncSourceV1::RemoteFolder,
                discover_remote_folders: false,
                requested_remote_folder_id: Some(requested_id.to_string()),
            },
        )
        .await
        {
            Ok(_) => return Ok(()),
            Err(error) if error == "remote_folder_not_selected" && Instant::now() < deadline => {
                page.wait_for_timeout(100).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn archive_snapshot_script() -> &'static str {
    include_str!("../../collector-scripts/react-archive-snapshot.js")
}
