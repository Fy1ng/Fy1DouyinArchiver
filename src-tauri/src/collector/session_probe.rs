use super::contracts::AccountIdentityV1;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use url::Url;

const MAX_REMOTE_ID_BYTES: usize = 256;
const MAX_NICKNAME_CHARS: usize = 200;
const MAX_URL_BYTES: usize = 2048;

pub const DOUYIN_HOME_URL: &str = "https://www.douyin.com/";
pub const DOUYIN_HISTORY_URL: &str = "https://www.douyin.com/user/self?showTab=record";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionStateSource {
    SsrRenderData,
    InitialStateUser,
    InitialStateStateUser,
    RouterData,
    NextData,
    RenderData,
    ReactRootProps,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionProbeReason {
    InvalidHomeUrl,
    NoValidSessionSource,
    IdentitySourcesUnlinked,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedHomeIdentityV1 {
    pub page_url: String,
    pub uid: Option<String>,
    pub sec_uid: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub sources: Vec<SessionStateSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionProbeV1 {
    Authenticated(ObservedHomeIdentityV1),
    Unauthenticated {
        page_url: String,
        source: SessionStateSource,
    },
    Indeterminate {
        page_url: String,
        reason: SessionProbeReason,
    },
    Conflicting {
        page_url: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum RawSessionProbeV1 {
    Authenticated {
        page_url: String,
        identity: RawObservedIdentityV1,
    },
    Unauthenticated {
        page_url: String,
        source: SessionStateSource,
    },
    Indeterminate {
        page_url: String,
        reason: SessionProbeReason,
    },
    Conflicting {
        page_url: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawObservedIdentityV1 {
    uid: Option<String>,
    sec_uid: Option<String>,
    nickname: Option<String>,
    avatar_url: Option<String>,
    sources: Vec<SessionStateSource>,
}

pub fn session_probe_from_value(value: &Value) -> Result<SessionProbeV1, String> {
    session_probe_from_value_with_page(value, is_valid_douyin_home_url)
}

pub fn session_probe_from_history_value(value: &Value) -> Result<SessionProbeV1, String> {
    session_probe_from_page_value(value, DOUYIN_HISTORY_URL)
}

pub fn session_probe_from_page_value(
    value: &Value,
    expected_page_url: &str,
) -> Result<SessionProbeV1, String> {
    if expected_page_url.is_empty()
        || expected_page_url != expected_page_url.trim()
        || expected_page_url.len() > MAX_URL_BYTES
    {
        return Err("invalid_session_probe".to_string());
    }
    session_probe_from_value_with_page(value, |page_url| page_url == expected_page_url)
}

fn session_probe_from_value_with_page(
    value: &Value,
    valid_page: impl Fn(&str) -> bool,
) -> Result<SessionProbeV1, String> {
    let raw: RawSessionProbeV1 =
        serde_json::from_value(value.clone()).map_err(|_| "invalid_session_probe".to_string())?;
    match raw {
        RawSessionProbeV1::Authenticated { page_url, identity } => {
            if !valid_page(&page_url) {
                return Err("invalid_session_probe".to_string());
            }
            let identity = ObservedHomeIdentityV1 {
                page_url,
                uid: validated_optional(identity.uid, MAX_REMOTE_ID_BYTES)?,
                sec_uid: validated_optional(identity.sec_uid, MAX_REMOTE_ID_BYTES)?,
                nickname: validated_nickname(identity.nickname)?,
                avatar_url: validated_avatar_url(identity.avatar_url)?,
                sources: validated_sources(identity.sources)?,
            };
            validate_observed_identity_fields(&identity)?;
            Ok(SessionProbeV1::Authenticated(identity))
        }
        RawSessionProbeV1::Unauthenticated { page_url, source } => {
            require_valid_page(&page_url, &valid_page)?;
            Ok(SessionProbeV1::Unauthenticated { page_url, source })
        }
        RawSessionProbeV1::Indeterminate { page_url, reason } => {
            validate_page_url_text(&page_url)?;
            if reason != SessionProbeReason::InvalidHomeUrl {
                require_valid_page(&page_url, &valid_page)?;
            }
            Ok(SessionProbeV1::Indeterminate { page_url, reason })
        }
        RawSessionProbeV1::Conflicting { page_url } => {
            require_valid_page(&page_url, &valid_page)?;
            Ok(SessionProbeV1::Conflicting { page_url })
        }
    }
}

pub fn account_identity_from_home(
    identity: &ObservedHomeIdentityV1,
    account_id: i64,
    login_attempt_id: &str,
    profile_key: &str,
    validated_at: &str,
) -> Result<AccountIdentityV1, String> {
    validate_observed_identity(identity)?;
    let account = AccountIdentityV1 {
        schema_version: 1,
        account_id,
        login_attempt_id: login_attempt_id.to_string(),
        profile_key: profile_key.to_string(),
        uid: identity.uid.clone(),
        sec_uid: identity.sec_uid.clone(),
        nickname: identity.nickname.clone(),
        avatar_url: identity.avatar_url.clone(),
        validated_at: validated_at.to_string(),
        history_url: DOUYIN_HISTORY_URL.to_string(),
    };
    account.validate_for(account_id, login_attempt_id, profile_key)?;
    Ok(account)
}

pub fn is_valid_douyin_home_url(value: &str) -> bool {
    if value != value.trim() || value.len() > MAX_URL_BYTES {
        return false;
    }
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("www.douyin.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(url.path(), "/" | "/jingxuan")
        && url.fragment().is_none()
}

fn validate_observed_identity(identity: &ObservedHomeIdentityV1) -> Result<(), String> {
    require_valid_home_url(&identity.page_url)?;
    validate_observed_identity_fields(identity)
}

fn validate_observed_identity_fields(identity: &ObservedHomeIdentityV1) -> Result<(), String> {
    if identity.uid.is_none() && identity.sec_uid.is_none() {
        return Err("account_identity_unavailable".to_string());
    }
    if identity.sources.is_empty() {
        return Err("invalid_session_probe".to_string());
    }
    Ok(())
}

fn require_valid_page(value: &str, valid_page: &impl Fn(&str) -> bool) -> Result<(), String> {
    if valid_page(value) {
        Ok(())
    } else {
        Err("invalid_session_probe".to_string())
    }
}

fn require_valid_home_url(value: &str) -> Result<(), String> {
    if is_valid_douyin_home_url(value) {
        Ok(())
    } else {
        Err("invalid_session_probe".to_string())
    }
}

fn validate_page_url_text(value: &str) -> Result<(), String> {
    if value.is_empty() || value != value.trim() || value.len() > MAX_URL_BYTES {
        Err("invalid_session_probe".to_string())
    } else {
        Ok(())
    }
}

fn validated_optional(
    value: Option<String>,
    maximum_bytes: usize,
) -> Result<Option<String>, String> {
    match value {
        Some(value)
            if !value.is_empty() && value == value.trim() && value.len() <= maximum_bytes =>
        {
            Ok(Some(value))
        }
        Some(_) => Err("invalid_session_probe".to_string()),
        None => Ok(None),
    }
}

fn validated_nickname(value: Option<String>) -> Result<Option<String>, String> {
    let value = validated_optional(value, MAX_NICKNAME_CHARS * 4)?;
    if value
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_NICKNAME_CHARS)
    {
        return Err("invalid_session_probe".to_string());
    }
    Ok(value)
}

fn validated_avatar_url(value: Option<String>) -> Result<Option<String>, String> {
    let value = validated_optional(value, MAX_URL_BYTES)?;
    if let Some(value) = &value {
        let url = Url::parse(value).map_err(|_| "invalid_session_probe".to_string())?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err("invalid_session_probe".to_string());
        }
    }
    Ok(value)
}

fn validated_sources(sources: Vec<SessionStateSource>) -> Result<Vec<SessionStateSource>, String> {
    let mut unique = HashSet::new();
    if sources.is_empty() || sources.iter().any(|source| !unique.insert(*source)) {
        return Err("invalid_session_probe".to_string());
    }
    Ok(sources)
}
