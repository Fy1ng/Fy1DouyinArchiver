use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::future::Future;

#[derive(Debug, Clone, Copy)]
pub struct Deadline {
    at: Instant,
}

impl Deadline {
    pub fn after(duration: Duration) -> Self {
        Self {
            at: Instant::now() + duration,
        }
    }

    pub fn from_unix_ms(unix_ms: i64) -> Result<Self, String> {
        let target = UNIX_EPOCH
            .checked_add(Duration::from_millis(
                u64::try_from(unix_ms).map_err(|_| "invalid_deadline".to_string())?,
            ))
            .ok_or_else(|| "invalid_deadline".to_string())?;
        let remaining = target
            .duration_since(SystemTime::now())
            .map_err(|_| "collector_deadline_exceeded".to_string())?;
        Ok(Self::after(remaining))
    }

    pub fn remaining(self) -> Result<Duration, String> {
        self.at
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| "collector_deadline_exceeded".to_string())
    }

    pub fn capped(self, duration: Duration) -> Result<Self, String> {
        Ok(Self::after(self.remaining()?.min(duration)))
    }

    pub async fn run<T, F>(self, future: F) -> Result<T, String>
    where
        F: Future<Output = Result<T, String>>,
    {
        tokio::time::timeout(self.remaining()?, future)
            .await
            .map_err(|_| "collector_deadline_exceeded".to_string())?
    }
}
