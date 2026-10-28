use crate::device_settings::{DeviceSettingsStore, DeviceSettingsV1};
use serde::{Deserialize, Serialize};
use std::{io::ErrorKind, path::Path, sync::Mutex};
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
    RegKey,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const EMILY_RUN_VALUE: &str = "LifeArchiveProject.Emily";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutostartStateV1 {
    pub enabled: bool,
    pub command: Option<String>,
    pub executable_matches: bool,
}

pub trait RunRegistry: Send + Sync {
    fn read_value(&self) -> Result<Option<String>, String>;
    fn write_value(&self, value: &str) -> Result<(), String>;
    fn delete_value(&self) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct HkcuRunRegistry {
    value_name: String,
}

impl HkcuRunRegistry {
    pub fn emily() -> Self {
        Self {
            value_name: EMILY_RUN_VALUE.to_string(),
        }
    }

    pub fn with_value_name(value_name: String) -> Result<Self, String> {
        if value_name.is_empty()
            || value_name.len() > 256
            || value_name.chars().any(|character| character == '\0')
        {
            return Err("invalid_autostart_value_name".to_string());
        }
        Ok(Self { value_name })
    }

    fn current_user() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }
}

impl Default for HkcuRunRegistry {
    fn default() -> Self {
        Self::emily()
    }
}

