use douyin_history_archive::{
    process_lock::{BootstrapProcessGuard, GuiInstanceGuard},
    runtime_paths::RuntimePaths,
    AppPaths,
};
use std::{
    fs,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn temp_root(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "emily-runtime-paths-{label}-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn app_paths_distinguish_bootstrap_data_and_resource_roots() {
    let root = temp_root("roots");
    let bootstrap = root.join("bootstrap");
    let data = root.join("data");
    let resources = root.join("resources");
    let paths = AppPaths::from_roots(bootstrap.clone(), data.clone(), resources.clone());

    assert_eq!(paths.bootstrap_dir, bootstrap);
    assert_eq!(paths.data_root, data);
    assert_eq!(paths.app_data, paths.data_root);
    assert_eq!(paths.resource_dir, resources);
    assert_eq!(paths.database, paths.data_root.join("history.sqlite"));
    assert_eq!(paths.covers, paths.data_root.join("covers"));
    assert_eq!(paths.backups, paths.data_root.join("backups"));
    assert_eq!(paths.downloads, paths.data_root.join("downloads"));
}

#[test]
fn maintenance_waits_for_existing_leases_and_blocks_new_access() {
    let root = temp_root("maintenance");
    let old_paths = AppPaths::new(root.join("old"), root.join("resources"));
    let new_paths = AppPaths::new(root.join("new"), root.join("resources"));
    let runtime = RuntimePaths::new(old_paths.clone());
    let existing = runtime.acquire().unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker = runtime.clone();

    let thread = thread::spawn(move || {
        started_tx.send(()).unwrap();
        let maintenance = worker.begin_maintenance().unwrap();
        ready_tx.send(maintenance).unwrap();
    });
    started_rx.recv().unwrap();
    for _ in 0..100 {
        if runtime.maintenance_active().unwrap() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert!(runtime.maintenance_active().unwrap());
    assert_eq!(
        runtime.acquire().unwrap_err(),
        "data_root_migration_in_progress"
    );
    assert!(ready_rx.try_recv().is_err());
    drop(existing);

    let maintenance = ready_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(maintenance.current_paths().data_root, old_paths.data_root);
    maintenance.commit(new_paths.clone()).unwrap();
    thread.join().unwrap();
    assert_eq!(runtime.acquire().unwrap().data_root, new_paths.data_root);
}

#[test]
fn dropping_maintenance_restores_the_old_runtime_path() {
    let root = temp_root("abort");
    let paths = AppPaths::new(root.join("old"), root.join("resources"));
    let runtime = RuntimePaths::new(paths.clone());

    let maintenance = runtime.begin_maintenance().unwrap();
    assert_eq!(
        runtime.acquire().unwrap_err(),
        "data_root_migration_in_progress"
    );
    drop(maintenance);

    assert!(!runtime.maintenance_active().unwrap());
    assert_eq!(runtime.acquire().unwrap().data_root, paths.data_root);
}

#[cfg(target_os = "windows")]
#[test]
fn windows_gui_mutex_allows_only_one_holder() {
    let unique = format!(
        "Local\\LifeArchiveProject.Emily.Test.Gui.{}.{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let first = GuiInstanceGuard::acquire_named(&unique).unwrap();
    assert_eq!(
        GuiInstanceGuard::acquire_named(&unique).unwrap_err(),
        "gui_instance_already_running"
    );
    drop(first);
    GuiInstanceGuard::acquire_named(&unique).unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn bootstrap_mutex_is_stable_per_normalized_path() {
    let root = temp_root("bootstrap-lock");
    let first = BootstrapProcessGuard::acquire(&root).unwrap();
    let alternate_spelling = PathBuf::from(root.to_string_lossy().to_uppercase());
    assert_eq!(
        BootstrapProcessGuard::acquire(&alternate_spelling).unwrap_err(),
        "bootstrap_locked"
    );
    BootstrapProcessGuard::acquire(&root.join("other")).unwrap();
    drop(first);
    BootstrapProcessGuard::acquire(&root).unwrap();
}
