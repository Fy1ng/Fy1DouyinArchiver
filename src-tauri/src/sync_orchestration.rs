use crate::{collector::file_store, AppPaths};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    Collecting,
    RecoveringLogin,
    RetryingAfterLogin,
}

impl SyncPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collecting => "collecting",
            Self::RecoveringLogin => "recovering_login",
            Self::RetryingAfterLogin => "retrying_after_login",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncLifecycle {
    Preparing,
    Running,
    Consuming,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncPhaseEnvelopeV1<T> {
    pub envelope_version: i64,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: SyncPhase,
    pub phase_attempt_id: String,
    pub payload: T,
}

impl<T> SyncPhaseEnvelopeV1<T> {
    pub fn new(
        account_id: i64,
        sync_job_id: i64,
        phase: SyncPhase,
        phase_attempt_id: &str,
        payload: T,
    ) -> Result<Self, String> {
        let envelope = Self {
            envelope_version: 1,
            account_id,
            sync_job_id,
            phase,
            phase_attempt_id: phase_attempt_id.to_string(),
            payload,
        };
        envelope.validate_context(account_id, sync_job_id, phase, phase_attempt_id)?;
        Ok(envelope)
    }

    pub fn validate_context(
        &self,
        account_id: i64,
        sync_job_id: i64,
        phase: SyncPhase,
        phase_attempt_id: &str,
    ) -> Result<(), String> {
        if self.envelope_version != 1
            || self.account_id <= 0
            || self.sync_job_id <= 0
            || self.account_id != account_id
            || self.sync_job_id != sync_job_id
            || self.phase != phase
            || !is_simple_uuid(&self.phase_attempt_id)
            || self.phase_attempt_id != phase_attempt_id
        {
            return Err("sync_phase_envelope_mismatch".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncOrchestrationV1 {
    pub schema_version: i64,
    pub revision: u64,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: SyncPhase,
    pub phase_attempt_id: String,
    pub lifecycle: SyncLifecycle,
    pub login_recovery_count: u8,
    pub cancel_requested_at: Option<String>,
    pub options_hash: String,
    pub deadline_at_ms: Option<i64>,
    pub updated_at: String,
}

impl SyncOrchestrationV1 {
    pub fn next(&self, lifecycle: SyncLifecycle, updated_at: &str) -> Result<Self, String> {
        if lifecycle == SyncLifecycle::Consuming && self.cancel_requested_at.is_some() {
            return Err("sync_cancel_requested".to_string());
        }
        let next = Self {
            revision: self.revision.checked_add(1)
                .ok_or_else(|| "sync_orchestration_revision_overflow".to_string())?,
            lifecycle,
            updated_at: updated_at.to_string(),
            ..self.clone()
        };
        next.validate()?;
        if !valid_lifecycle_transition(self.lifecycle, lifecycle) {
            return Err("sync_orchestration_conflict".to_string());
        }
        Ok(next)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.account_id <= 0
            || self.sync_job_id <= 0
            || !is_simple_uuid(&self.phase_attempt_id)
            || self.login_recovery_count > 1
            || !is_sha256(&self.options_hash)
            || self.deadline_at_ms.is_some_and(|value| value <= 0)
        {
            return Err("invalid_sync_orchestration".to_string());
        }
        validate_timestamp(&self.updated_at)?;
        if let Some(value) = &self.cancel_requested_at {
            validate_timestamp(value)?;
        }
        Ok(())
    }
}

pub fn create_orchestration(
    paths: &AppPaths,
    account_id: i64,
    sync_job_id: i64,
    phase: SyncPhase,
    phase_attempt_id: &str,
    options_hash: &str,
    deadline_at_ms: Option<i64>,
    updated_at: &str,
) -> Result<SyncOrchestrationV1, String> {
    let state = SyncOrchestrationV1 {
        schema_version: 1,
        revision: 0,
        account_id,
        sync_job_id,
        phase,
        phase_attempt_id: phase_attempt_id.to_string(),
        lifecycle: SyncLifecycle::Preparing,
        login_recovery_count: 0,
        cancel_requested_at: None,
        options_hash: options_hash.to_string(),
        deadline_at_ms,
        updated_at: updated_at.to_string(),
    };
    state.validate()?;
    let path = paths.orchestration_path(account_id, sync_job_id);
    if path.exists() {
        return Err("sync_orchestration_already_exists".to_string());
    }
    file_store::write_json_atomically(&path, &state)?;
    Ok(state)
}

pub fn read_orchestration(
    paths: &AppPaths,
    account_id: i64,
    sync_job_id: i64,
) -> Result<SyncOrchestrationV1, String> {
    let state: SyncOrchestrationV1 =
        file_store::read_json(&paths.orchestration_path(account_id, sync_job_id))?;
    state.validate()?;
    if state.account_id != account_id || state.sync_job_id != sync_job_id {
        return Err("sync_orchestration_mismatch".to_string());
    }
    Ok(state)
}

pub fn cas_orchestration(
    paths: &AppPaths,
    expected: &SyncOrchestrationV1,
    next: &SyncOrchestrationV1,
) -> Result<(), String> {
    expected.validate()?;
    next.validate()?;
    if next.account_id != expected.account_id
        || next.sync_job_id != expected.sync_job_id
        || next.options_hash != expected.options_hash
        || next.cancel_requested_at != expected.cancel_requested_at
        || next.revision != expected.revision.saturating_add(1)
        || !valid_orchestration_transition(expected, next)
    {
        return Err("sync_orchestration_conflict".to_string());
    }
    let current = read_orchestration(paths, expected.account_id, expected.sync_job_id)?;
    if current != *expected {
        return Err("sync_orchestration_conflict".to_string());
    }
    file_store::write_json_atomically(
        &paths.orchestration_path(expected.account_id, expected.sync_job_id),
        next,
    )
}

pub fn request_orchestration_cancel(
    paths: &AppPaths,
    expected: &SyncOrchestrationV1,
    requested_at: &str,
) -> Result<SyncOrchestrationV1, String> {
    expected.validate()?;
    validate_timestamp(requested_at)?;
    let current = read_orchestration(paths, expected.account_id, expected.sync_job_id)?;
    if current != *expected {
        return Err("sync_orchestration_conflict".to_string());
    }
    if current.lifecycle == SyncLifecycle::Consuming {
        return Err("sync_finalizing".to_string());
    }
    if current.cancel_requested_at.is_some() {
        return Ok(current);
    }
    let next = SyncOrchestrationV1 {
        revision: current
            .revision
            .checked_add(1)
            .ok_or_else(|| "sync_orchestration_revision_overflow".to_string())?,
        cancel_requested_at: Some(requested_at.to_string()),
        updated_at: requested_at.to_string(),
        ..current
    };
    next.validate()?;
    file_store::write_json_atomically(
        &paths.orchestration_path(next.account_id, next.sync_job_id),
        &next,
    )?;
    Ok(next)
}

fn valid_lifecycle_transition(from: SyncLifecycle, to: SyncLifecycle) -> bool {
    matches!(
        (from, to),
        (SyncLifecycle::Preparing, SyncLifecycle::Running)
            | (SyncLifecycle::Running, SyncLifecycle::Consuming)
    )
}

fn valid_orchestration_transition(
    current: &SyncOrchestrationV1,
    next: &SyncOrchestrationV1,
) -> bool {
    if current.phase == next.phase
        && current.phase_attempt_id == next.phase_attempt_id
        && current.login_recovery_count == next.login_recovery_count
        && current.deadline_at_ms == next.deadline_at_ms
    {
        return valid_lifecycle_transition(current.lifecycle, next.lifecycle);
    }
    if current.lifecycle != SyncLifecycle::Running
        || next.lifecycle != SyncLifecycle::Preparing
        || current.phase_attempt_id == next.phase_attempt_id
        || current.cancel_requested_at.is_some()
    {
        return false;
    }
    match (current.phase, next.phase) {
        (SyncPhase::Collecting, SyncPhase::RecoveringLogin) => {
            current.login_recovery_count == 0
                && current.deadline_at_ms.is_none()
                && next.login_recovery_count == 1
                && next.deadline_at_ms.is_some()
        }
        (SyncPhase::RecoveringLogin, SyncPhase::RetryingAfterLogin) => {
            current.login_recovery_count == 1
                && current.deadline_at_ms.is_some()
                && next.login_recovery_count == 1
                && next.deadline_at_ms.is_none()
        }
        _ => false,
    }
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| "invalid_sync_orchestration".to_string())
}

pub(crate) fn is_simple_uuid(value: &str) -> bool {
    value.len() == 32
        && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
