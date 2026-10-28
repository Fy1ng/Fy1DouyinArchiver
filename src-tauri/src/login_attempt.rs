use crate::{collector::file_store, AppPaths};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginAttemptEnvelopeV1<T> {
    pub envelope_version: i64,
    pub login_attempt_id: String,
    pub target_account_id: Option<i64>,
    pub profile_key: String,
    pub payload: T,
}

impl<T> LoginAttemptEnvelopeV1<T> {
    pub fn new(
        login_attempt_id: &str,
        target_account_id: Option<i64>,
        profile_key: &str,
        payload: T,
    ) -> Result<Self, String> {
        let envelope = Self {
            envelope_version: 1,
            login_attempt_id: login_attempt_id.to_string(),
            target_account_id,
            profile_key: profile_key.to_string(),
            payload,
        };
        envelope.validate_context(login_attempt_id, target_account_id, profile_key)?;
        Ok(envelope)
    }

    pub fn validate_context(
        &self,
        login_attempt_id: &str,
        target_account_id: Option<i64>,
        profile_key: &str,
    ) -> Result<(), String> {
        if self.envelope_version != 1
            || !is_simple_uuid(&self.login_attempt_id)
            || self.login_attempt_id != login_attempt_id
            || self.target_account_id != target_account_id
            || self.target_account_id.is_some_and(|value| value <= 0)
            || !is_simple_uuid(&self.profile_key)
            || self.profile_key != profile_key
        {
            return Err("login_attempt_envelope_mismatch".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginProgressPayloadV1 {
    pub state: String,
    pub reason: crate::collector::contracts::LoginProgressReason,
    pub observed_at_ms: i64,
    pub deadline_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginDiagnosticPayloadV1 {
    pub status: String,
    pub error_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginAttemptLifecycle {
    Preparing,
    Running,
    Observed,
    Finalizing,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginAttemptStateV1 {
    pub schema_version: i64,
    pub revision: u64,
    pub login_attempt_id: String,
    pub profile_key: String,
    pub target_account_id: Option<i64>,
    pub lifecycle: LoginAttemptLifecycle,
    pub observed_identity_hash: Option<String>,
    pub marker_validation_id: Option<String>,
    pub updated_at: String,
}

impl LoginAttemptStateV1 {
    pub fn next(&self, lifecycle: LoginAttemptLifecycle, updated_at: &str) -> Result<Self, String> {
        validate_timestamp(updated_at)?;
        Ok(Self {
            revision: self
                .revision
                .checked_add(1)
                .ok_or_else(|| "login_attempt_revision_overflow".to_string())?,
            lifecycle,
            updated_at: updated_at.to_string(),
            ..self.clone()
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || !is_simple_uuid(&self.login_attempt_id)
            || !is_simple_uuid(&self.profile_key)
            || self.target_account_id.is_some_and(|value| value <= 0)
            || self
                .observed_identity_hash
                .as_deref()
                .is_some_and(|value| !is_sha256(value))
            || self
                .marker_validation_id
                .as_deref()
                .is_some_and(|value| !is_simple_uuid(value))
        {
            return Err("invalid_login_attempt_state".to_string());
        }
        validate_timestamp(&self.updated_at)
    }
}

pub fn create_login_attempt_state(
    paths: &AppPaths,
    login_attempt_id: &str,
    profile_key: &str,
    target_account_id: Option<i64>,
    updated_at: &str,
) -> Result<LoginAttemptStateV1, String> {
    let state = LoginAttemptStateV1 {
        schema_version: 1,
        revision: 0,
        login_attempt_id: login_attempt_id.to_string(),
        profile_key: profile_key.to_string(),
        target_account_id,
        lifecycle: LoginAttemptLifecycle::Preparing,
        observed_identity_hash: None,
        marker_validation_id: None,
        updated_at: updated_at.to_string(),
    };
    state.validate()?;
    let path = paths.login_attempt_state_path(login_attempt_id)?;
    if path.exists() {
        return Err("login_attempt_already_exists".to_string());
    }
    file_store::write_json_atomically(&path, &state)?;
    Ok(state)
}

pub fn read_login_attempt_state(
    paths: &AppPaths,
    login_attempt_id: &str,
) -> Result<LoginAttemptStateV1, String> {
    let state: LoginAttemptStateV1 =
        file_store::read_json(&paths.login_attempt_state_path(login_attempt_id)?)?;
    state.validate()?;
    if state.login_attempt_id != login_attempt_id {
        return Err("login_attempt_state_mismatch".to_string());
    }
    Ok(state)
}

pub fn compare_and_swap_login_attempt(
    paths: &AppPaths,
    expected: &LoginAttemptStateV1,
    next: &LoginAttemptStateV1,
) -> Result<(), String> {
    expected.validate()?;
    next.validate()?;
    if next.login_attempt_id != expected.login_attempt_id
        || next.profile_key != expected.profile_key
        || next.target_account_id != expected.target_account_id
        || next.revision != expected.revision.saturating_add(1)
    {
        return Err("login_attempt_state_conflict".to_string());
    }
    let current = read_login_attempt_state(paths, &expected.login_attempt_id)?;
    if current != *expected {
        return Err("login_attempt_state_conflict".to_string());
    }
    file_store::write_json_atomically(
        &paths.login_attempt_state_path(&expected.login_attempt_id)?,
        next,
    )
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| "invalid_login_attempt_state".to_string())
}

fn is_simple_uuid(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
