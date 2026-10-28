use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri_plugin_updater::Update;
use url::Url;

pub const UPDATE_NOT_CONFIGURED: &str = "update_not_configured";
pub const INVALID_UPDATE_ENDPOINT: &str = "invalid_update_endpoint";
pub const UPDATE_OPERATION_IN_PROGRESS: &str = "update_operation_in_progress";
pub const UPDATE_NOT_AVAILABLE: &str = "update_not_available";
pub const UPDATE_NOT_DOWNLOADED: &str = "update_not_downloaded";
pub const INVALID_UPDATE_STATE: &str = "invalid_update_state";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateConfigurationV1 {
    pub endpoint: Url,
    pub pubkey: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateConfigurationStateV1 {
    Configured(UpdateConfigurationV1),
    NotConfigured,
    Invalid(String),
}

impl UpdateConfigurationStateV1 {
    pub fn from_values(endpoint: Option<&str>, pubkey: Option<&str>) -> Self {
        let endpoint = endpoint.map(str::trim).filter(|value| !value.is_empty());
        let pubkey = pubkey.map(str::trim).filter(|value| !value.is_empty());
        let (Some(endpoint), Some(pubkey)) = (endpoint, pubkey) else {
            return Self::NotConfigured;
        };
        let endpoint = match Url::parse(endpoint) {
            Ok(endpoint) if endpoint.scheme() == "https" => endpoint,
            _ => return Self::Invalid(INVALID_UPDATE_ENDPOINT.to_string()),
        };
        Self::Configured(UpdateConfigurationV1 {
            endpoint,
            pubkey: pubkey.to_string(),
        })
    }

    pub fn compiled() -> Self {
        Self::from_values(
            option_env!("EMILY_UPDATER_ENDPOINT"),
            option_env!("EMILY_UPDATER_PUBKEY"),
        )
    }

    pub fn require(&self) -> Result<&UpdateConfigurationV1, String> {
        match self {
            Self::Configured(value) => Ok(value),
            Self::NotConfigured => Err(UPDATE_NOT_CONFIGURED.to_string()),
            Self::Invalid(error) => Err(error.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhaseV1 {
    NotConfigured,
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Downloaded,
    Installing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReleaseV1 {
    pub version: String,
    pub notes: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStateViewV1 {
    pub configured: bool,
    pub phase: UpdatePhaseV1,
    pub current_version: String,
    pub available_version: Option<String>,
    pub notes: Option<String>,
    pub published_at: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub last_error: Option<String>,
    pub can_check: bool,
    pub can_download: bool,
    pub can_install: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateStateMachineV1 {
    configured: bool,
    configuration_error: Option<String>,
    phase: UpdatePhaseV1,
    current_version: String,
    release: Option<UpdateReleaseV1>,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    last_error: Option<String>,
}

impl UpdateStateMachineV1 {
    pub fn new(
        configuration: &UpdateConfigurationStateV1,
        current_version: impl Into<String>,
    ) -> Self {
        let configured = configuration.require().is_ok();
        let configuration_error = if configured {
            None
        } else {
            configuration.require().err()
        };
        Self {
            configured,
            configuration_error,
            phase: if configured {
                UpdatePhaseV1::Idle
            } else {
                UpdatePhaseV1::NotConfigured
            },
            current_version: current_version.into(),
            release: None,
            downloaded_bytes: 0,
            total_bytes: None,
            last_error: None,
        }
    }

    pub fn snapshot(&self) -> UpdateStateViewV1 {
        let busy = matches!(
            self.phase,
            UpdatePhaseV1::Checking | UpdatePhaseV1::Downloading | UpdatePhaseV1::Installing
        );
        UpdateStateViewV1 {
            configured: self.configured,
            phase: self.phase.clone(),
            current_version: self.current_version.clone(),
            available_version: self.release.as_ref().map(|release| release.version.clone()),
            notes: self
                .release
                .as_ref()
                .and_then(|release| release.notes.clone()),
            published_at: self
                .release
                .as_ref()
                .and_then(|release| release.published_at.clone()),
            downloaded_bytes: self.downloaded_bytes,
            total_bytes: self.total_bytes,
            last_error: self
                .last_error
                .clone()
                .or_else(|| self.configuration_error.clone()),
            can_check: self.configured && !busy,
            can_download: self.configured
                && self.release.is_some()
                && !busy
                && self.phase != UpdatePhaseV1::Downloaded,
            can_install: self.phase == UpdatePhaseV1::Downloaded,
        }
    }

    pub fn begin_check(&mut self) -> Result<(), String> {
        self.require_configured()?;
        self.require_not_busy()?;
        self.phase = UpdatePhaseV1::Checking;
        self.release = None;
        self.downloaded_bytes = 0;
        self.total_bytes = None;
        self.last_error = None;
        Ok(())
    }

    pub fn complete_check(&mut self, release: Option<UpdateReleaseV1>) -> Result<(), String> {
        if self.phase != UpdatePhaseV1::Checking {
            return Err(INVALID_UPDATE_STATE.to_string());
        }
        self.release = release;
        self.phase = if self.release.is_some() {
            UpdatePhaseV1::Available
        } else {
            UpdatePhaseV1::UpToDate
        };
        self.last_error = None;
        Ok(())
    }

    pub fn begin_download(&mut self) -> Result<(), String> {
        self.require_configured()?;
        self.require_not_busy()?;
        if self.release.is_none() {
            return Err(UPDATE_NOT_AVAILABLE.to_string());
        }
        self.phase = UpdatePhaseV1::Downloading;
        self.downloaded_bytes = 0;
        self.total_bytes = None;
        self.last_error = None;
        Ok(())
    }

    pub fn record_download_chunk(
        &mut self,
        bytes: usize,
        total_bytes: Option<u64>,
    ) -> Result<(), String> {
        if self.phase != UpdatePhaseV1::Downloading {
            return Err(INVALID_UPDATE_STATE.to_string());
        }
        self.downloaded_bytes = self.downloaded_bytes.saturating_add(bytes as u64);
        if total_bytes.is_some() {
            self.total_bytes = total_bytes;
        }
        Ok(())
    }

    pub fn complete_download(&mut self) -> Result<(), String> {
        if self.phase != UpdatePhaseV1::Downloading {
            return Err(INVALID_UPDATE_STATE.to_string());
        }
        self.phase = UpdatePhaseV1::Downloaded;
        self.last_error = None;
        Ok(())
    }

    pub fn begin_install(&mut self) -> Result<(), String> {
        if self.phase != UpdatePhaseV1::Downloaded {
            return Err(UPDATE_NOT_DOWNLOADED.to_string());
        }
        self.phase = UpdatePhaseV1::Installing;
        self.last_error = None;
        Ok(())
    }

    pub fn fail(&mut self, error: impl Into<String>) {
        self.phase = if self.configured {
            UpdatePhaseV1::Failed
        } else {
            UpdatePhaseV1::NotConfigured
        };
        self.downloaded_bytes = 0;
        self.total_bytes = None;
        self.last_error = Some(error.into());
    }

    pub fn restore_downloaded_after_install_failure(&mut self, error: impl Into<String>) {
        self.phase = if self.release.is_some() {
            UpdatePhaseV1::Downloaded
        } else {
            UpdatePhaseV1::Failed
        };
        self.last_error = Some(error.into());
    }

    fn require_configured(&self) -> Result<(), String> {
        if self.configured {
            Ok(())
        } else {
            Err(self
                .configuration_error
                .clone()
                .unwrap_or_else(|| UPDATE_NOT_CONFIGURED.to_string()))
        }
    }

    fn require_not_busy(&self) -> Result<(), String> {
        if matches!(
            self.phase,
            UpdatePhaseV1::Checking | UpdatePhaseV1::Downloading | UpdatePhaseV1::Installing
        ) {
            Err(UPDATE_OPERATION_IN_PROGRESS.to_string())
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub struct NativeUpdateRuntime {
    configuration: UpdateConfigurationStateV1,
    inner: Arc<Mutex<NativeUpdateRuntimeInner>>,
}

impl Default for NativeUpdateRuntime {
    fn default() -> Self {
        Self::compiled(env!("CARGO_PKG_VERSION"))
    }
}

struct NativeUpdateRuntimeInner {
    state: UpdateStateMachineV1,
    prepared: Option<Update>,
    bytes: Option<Vec<u8>>,
}

impl NativeUpdateRuntime {
    pub fn compiled(current_version: impl Into<String>) -> Self {
        Self::new(UpdateConfigurationStateV1::compiled(), current_version)
    }

    pub fn new(
        configuration: UpdateConfigurationStateV1,
        current_version: impl Into<String>,
    ) -> Self {
        let state = UpdateStateMachineV1::new(&configuration, current_version);
        Self {
            configuration,
            inner: Arc::new(Mutex::new(NativeUpdateRuntimeInner {
                state,
                prepared: None,
                bytes: None,
            })),
        }
    }

    pub fn configuration(&self) -> Result<UpdateConfigurationV1, String> {
        self.configuration.require().cloned()
    }

    pub fn snapshot(&self) -> Result<UpdateStateViewV1, String> {
        self.inner
            .lock()
            .map(|inner| inner.state.snapshot())
            .map_err(|_| "update_state_poisoned".to_string())
    }

    pub fn begin_check(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?;
        inner.state.begin_check()?;
        inner.prepared = None;
        inner.bytes = None;
        Ok(())
    }

    pub fn complete_check(&self, update: Option<Update>) -> Result<(), String> {
        let release = update.as_ref().map(|update| UpdateReleaseV1 {
            version: update.version.clone(),
            notes: update.body.clone(),
            published_at: update.date.map(|value| value.to_string()),
        });
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?;
        inner.state.complete_check(release)?;
        inner.prepared = update;
        inner.bytes = None;
        Ok(())
    }

    pub fn begin_download(&self) -> Result<Update, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?;
        inner.state.begin_download()?;
        inner.bytes = None;
        inner
            .prepared
            .clone()
            .ok_or_else(|| UPDATE_NOT_AVAILABLE.to_string())
    }

    pub fn record_download_chunk(
        &self,
        bytes: usize,
        total_bytes: Option<u64>,
    ) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?
            .state
            .record_download_chunk(bytes, total_bytes)
    }

    pub fn complete_download(&self, bytes: Vec<u8>) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?;
        inner.state.complete_download()?;
        inner.bytes = Some(bytes);
        Ok(())
    }

    pub fn begin_install(&self) -> Result<(Update, Vec<u8>), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?;
        let update = inner
            .prepared
            .clone()
            .ok_or_else(|| UPDATE_NOT_AVAILABLE.to_string())?;
        if inner.bytes.is_none() {
            return Err(UPDATE_NOT_DOWNLOADED.to_string());
        }
        inner.state.begin_install()?;
        let bytes = inner
            .bytes
            .take()
            .ok_or_else(|| UPDATE_NOT_DOWNLOADED.to_string())?;
        Ok((update, bytes))
    }

    pub fn restore_install_payload(
        &self,
        bytes: Vec<u8>,
        error: impl Into<String>,
    ) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?;
        inner.bytes = Some(bytes);
        inner.state.restore_downloaded_after_install_failure(error);
        Ok(())
    }

    pub fn fail(&self, error: impl Into<String>) -> Result<(), String> {
        self.inner
            .lock()
            .map_err(|_| "update_state_poisoned".to_string())?
            .state
            .fail(error);
        Ok(())
    }
}
