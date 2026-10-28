use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

const MAX_REMOTE_ID_BYTES: usize = 256;
const MAX_NICKNAME_CHARS: usize = 200;
const MAX_AVATAR_URL_BYTES: usize = 2048;

pub use crate::{CollectorRange, CollectorRecordV2, CollectorSuccessV3};

pub const COLLECTOR_SCHEMA_VERSION: i64 = 3;
pub const COLLECTOR_TIME_ZONE: &str = "Asia/Shanghai";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountIdentityV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub login_attempt_id: String,
    pub profile_key: String,
    pub uid: Option<String>,
    pub sec_uid: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub validated_at: String,
    pub history_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAccountIdentity {
    pub uid: Option<String>,
    pub sec_uid: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

impl AccountIdentityV1 {
    pub fn validate_for(
        &self,
        account_id: i64,
        login_attempt_id: &str,
        profile_key: &str,
    ) -> Result<(), String> {
        if self.account_id != account_id
            || self.login_attempt_id != login_attempt_id
            || self.profile_key != profile_key
        {
            return Err("account_identity_mismatch".to_string());
        }
        self.validate()
    }

    pub fn validated_for(
        &self,
        account_id: i64,
        login_attempt_id: &str,
        profile_key: &str,
    ) -> Result<ValidatedAccountIdentity, String> {
        self.validate_for(account_id, login_attempt_id, profile_key)?;
        Ok(ValidatedAccountIdentity {
            uid: self.uid.clone(),
            sec_uid: self.sec_uid.clone(),
            nickname: self.nickname.clone(),
            avatar_url: self.avatar_url.clone(),
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.account_id <= 0
            || !is_simple_uuid(&self.login_attempt_id)
            || !is_simple_uuid(&self.profile_key)
            || chrono::DateTime::parse_from_rfc3339(&self.validated_at).is_err()
        {
            return Err("invalid_account_identity".to_string());
        }
        let uid = validated_optional(&self.uid, MAX_REMOTE_ID_BYTES)?;
        let sec_uid = validated_optional(&self.sec_uid, MAX_REMOTE_ID_BYTES)?;
        if uid.is_none() && sec_uid.is_none() {
            return Err("account_identity_unavailable".to_string());
        }
        let nickname = validated_optional(&self.nickname, MAX_NICKNAME_CHARS * 4)?;
        let avatar_url = validated_optional(&self.avatar_url, MAX_AVATAR_URL_BYTES)?;
        if nickname.is_some_and(|value| value.chars().count() > MAX_NICKNAME_CHARS)
            || !is_douyin_self_history_url(&self.history_url)
        {
            return Err("invalid_account_identity".to_string());
        }
        if let Some(avatar_url) = avatar_url {
            let url = Url::parse(avatar_url)
                .map_err(|_| "invalid_account_identity".to_string())?;
            if url.scheme() != "http" && url.scheme() != "https" {
                return Err("invalid_account_identity".to_string());
            }
        }
        Ok(())
    }
}

pub fn account_identity_from_page_state(
    page: &Value,
    account_id: i64,
    login_attempt_id: &str,
    profile_key: &str,
    validated_at: &str,
) -> Result<AccountIdentityV1, String> {
    let page_url = page
        .get("pageUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "account_identity_unavailable".to_string())?;
    if !is_douyin_self_history_url(page_url) {
        return Err("account_identity_unavailable".to_string());
    }
    if page.get("error").and_then(Value::as_str) == Some("account_identity_ambiguous") {
        return Err("account_identity_ambiguous".to_string());
    }
    let candidates = page
        .pointer("/state/candidates")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| "account_identity_unavailable".to_string())?;
    let uid = consistent_field(candidates, &["uid"])?;
    let sec_uid = consistent_field(candidates, &["secUid", "sec_uid"])?;
    if uid.is_none() && sec_uid.is_none() {
        return Err("account_identity_unavailable".to_string());
    }
    let identity = AccountIdentityV1 {
        schema_version: 1,
        account_id,
        login_attempt_id: login_attempt_id.to_string(),
        profile_key: profile_key.to_string(),
        uid,
        sec_uid,
        nickname: first_string_field(candidates, &["nickname", "nickName"]),
        avatar_url: first_string_field(candidates, &["avatarUrl", "avatar_url"]),
        validated_at: validated_at.to_string(),
        history_url: page_url.to_string(),
    };
    identity.validate_for(account_id, login_attempt_id, profile_key)?;
    Ok(identity)
}

fn string_field(value: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn first_string_field(values: &[Value], names: &[&str]) -> Option<String> {
    values.iter().find_map(|value| string_field(value, names))
}

fn consistent_field(values: &[Value], names: &[&str]) -> Result<Option<String>, String> {
    let mut selected: Option<String> = None;
    for value in values.iter().filter_map(|value| string_field(value, names)) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if selected
            .as_deref()
            .is_some_and(|selected| selected != trimmed)
        {
            return Err("account_identity_ambiguous".to_string());
        }
        selected = Some(trimmed.to_string());
    }
    Ok(selected)
}

fn validated_optional(
    value: &Option<String>,
    maximum_bytes: usize,
) -> Result<Option<&str>, String> {
    match value.as_deref() {
        Some(value)
            if !value.is_empty() && value == value.trim() && value.len() <= maximum_bytes =>
        {
            Ok(Some(value))
        }
        Some(_) => Err("invalid_account_identity".to_string()),
        None => Ok(None),
    }
}

fn is_simple_uuid(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_douyin_self_history_url(value: &str) -> bool {
    if value != value.trim() {
        return false;
    }
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    (url.scheme() == "http" || url.scheme() == "https")
        && url.host_str() == Some("www.douyin.com")
        && url.path() == "/user/self"
        && url
            .query_pairs()
            .any(|(key, value)| key == "showTab" && value == "record")
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectorProgress {
    pub schema_version: i64,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: String,
    pub record_count: i64,
    pub elapsed_ms: i64,
    pub retry_count: i64,
    pub can_cancel: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectorDiagnostic {
    pub schema_version: i64,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub status: String,
    pub phase: String,
    pub error_code: String,
    pub message: String,
    pub record_count: i64,
    pub dom_card_count: i64,
    pub calendar_time_zone: String,
    pub elapsed_ms: i64,
    pub no_progress_ms: i64,
    pub retry_count: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginDiagnostic {
    pub schema_version: i64,
    pub account_id: i64,
    pub login_attempt_id: String,
    pub status: String,
    pub error_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginProgressV1 {
    pub schema_version: i64,
    pub account_id: i64,
    pub login_attempt_id: String,
    pub state: String,
    pub reason: LoginProgressReason,
    pub observed_at_ms: i64,
    pub deadline_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginProgressReason {
    Cancelled,
    BrowserClosed,
    Timeout,
    Failed,
}

impl LoginProgressV1 {
    pub fn validate_for(&self, account_id: i64, login_attempt_id: &str) -> Result<(), String> {
        if self.schema_version != 1
            || self.account_id != account_id
            || self.login_attempt_id != login_attempt_id
            || !is_simple_uuid(&self.login_attempt_id)
            || self.state != "closing"
            || self.observed_at_ms < 0
            || self.deadline_at_ms < self.observed_at_ms
            || self.deadline_at_ms - self.observed_at_ms > 2_000
        {
            return Err("invalid_login_progress".to_string());
        }
        Ok(())
    }
}
