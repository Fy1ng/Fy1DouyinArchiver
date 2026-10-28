use crate::sync_orchestration::{is_simple_uuid, SyncLifecycle, SyncOrchestrationV1, SyncPhase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseCompletionDecision {
    Consume,
    Cancel,
    FailInterrupted,
    StartRecovery,
    StartRetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupRecoveryDecision {
    Consume,
    Cancel,
    FailInterrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseCompletionEvidence {
    pub process_succeeded: bool,
    pub result_ready: bool,
    pub diagnostic_status: Option<String>,
    pub diagnostic_error_code: Option<String>,
    pub cancel_file_exists: bool,
}

pub fn phase_completion_decision(
    orchestration: &SyncOrchestrationV1,
    evidence: &PhaseCompletionEvidence,
) -> PhaseCompletionDecision {
    if orchestration.lifecycle == SyncLifecycle::Consuming {
        return if matches!(
            orchestration.phase,
            SyncPhase::Collecting | SyncPhase::RetryingAfterLogin
        ) && evidence.process_succeeded
            && evidence.result_ready
        {
            PhaseCompletionDecision::Consume
        } else {
            PhaseCompletionDecision::FailInterrupted
        };
    }

    if orchestration.cancel_requested_at.is_some()
        || evidence.cancel_file_exists
        || evidence.diagnostic_status.as_deref() == Some("cancelled")
        || evidence.diagnostic_error_code.as_deref() == Some("collector_cancelled")
    {
        return PhaseCompletionDecision::Cancel;
    }
    if orchestration.lifecycle != SyncLifecycle::Running {
        return PhaseCompletionDecision::FailInterrupted;
    }

    match orchestration.phase {
        SyncPhase::Collecting if evidence.process_succeeded && evidence.result_ready => {
            PhaseCompletionDecision::Consume
        }
        SyncPhase::Collecting
            if orchestration.login_recovery_count == 0
                && evidence.diagnostic_error_code.as_deref() == Some("session_expired") =>
        {
            PhaseCompletionDecision::StartRecovery
        }
        SyncPhase::RecoveringLogin if evidence.process_succeeded && evidence.result_ready => {
            PhaseCompletionDecision::StartRetry
        }
        SyncPhase::RetryingAfterLogin if evidence.process_succeeded && evidence.result_ready => {
            PhaseCompletionDecision::Consume
        }
        _ => PhaseCompletionDecision::FailInterrupted,
    }
}

pub fn startup_recovery_decision(
    orchestration: &SyncOrchestrationV1,
    result_ready: bool,
    cancel_file_exists: bool,
) -> StartupRecoveryDecision {
    if orchestration.lifecycle == SyncLifecycle::Consuming
        && matches!(
            orchestration.phase,
            SyncPhase::Collecting | SyncPhase::RetryingAfterLogin
        )
        && result_ready
    {
        return StartupRecoveryDecision::Consume;
    }
    if orchestration.cancel_requested_at.is_some() || cancel_file_exists {
        return StartupRecoveryDecision::Cancel;
    }
    StartupRecoveryDecision::FailInterrupted
}

pub fn prepare_next_phase(
    current: &SyncOrchestrationV1,
    next_phase: SyncPhase,
    next_phase_attempt_id: &str,
    next_deadline_at_ms: Option<i64>,
    now_ms: i64,
    updated_at: &str,
) -> Result<SyncOrchestrationV1, String> {
    if current.cancel_requested_at.is_some() {
        return Err("sync_cancel_requested".to_string());
    }
    if current.lifecycle == SyncLifecycle::Consuming {
        return Err("sync_finalizing".to_string());
    }
    if current.lifecycle != SyncLifecycle::Running
        || !is_simple_uuid(next_phase_attempt_id)
        || now_ms <= 0
        || chrono::DateTime::parse_from_rfc3339(updated_at).is_err()
    {
        return Err("sync_phase_transition_invalid".to_string());
    }

    let (login_recovery_count, deadline_at_ms) = match (current.phase, next_phase) {
        (SyncPhase::Collecting, SyncPhase::RecoveringLogin)
            if current.login_recovery_count == 0
                && current.deadline_at_ms.is_none()
                && next_deadline_at_ms.is_some_and(|deadline| deadline > now_ms) =>
        {
            (1, next_deadline_at_ms)
        }
        (SyncPhase::RecoveringLogin, SyncPhase::RetryingAfterLogin)
            if current.login_recovery_count == 1 && next_deadline_at_ms.is_none() =>
        {
            let deadline = current
                .deadline_at_ms
                .ok_or_else(|| "sync_phase_transition_invalid".to_string())?;
            if now_ms >= deadline {
                return Err("login_recovery_failed".to_string());
            }
            (1, None)
        }
        _ => return Err("sync_phase_transition_invalid".to_string()),
    };

    Ok(SyncOrchestrationV1 {
        revision: current
            .revision
            .checked_add(1)
            .ok_or_else(|| "sync_orchestration_revision_overflow".to_string())?,
        phase: next_phase,
        phase_attempt_id: next_phase_attempt_id.to_string(),
        lifecycle: SyncLifecycle::Preparing,
        login_recovery_count,
        deadline_at_ms,
        updated_at: updated_at.to_string(),
        ..current.clone()
    })
}