impl RunRegistry for HkcuRunRegistry {
    fn read_value(&self) -> Result<Option<String>, String> {
        let key = match Self::current_user().open_subkey_with_flags(RUN_KEY, KEY_READ) {
            Ok(key) => key,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        match key.get_value::<String, _>(&self.value_name) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write_value(&self, value: &str) -> Result<(), String> {
        let (key, _) = Self::current_user()
            .create_subkey(RUN_KEY)
            .map_err(|error| error.to_string())?;
        key.set_value(&self.value_name, &value)
            .map_err(|error| error.to_string())
    }

    fn delete_value(&self) -> Result<(), String> {
        let key = match Self::current_user().open_subkey_with_flags(RUN_KEY, KEY_WRITE) {
            Ok(key) => key,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        match key.delete_value(&self.value_name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

pub fn expected_run_command(executable: &Path) -> Result<String, String> {
    if !executable.is_absolute() {
        return Err("invalid_autostart_executable".to_string());
    }
    let executable = executable
        .to_str()
        .ok_or_else(|| "invalid_autostart_executable".to_string())?;
    if executable.is_empty()
        || executable
            .chars()
            .any(|character| matches!(character, '"' | '\r' | '\n' | '\0'))
    {
        return Err("invalid_autostart_executable".to_string());
    }
    Ok(format!(r#""{executable}" --autostart"#))
}

pub fn inspect_autostart(executable: &Path) -> Result<AutostartStateV1, String> {
    inspect_autostart_with_registry(&HkcuRunRegistry::emily(), executable)
}

pub fn inspect_autostart_with_registry(
    registry: &(impl RunRegistry + ?Sized),
    executable: &Path,
) -> Result<AutostartStateV1, String> {
    let expected = expected_run_command(executable)?;
    let command = read_registry(registry)?;
    Ok(state_from_command(command, &expected))
}

pub fn set_autostart_transactional(
    store: &DeviceSettingsStore,
    desktop_settings_tx: &Mutex<()>,
    executable: &Path,
    enabled: bool,
    silent: bool,
) -> Result<DeviceSettingsV1, String> {
    let expected_revision = store.load()?.revision;
    set_autostart_transactional_at_revision(
        store,
        desktop_settings_tx,
        executable,
        expected_revision,
        enabled,
        silent,
        |_| Ok(()),
    )
}

pub fn set_autostart_transactional_at_revision(
    store: &DeviceSettingsStore,
    desktop_settings_tx: &Mutex<()>,
    executable: &Path,
    expected_revision: u64,
    enabled: bool,
    silent: bool,
    update: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>,
) -> Result<DeviceSettingsV1, String> {
    set_autostart_transactional_at_revision_with_registry(
        store,
        desktop_settings_tx,
        &HkcuRunRegistry::emily(),
        executable,
        expected_revision,
        enabled,
        silent,
        update,
    )
}

pub fn set_autostart_transactional_with_registry(
    store: &DeviceSettingsStore,
    desktop_settings_tx: &Mutex<()>,
    registry: &(impl RunRegistry + ?Sized),
    executable: &Path,
    enabled: bool,
    silent: bool,
) -> Result<DeviceSettingsV1, String> {
    let expected_revision = store.load()?.revision;
    set_autostart_transactional_at_revision_with_registry(
        store,
        desktop_settings_tx,
        registry,
        executable,
        expected_revision,
        enabled,
        silent,
        |_| Ok(()),
    )
}

pub fn set_autostart_transactional_at_revision_with_registry(
    store: &DeviceSettingsStore,
    desktop_settings_tx: &Mutex<()>,
    registry: &(impl RunRegistry + ?Sized),
    executable: &Path,
    expected_revision: u64,
    enabled: bool,
    silent: bool,
    update: impl FnOnce(&mut DeviceSettingsV1) -> Result<(), String>,
) -> Result<DeviceSettingsV1, String> {
    if silent && !enabled {
        return Err("invalid_device_settings".to_string());
    }
    let expected = expected_run_command(executable)?;
    let _transaction = desktop_settings_tx
        .lock()
        .map_err(|_| "desktop_settings_lock_poisoned".to_string())?;
    let old_device = store.load()?;
    if old_device.revision != expected_revision {
        return Err("device_settings_revision_conflict".to_string());
    }
    let old_registry = read_registry(registry)?;
    let desired = enabled.then_some(expected);
    update_registry_transactionally(registry, old_registry.as_deref(), desired.as_deref())?;

    match store.compare_and_swap(expected_revision, |settings| {
        settings.launch_at_startup = enabled;
        settings.silent_autostart = silent;
        update(settings)
    }) {
        Ok(settings) => Ok(settings),
        Err(error) => {
            restore_registry(registry, old_registry.as_deref())?;
            Err(error)
        }
    }
}

pub fn repair_autostart_on_startup(
    store: &DeviceSettingsStore,
    desktop_settings_tx: &Mutex<()>,
    executable: &Path,
) -> Result<AutostartStateV1, String> {
    repair_autostart_on_startup_with_registry(
        store,
        desktop_settings_tx,
        &HkcuRunRegistry::emily(),
        executable,
    )
}

pub fn repair_autostart_on_startup_with_registry(
    store: &DeviceSettingsStore,
    desktop_settings_tx: &Mutex<()>,
    registry: &(impl RunRegistry + ?Sized),
    executable: &Path,
) -> Result<AutostartStateV1, String> {
    let expected = expected_run_command(executable)?;
    let _transaction = desktop_settings_tx
        .lock()
        .map_err(|_| "desktop_settings_lock_poisoned".to_string())?;
    let settings = store.load()?;
    let old_registry = read_registry(registry)?;
    let desired = settings.launch_at_startup.then_some(expected.clone());
    update_registry_transactionally(registry, old_registry.as_deref(), desired.as_deref())?;
    Ok(state_from_command(desired, &expected))
}

fn read_registry(registry: &(impl RunRegistry + ?Sized)) -> Result<Option<String>, String> {
    registry
        .read_value()
        .map_err(|_| "autostart_registry_update_failed".to_string())
}

fn update_registry_transactionally(
    registry: &(impl RunRegistry + ?Sized),
    old_value: Option<&str>,
    desired: Option<&str>,
) -> Result<(), String> {
    if old_value == desired {
        return Ok(());
    }
    let mutation_failed = apply_registry_value(registry, desired).is_err();
    let desired_verified = registry
        .read_value()
        .map(|value| value.as_deref() == desired)
        .unwrap_or(false);
    if mutation_failed || !desired_verified {
        return match restore_registry(registry, old_value) {
            Ok(()) => Err("autostart_registry_update_failed".to_string()),
            Err(error) => Err(error),
        };
    }
    Ok(())
}

fn restore_registry(
    registry: &(impl RunRegistry + ?Sized),
    old_value: Option<&str>,
) -> Result<(), String> {
    let _ = apply_registry_value(registry, old_value);
    let restored = registry
        .read_value()
        .map(|value| value.as_deref() == old_value)
        .unwrap_or(false);
    if !restored {
        return Err("autostart_state_uncertain".to_string());
    }
    Ok(())
}

fn apply_registry_value(
    registry: &(impl RunRegistry + ?Sized),
    value: Option<&str>,
) -> Result<(), String> {
    match value {
        Some(value) => registry.write_value(value),
        None => registry.delete_value(),
    }
}

fn state_from_command(command: Option<String>, expected: &str) -> AutostartStateV1 {
    AutostartStateV1 {
        enabled: command.is_some(),
        executable_matches: command.as_deref() == Some(expected),
        command,
    }
}
