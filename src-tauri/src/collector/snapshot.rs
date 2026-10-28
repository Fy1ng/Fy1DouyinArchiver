use super::{cdp::CdpPage, normalization::RawHistoryItem};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawReactSnapshot {
    pub kind: String,
    pub items: Vec<RawHistoryItem>,
    pub dom_card_count: i64,
    pub scroll_top: f64,
    pub scroll_height: f64,
    pub client_height: f64,
    pub no_more_visible: bool,
    pub loading_visible: bool,
}

pub async fn read_raw_snapshot(page: &CdpPage) -> Result<RawReactSnapshot, String> {
    let value: serde_json::Value = page.evaluate(snapshot_script()).await?;
    if let Some(error) = value.get("error") {
        return Err(error.get("code").and_then(serde_json::Value::as_str).unwrap_or("snapshot_failed").to_string());
    }
    serde_json::from_value(value).map_err(|_| "snapshot_decode_failed".to_string())
}

fn snapshot_script() -> &'static str {
    include_str!("../../collector-scripts/react-history-snapshot.js")
}
