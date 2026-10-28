use crate::{
    device_settings::{
        BrowserPreferenceV1, DeviceSettingsStore, DeviceSettingsV1, DeviceSettingsViewV1,
    },
    settings_store::{
        compare_and_swap_portable_settings, load_portable_settings, update_account_defaults,
        AccountSettingsV2, ArchiveListRangeV1, CloseBehaviorV1, HistoryRangeV1, PortableSettingsV2,
    },
    sync::{SourceCapabilityViewV1, SyncRangeKindV1, SyncSourceV1},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BeginAccountLoginRequestV1 {
    pub browser_preference: Option<BrowserPreferenceV1>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitOnboardingStepRequestV1 {
    pub expected_portable_revision: u64,
    pub expected_device_revision: u64,
    pub step: OnboardingStepRequestV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum OnboardingStepRequestV1 {
    Account {
        skipped: bool,
    },
    SyncRange {
        account_defaults: AccountSettingsV2,
        sync_after_onboarding: bool,
    },
    AutomaticSync {
        account_defaults: AccountSettingsV2,
        startup_sync_delay_minutes: u32,
    },
    DesktopBehavior {
        close_behavior: CloseBehaviorV1,
        launch_at_startup: bool,
        silent_autostart: bool,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommitOnboardingStepResultV1 {
    pub portable_revision: u64,
    pub device: DeviceSettingsViewV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetBrowserPreferenceRequestV1 {
    pub account_id: i64,
    pub browser_preference: BrowserPreferenceV1,
    pub confirm_relogin: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetAutostartRequestV1 {
    pub expected_device_revision: u64,
    pub enabled: bool,
    pub silent: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopStateViewV1 {
    pub device: DeviceSettingsViewV1,
    pub tray_available: bool,
    pub tray_error: Option<String>,
    pub autostart_error: Option<String>,
}

pub fn build_desktop_state_view(
    device: DeviceSettingsV1,
    tray_available: bool,
    tray_error: Option<String>,
    autostart_error: Option<String>,
) -> DesktopStateViewV1 {
    DesktopStateViewV1 {
        device: device.into(),
        tray_available,
        tray_error,
        autostart_error,
    }
}

pub fn update_account_defaults_for_capabilities(
    settings_root: &Path,
    value: AccountSettingsV2,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<crate::settings_store::PortableSettingsV2, String> {
    validate_account_defaults_for_capabilities(&value, capabilities)?;
    update_account_defaults(settings_root, value)
}

pub fn commit_onboarding_step(
    settings_root: &Path,
    device_store: &DeviceSettingsStore,
    request: &CommitOnboardingStepRequestV1,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<CommitOnboardingStepResultV1, String> {
    commit_onboarding_step_with_device_commit(
        settings_root,
        device_store,
        request,
        capabilities,
        |expected_revision, step, next_step, completes_onboarding| {
            device_store.compare_and_swap(expected_revision, |settings| {
                if let OnboardingStepRequestV1::DesktopBehavior {
                    launch_at_startup,
                    silent_autostart,
                    ..
                } = step
                {
                    settings.launch_at_startup = *launch_at_startup;
                    settings.silent_autostart = *silent_autostart;
                }
                settings.onboarding_step = next_step;
                settings.onboarding_completed = completes_onboarding;
                Ok(())
            })
        },
    )
}

pub fn commit_onboarding_step_with_device_commit<F>(
    settings_root: &Path,
    device_store: &DeviceSettingsStore,
    request: &CommitOnboardingStepRequestV1,
    capabilities: &[SourceCapabilityViewV1],
    commit_device: F,
) -> Result<CommitOnboardingStepResultV1, String>
where
    F: FnOnce(
        u64,
        &OnboardingStepRequestV1,
        u8,
        bool,
    ) -> Result<crate::device_settings::DeviceSettingsV1, String>,
{
    let portable = load_portable_settings(settings_root)?;
    let device = device_store.load()?;
    let current_step = request.step.current_step();
    let (requested_next_step, completes_onboarding) = request.step.progress_target();

    if request.step.already_committed(&portable, &device) {
        return success(portable, device);
    }
    if device.onboarding_completed {
        return Err("onboarding_request_conflict".to_string());
    }
    let portable_already_committed = request.step.portable_already_committed(&portable);
    let portable_revision_matches = portable.revision == request.expected_portable_revision
        || (portable_already_committed
            && portable.revision == request.expected_portable_revision.saturating_add(1));
    if !portable_revision_matches {
        return Err("portable_settings_revision_conflict".to_string());
    }
    if device.revision != request.expected_device_revision {
        return Err("device_settings_revision_conflict".to_string());
    }
    if device.onboarding_step < current_step {
        return Err("onboarding_step_mismatch".to_string());
    }
    let next_step = device.onboarding_step.max(requested_next_step);

    let portable = match &request.step {
        OnboardingStepRequestV1::Account { skipped: _ } => {
            validate_account_defaults_for_capabilities(&portable.account_defaults, capabilities)?;
            portable
        }
        OnboardingStepRequestV1::SyncRange {
            account_defaults,
            sync_after_onboarding: _,
        } => update_onboarding_portable(
            settings_root,
            request.expected_portable_revision,
            portable,
            account_defaults,
            None,
            capabilities,
        )?,
        OnboardingStepRequestV1::AutomaticSync {
            account_defaults,
            startup_sync_delay_minutes,
        } => update_onboarding_portable(
            settings_root,
            request.expected_portable_revision,
            portable,
            account_defaults,
            Some(*startup_sync_delay_minutes),
            capabilities,
        )?,
        OnboardingStepRequestV1::DesktopBehavior {
            close_behavior,
            launch_at_startup: _,
            silent_autostart: _,
        } => update_onboarding_close_behavior(
            settings_root,
            request.expected_portable_revision,
            portable,
            *close_behavior,
        )?,
    };

    let persisted = commit_device(
        request.expected_device_revision,
        &request.step,
        next_step,
        completes_onboarding,
    )?;
    success(portable, persisted)
}

impl OnboardingStepRequestV1 {
    fn current_step(&self) -> u8 {
        match self {
            Self::Account { .. } => 0,
            Self::SyncRange { .. } => 1,
            Self::AutomaticSync { .. } => 2,
            Self::DesktopBehavior { .. } => 3,
        }
    }

    fn progress_target(&self) -> (u8, bool) {
        match self {
            Self::Account { .. } => (1, false),
            Self::SyncRange { .. } => (2, false),
            Self::AutomaticSync { .. } => (3, false),
            Self::DesktopBehavior { .. } => (3, true),
        }
    }

    fn already_committed(
        &self,
        portable: &PortableSettingsV2,
        device: &crate::device_settings::DeviceSettingsV1,
    ) -> bool {
        match self {
            Self::Account { .. } => device.onboarding_step >= 1,
            Self::SyncRange {
                account_defaults, ..
            } => device.onboarding_step >= 2 && portable.account_defaults == *account_defaults,
            Self::AutomaticSync {
                account_defaults,
                startup_sync_delay_minutes,
            } => {
                device.onboarding_step >= 3
                    && portable.account_defaults == *account_defaults
                    && portable.global.startup_sync_delay_minutes == *startup_sync_delay_minutes
            }
            Self::DesktopBehavior {
                close_behavior,
                launch_at_startup,
                silent_autostart,
            } => {
                device.onboarding_completed
                    && portable.global.close_behavior == *close_behavior
                    && device.launch_at_startup == *launch_at_startup
                    && device.silent_autostart == *silent_autostart
            }
        }
    }

    fn portable_already_committed(&self, portable: &PortableSettingsV2) -> bool {
        match self {
            Self::Account { .. } => true,
            Self::SyncRange {
                account_defaults, ..
            } => portable.account_defaults == *account_defaults,
            Self::AutomaticSync {
                account_defaults,
                startup_sync_delay_minutes,
            } => {
                portable.account_defaults == *account_defaults
                    && portable.global.startup_sync_delay_minutes == *startup_sync_delay_minutes
            }
            Self::DesktopBehavior { close_behavior, .. } => {
                portable.global.close_behavior == *close_behavior
            }
        }
    }
}

fn update_onboarding_portable(
    settings_root: &Path,
    expected_revision: u64,
    current: PortableSettingsV2,
    account_defaults: &AccountSettingsV2,
    startup_sync_delay_minutes: Option<u32>,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<PortableSettingsV2, String> {
    validate_account_defaults_for_capabilities(account_defaults, capabilities)?;
    let already_persisted = current.revision > expected_revision
        && current.account_defaults == *account_defaults
        && startup_sync_delay_minutes
            .is_none_or(|value| current.global.startup_sync_delay_minutes == value);
    if already_persisted {
        return Ok(current);
    }
    compare_and_swap_portable_settings(settings_root, expected_revision, |settings| {
        settings.account_defaults = account_defaults.clone();
        if let Some(value) = startup_sync_delay_minutes {
            settings.global.startup_sync_delay_minutes = value;
        }
        Ok(())
    })
}

fn update_onboarding_close_behavior(
    settings_root: &Path,
    expected_revision: u64,
    current: PortableSettingsV2,
    close_behavior: CloseBehaviorV1,
) -> Result<PortableSettingsV2, String> {
    if current.revision > expected_revision && current.global.close_behavior == close_behavior {
        return Ok(current);
    }
    compare_and_swap_portable_settings(settings_root, expected_revision, |settings| {
        settings.global.close_behavior = close_behavior;
        Ok(())
    })
}

fn success(
    portable: PortableSettingsV2,
    device: crate::device_settings::DeviceSettingsV1,
) -> Result<CommitOnboardingStepResultV1, String> {
    Ok(CommitOnboardingStepResultV1 {
        portable_revision: portable.revision,
        device: device.into(),
    })
}

fn validate_account_defaults_for_capabilities(
    value: &AccountSettingsV2,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<(), String> {
    validate_history_range(value.history.range, capabilities)?;
    validate_archive_range(
        SyncSourceV1::Likes,
        value.likes.range,
        value.likes.schedule.enabled,
        capabilities,
    )?;
    validate_archive_range(
        SyncSourceV1::Favorites,
        value.favorites.archive.range,
        value.favorites.archive.schedule.enabled,
        capabilities,
    )?;
    validate_archive_range(
        SyncSourceV1::RemoteFolder,
        value.favorites.remote_folder_defaults.range,
        value.favorites.remote_folder_defaults.schedule.enabled,
        capabilities,
    )?;
    for settings in value.favorites.remote_folders.values() {
        validate_archive_range(
            SyncSourceV1::RemoteFolder,
            settings.range,
            settings.schedule.enabled,
            capabilities,
        )?;
    }
    Ok(())
}

fn validate_history_range(
    range: HistoryRangeV1,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<(), String> {
    let kind = match range {
        HistoryRangeV1::Days { .. } => SyncRangeKindV1::Days,
        HistoryRangeV1::Bottom => SyncRangeKindV1::Bottom,
    };
    ensure_range_supported(SyncSourceV1::History, kind, false, capabilities)
}

fn validate_archive_range(
    source: SyncSourceV1,
    range: ArchiveListRangeV1,
    schedule_enabled: bool,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<(), String> {
    let kind = match range {
        ArchiveListRangeV1::All => SyncRangeKindV1::All,
        ArchiveListRangeV1::RecentItems { .. } => SyncRangeKindV1::RecentItems,
        ArchiveListRangeV1::RecordLimit { .. } => SyncRangeKindV1::RecordLimit,
        ArchiveListRangeV1::RecentDays { .. } => SyncRangeKindV1::RecentDays,
    };
    ensure_range_supported(source, kind, schedule_enabled, capabilities)
}

fn ensure_range_supported(
    source: SyncSourceV1,
    kind: SyncRangeKindV1,
    schedule_enabled: bool,
    capabilities: &[SourceCapabilityViewV1],
) -> Result<(), String> {
    let capability = capabilities
        .iter()
        .find(|capability| capability.source == source)
        .ok_or_else(|| "invalid_capability_registry".to_string())?;
    if kind == SyncRangeKindV1::RecentDays && !capability.supported_ranges.contains(&kind) {
        return Err("time_range_not_supported".to_string());
    }
    if !capability.supported_ranges.contains(&kind) {
        return Err("sync_range_not_supported".to_string());
    }
    if schedule_enabled && !capability.available {
        return Err("source_unavailable".to_string());
    }
    Ok(())
}
