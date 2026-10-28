use crate::collector::contracts::{LoginProgressReason, LoginProgressV1};
use crate::login_attempt::LoginProgressPayloadV1;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginTermination {
    Cancelled,
    BrowserClosed,
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginWatchdogEvent {
    None,
    BeginShutdown(LoginTermination),
    ForceBrowser,
    ForceCollectorAndRelease,
}

#[derive(Debug, Clone)]
pub struct LoginWatchdog {
    started_at: Instant,
    hard_deadline: Instant,
    shutdown_started_at: Option<Instant>,
    shutdown_deadline: Option<Instant>,
    browser_forced: bool,
    released: bool,
}

impl LoginWatchdog {
    pub fn new(started_at: Instant, hard_deadline: Instant) -> Self {
        Self {
            started_at,
            hard_deadline,
            shutdown_started_at: None,
            shutdown_deadline: None,
            browser_forced: false,
            released: false,
        }
    }

    pub fn observe_progress(
        &mut self,
        progress: &LoginProgressV1,
        account_id: i64,
        login_attempt_id: &str,
        now: Instant,
    ) -> LoginWatchdogEvent {
        if progress.validate_for(account_id, login_attempt_id).is_err() {
            return LoginWatchdogEvent::None;
        }
        self.observe_payload(
            &LoginProgressPayloadV1 {
                state: progress.state.clone(),
                reason: progress.reason,
                observed_at_ms: progress.observed_at_ms,
                deadline_at_ms: progress.deadline_at_ms,
            },
            now,
        )
    }

    pub fn observe_payload(
        &mut self,
        progress: &LoginProgressPayloadV1,
        now: Instant,
    ) -> LoginWatchdogEvent {
        if self.shutdown_started_at.is_some()
            || progress.state != "closing"
            || progress.observed_at_ms < 0
            || progress.deadline_at_ms < progress.observed_at_ms
            || progress.deadline_at_ms - progress.observed_at_ms > 2_000
        {
            return LoginWatchdogEvent::None;
        }
        let requested_ms = progress
            .deadline_at_ms
            .saturating_sub(progress.observed_at_ms)
            .clamp(0, 2_000) as u64;
        self.shutdown_started_at = Some(now);
        self.shutdown_deadline = Some(now + Duration::from_millis(requested_ms));
        LoginWatchdogEvent::BeginShutdown(progress.reason.into())
    }

    pub fn observe_browser_exit(&mut self, now: Instant) -> LoginWatchdogEvent {
        if self.shutdown_started_at.is_some() {
            return LoginWatchdogEvent::None;
        }
        self.shutdown_started_at = Some(now);
        self.shutdown_deadline = Some(now + Duration::from_secs(2));
        LoginWatchdogEvent::BeginShutdown(LoginTermination::BrowserClosed)
    }

    pub fn deadline(&self) -> Instant {
        self.shutdown_deadline.unwrap_or(self.hard_deadline)
    }

    pub fn tick(&mut self, now: Instant) -> LoginWatchdogEvent {
        if self.released {
            return LoginWatchdogEvent::None;
        }
        let Some(shutdown_started_at) = self.shutdown_started_at else {
            if now >= self.hard_deadline {
                self.shutdown_started_at = Some(now);
                self.shutdown_deadline = Some(now + Duration::from_secs(2));
                return LoginWatchdogEvent::BeginShutdown(LoginTermination::Timeout);
            }
            return LoginWatchdogEvent::None;
        };
        if now >= self.deadline() {
            self.released = true;
            return LoginWatchdogEvent::ForceCollectorAndRelease;
        }
        if !self.browser_forced
            && now.saturating_duration_since(shutdown_started_at) >= Duration::from_millis(500)
        {
            self.browser_forced = true;
            return LoginWatchdogEvent::ForceBrowser;
        }
        LoginWatchdogEvent::None
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started_at)
    }
}

impl From<LoginProgressReason> for LoginTermination {
    fn from(value: LoginProgressReason) -> Self {
        match value {
            LoginProgressReason::Cancelled => Self::Cancelled,
            LoginProgressReason::BrowserClosed => Self::BrowserClosed,
            LoginProgressReason::Timeout => Self::Timeout,
            LoginProgressReason::Failed => Self::Failed,
        }
    }
}
