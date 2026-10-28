use super::{DownloadError, DownloadEvent, DownloadMutation, PersistenceHandle};

#[derive(Clone)]
pub struct DownloadQueue {
    persistence: PersistenceHandle,
}

impl DownloadQueue {
    pub fn new(persistence: PersistenceHandle) -> Self {
        Self { persistence }
    }
    pub async fn start_next(&self) -> Result<DownloadEvent, DownloadError> {
        self.persistence
            .transition(DownloadMutation::StartNext)
            .await
    }
    pub async fn pause(&self, job_id: i64) -> Result<DownloadEvent, DownloadError> {
        self.persistence
            .transition(DownloadMutation::Pause { job_id })
            .await
    }
    pub async fn resume(&self, job_id: i64) -> Result<DownloadEvent, DownloadError> {
        self.persistence
            .transition(DownloadMutation::Resume { job_id })
            .await
    }
    pub async fn cancel(&self, job_id: i64) -> Result<DownloadEvent, DownloadError> {
        self.persistence
            .transition(DownloadMutation::Cancel { job_id })
            .await
    }
    pub async fn retry_failed(&self, job_id: i64) -> Result<DownloadEvent, DownloadError> {
        self.persistence
            .transition(DownloadMutation::RetryFailed { job_id })
            .await
    }
}
