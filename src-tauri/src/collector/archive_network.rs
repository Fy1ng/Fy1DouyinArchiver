use super::{
    archive_snapshot::{RawRemoteFolder},
    cdp::NetworkResponse,
};
use serde_json::Value;

const MAX_ARCHIVE_NETWORK_BODY: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFolderNetworkPage {
    pub folders: Vec<RawRemoteFolder>,
    pub has_more: bool,
}

pub fn is_remote_folder_list_response(response: &NetworkResponse) -> bool {
    let mime_type = response
        .mime_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if response.status != 200
        || !(mime_type.eq_ignore_ascii_case("application/json") || mime_type.ends_with("+json"))
    {
        return false;
    }
    url::Url::parse(&response.url).ok().is_some_and(|url| {
        matches!(url.host_str(), Some("www.douyin.com" | "douyin.com"))
            && url.path().trim_end_matches('/') == "/aweme/v1/web/collects/list"
    })
}

pub fn parse_remote_folder_list_response(
    body: &[u8],
) -> Result<RemoteFolderNetworkPage, String> {
    if body.len() > MAX_ARCHIVE_NETWORK_BODY {
        return Err("archive_network_response_too_large".to_string());
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| "archive_network_response_invalid".to_string())?;
    let status_code = value
        .get("status_code")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if status_code != 0 {
        return Err("archive_network_api_error".to_string());
    }
    let entries = value
        .get("collects_list")
        .or_else(|| value.get("collectsList"))
        .and_then(Value::as_array)
        .ok_or_else(|| "archive_network_response_shape_changed".to_string())?;
    let mut folders = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let id = string_field(
            entry,
            &[
                "collects_id_str",
                "collectsIdStr",
                "collects_id",
                "collectsId",
            ],
        )
        .ok_or_else(|| "archive_network_folder_id_missing".to_string())?;
        let name = string_field(
            entry,
            &["collects_name", "collectsName", "name", "title"],
        )
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "archive_network_folder_name_missing".to_string())?;
        folders.push(RawRemoteFolder {
            remote_folder_id: Value::String(id),
            name: Value::String(name),
            source_position: index as i64,
        });
    }
    let has_more = value
        .get("has_more")
        .or_else(|| value.get("hasMore"))
        .and_then(boolish)
        .ok_or_else(|| "archive_network_response_shape_changed".to_string())?;
    Ok(RemoteFolderNetworkPage { folders, has_more })
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        let Some(candidate) = object.get(*key) else {
            continue;
        };
        if let Some(text) = candidate.as_str().filter(|text| !text.trim().is_empty()) {
            return Some(text.trim().to_string());
        }
        if let Some(number) = candidate.as_u64().filter(|number| *number <= 9_007_199_254_740_991) {
            return Some(number.to_string());
        }
    }
    None
}

fn boolish(value: &Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_i64().map(|number| number != 0))
        .or_else(|| value.as_str().and_then(|text| match text {
            "0" | "false" => Some(false),
            "1" | "true" => Some(true),
            _ => None,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::cdp::NetworkResponse;
    use serde_json::json;

    #[test]
    fn parses_folder_list_with_string_ids_and_has_more() {
        let page = parse_remote_folder_list_response(
            r#"{"status_code":0,"collects_list":[{"collects_id_str":"90071992547409931","collects_name":"\u7a0d\u540e\u89c2\u770b"}],"has_more":1}"#
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(page.folders[0].remote_folder_id, json!("90071992547409931"));
        assert_eq!(page.folders[0].name, json!("稍后观看"));
        assert!(page.has_more);
    }

    #[test]
    fn rejects_unsafe_numeric_folder_ids_and_wrong_endpoint() {
        assert_eq!(
            parse_remote_folder_list_response(
                br#"{"status_code":0,"collects_list":[{"collects_id":90071992547409931,"collects_name":"A"}]}"#,
            )
            .unwrap_err(),
            "archive_network_folder_id_missing"
        );
        let response = NetworkResponse {
            request_id: "1".to_string(),
            resource_type: "XHR".to_string(),
            url: "https://www.douyin.com/aweme/v1/web/collects/video/list/".to_string(),
            status: 200,
            mime_type: "application/json".to_string(),
        };
        assert!(!is_remote_folder_list_response(&response));
        let mut wrong_host = response;
        wrong_host.url =
            "https://attacker.example/aweme/v1/web/collects/list/".to_string();
        assert!(!is_remote_folder_list_response(&wrong_host));
    }
}
