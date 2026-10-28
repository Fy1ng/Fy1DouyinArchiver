use douyin_history_archive::{
    autostart::{
        expected_run_command, inspect_autostart_with_registry,
        repair_autostart_on_startup_with_registry, set_autostart_transactional_with_registry,
        HkcuRunRegistry, RunRegistry,
    },
    device_settings::DeviceSettingsStore,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-autostart-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[derive(Default)]
struct FakeRegistry {
    value: Mutex<Option<String>>,
    fail_write: AtomicBool,
    fail_every_mutation: AtomicBool,
    fail_readback_once: AtomicBool,
    fail_mutation_number: AtomicUsize,
    mutations: AtomicUsize,
    after_first_mutation: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl FakeRegistry {
    fn with_value(value: Option<&str>) -> Self {
        Self {
            value: Mutex::new(value.map(str::to_string)),
            ..Self::default()
        }
    }

    fn value(&self) -> Option<String> {
        self.value.lock().unwrap().clone()
    }

    fn fail_write(&self) {
        self.fail_write.store(true, Ordering::SeqCst);
    }

    fn fail_every_mutation(&self) {
        self.fail_every_mutation.store(true, Ordering::SeqCst);
    }

    fn fail_next_readback(&self) {
        self.fail_readback_once.store(true, Ordering::SeqCst);
    }

    fn fail_mutation(&self, number: usize) {
        self.fail_mutation_number.store(number, Ordering::SeqCst);
    }

    fn after_first_mutation(&self, action: impl FnOnce() + Send + 'static) {
        *self.after_first_mutation.lock().unwrap() = Some(Box::new(action));
    }

    fn mutate(&self, value: Option<String>) -> Result<(), String> {
        let mutation = self.mutations.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_every_mutation.load(Ordering::SeqCst)
            || self.fail_write.swap(false, Ordering::SeqCst)
            || self.fail_mutation_number.load(Ordering::SeqCst) == mutation
        {
            return Err("fake_registry_failure".to_string());
        }
        *self.value.lock().unwrap() = value;
        if mutation == 1 {
            if let Some(action) = self.after_first_mutation.lock().unwrap().take() {
                action();
            }
        }
        Ok(())
    }
}

impl RunRegistry for FakeRegistry {
    fn read_value(&self) -> Result<Option<String>, String> {
        if self.mutations.load(Ordering::SeqCst) > 0
            && self.fail_readback_once.swap(false, Ordering::SeqCst)
        {
            return Err("fake_readback_failure".to_string());
        }
        Ok(self.value())
    }

    fn write_value(&self, value: &str) -> Result<(), String> {
        self.mutate(Some(value.to_string()))
    }

    fn delete_value(&self) -> Result<(), String> {
        self.mutate(None)
    }
}

#[test]
fn expected_command_quotes_the_absolute_executable_exactly() {
    let exe = Path::new(r"C:\Program Files\Emily\Emily.exe");
    assert_eq!(
        expected_run_command(exe).unwrap(),
        r#""C:\Program Files\Emily\Emily.exe" --autostart"#
    );
    assert_eq!(
        expected_run_command(Path::new("Emily.exe")).unwrap_err(),
        "invalid_autostart_executable"
    );
    assert_eq!(
        expected_run_command(Path::new("C:\\Bad\"Path\\Emily.exe")).unwrap_err(),
        "invalid_autostart_executable"
    );
}

#[test]
fn enable_and_disable_update_only_the_fixed_registry_value_and_device_fields() {
    let root = temp_root("toggle");
    let store = DeviceSettingsStore::open(root.clone()).unwrap();
    let registry = FakeRegistry::default();
    let desktop_tx = Mutex::new(());
    let exe = Path::new(r"C:\Program Files\Emily\Emily.exe");

    let enabled =
        set_autostart_transactional_with_registry(&store, &desktop_tx, &registry, exe, true, true)
            .unwrap();
    assert!(enabled.launch_at_startup);
    assert!(enabled.silent_autostart);
    assert_eq!(registry.value(), Some(expected_run_command(exe).unwrap()));

    let disabled = set_autostart_transactional_with_registry(
        &store,
        &desktop_tx,
        &registry,
        exe,
        false,
        false,
    )
    .unwrap();
    assert!(!disabled.launch_at_startup);
    assert!(!disabled.silent_autostart);
    assert_eq!(registry.value(), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn registry_write_or_readback_failure_preserves_device_settings_and_restores_registry() {
    let exe = Path::new(r"C:\Emily\Emily.exe");
    for mode in ["write", "readback", "permission-denied"] {
        let root = temp_root(mode);
        let store = DeviceSettingsStore::open(root.clone()).unwrap();
        let before = store.load().unwrap();
        let registry = FakeRegistry::default();
        if mode == "write" {
            registry.fail_write();
        } else if mode == "permission-denied" {
            registry.fail_every_mutation();
        } else {
            registry.fail_next_readback();
        }

        assert_eq!(
            set_autostart_transactional_with_registry(
                &store,
                &Mutex::new(()),
                &registry,
                exe,
                true,
                false,
            )
            .unwrap_err(),
            "autostart_registry_update_failed"
        );
        assert_eq!(store.load().unwrap(), before);
        assert_eq!(registry.value(), None);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn device_write_failure_restores_the_exact_prior_registry_value() {
    let root = temp_root("device-failure");
    let store = Arc::new(DeviceSettingsStore::open(root.clone()).unwrap());
    let initial = store
        .compare_and_swap(0, |settings| {
            settings.onboarding_step = 1;
            Ok(())
        })
        .unwrap();
    let old_command = r#""C:\Old\Emily.exe" --autostart"#;
    let registry = FakeRegistry::with_value(Some(old_command));
    let concurrent = Arc::clone(&store);
    registry.after_first_mutation(move || {
        concurrent
            .compare_and_swap(initial.revision, |settings| {
                settings.first_tray_hint_shown = true;
                Ok(())
            })
            .unwrap();
    });

    assert_eq!(
        set_autostart_transactional_with_registry(
            &store,
            &Mutex::new(()),
            &registry,
            Path::new(r"C:\New\Emily.exe"),
            true,
            false,
        )
        .unwrap_err(),
        "device_settings_revision_conflict"
    );
    assert_eq!(registry.value().as_deref(), Some(old_command));
    let after = store.load().unwrap();
    assert!(!after.launch_at_startup);
    assert!(after.first_tray_hint_shown);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_compensation_reports_uncertain_state() {
    let root = temp_root("uncertain");
    let store = Arc::new(DeviceSettingsStore::open(root.clone()).unwrap());
    let initial = store.load().unwrap();
    let registry = FakeRegistry::default();
    let concurrent = Arc::clone(&store);
    registry.after_first_mutation(move || {
        concurrent
            .compare_and_swap(initial.revision, |settings| {
                settings.first_tray_hint_shown = true;
                Ok(())
            })
            .unwrap();
    });
    registry.fail_mutation(2);

    assert_eq!(
        set_autostart_transactional_with_registry(
            &store,
            &Mutex::new(()),
            &registry,
            Path::new(r"C:\Emily\Emily.exe"),
            true,
            false,
        )
        .unwrap_err(),
        "autostart_state_uncertain"
    );
    assert_eq!(
        registry.value(),
        Some(r#""C:\Emily\Emily.exe" --autostart"#.to_string())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn startup_repair_follows_device_settings_in_both_directions() {
    let exe = Path::new(r"C:\Current\Emily.exe");

    let enabled_root = temp_root("repair-enabled");
    let enabled_store = DeviceSettingsStore::open(enabled_root.clone()).unwrap();
    enabled_store
        .compare_and_swap(0, |settings| {
            settings.launch_at_startup = true;
            settings.silent_autostart = true;
            Ok(())
        })
        .unwrap();
    let enabled_registry = FakeRegistry::with_value(Some(r#""C:\Stale\Emily.exe" --autostart"#));
    let repaired = repair_autostart_on_startup_with_registry(
        &enabled_store,
        &Mutex::new(()),
        &enabled_registry,
        exe,
    )
    .unwrap();
    assert!(repaired.enabled);
    assert!(repaired.executable_matches);
    assert_eq!(repaired.command, Some(expected_run_command(exe).unwrap()));

    let disabled_root = temp_root("repair-disabled");
    let disabled_store = DeviceSettingsStore::open(disabled_root.clone()).unwrap();
    let disabled_registry = FakeRegistry::with_value(Some(r#""C:\Stale\Emily.exe" --autostart"#));
    let repaired = repair_autostart_on_startup_with_registry(
        &disabled_store,
        &Mutex::new(()),
        &disabled_registry,
        exe,
    )
    .unwrap();
    assert!(!repaired.enabled);
    assert!(!repaired.executable_matches);
    assert_eq!(repaired.command, None);

    fs::remove_dir_all(enabled_root).unwrap();
    fs::remove_dir_all(disabled_root).unwrap();
}

#[test]
fn inspect_reports_the_exact_registry_command() {
    let exe = Path::new(r"C:\Emily\Emily.exe");
    let registry = FakeRegistry::with_value(Some(r#""C:\Other\Emily.exe" --autostart"#));
    let state = inspect_autostart_with_registry(&registry, exe).unwrap();
    assert!(state.enabled);
    assert!(!state.executable_matches);
    assert_eq!(
        state.command.as_deref(),
        Some(r#""C:\Other\Emily.exe" --autostart"#)
    );
}

struct RegistryCleanup(HkcuRunRegistry);

impl Drop for RegistryCleanup {
    fn drop(&mut self) {
        let _ = self.0.delete_value();
    }
}

#[test]
#[ignore = "mutates one unique HKCU Run test value"]
fn real_hkcu_round_trip() {
    let name = format!(
        "LifeArchiveProject.Emily.Test.{}",
        uuid::Uuid::new_v4().simple()
    );
    let registry = HkcuRunRegistry::with_value_name(name).unwrap();
    let _cleanup = RegistryCleanup(registry.clone());
    registry.delete_value().unwrap();
    assert_eq!(registry.read_value().unwrap(), None);
    registry.write_value("emily-autostart-test").unwrap();
    assert_eq!(
        registry.read_value().unwrap().as_deref(),
        Some("emily-autostart-test")
    );
    registry.delete_value().unwrap();
    assert_eq!(registry.read_value().unwrap(), None);
}
