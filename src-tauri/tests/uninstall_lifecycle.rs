#[path = "../build_support/package_identity.rs"]
mod package_identity;

use douyin_history_archive::{
    bootstrap::{write_bootstrap_atomic, BootstrapV1, APPLICATION_ID},
    uninstall::{
        abort_uninstall_before_commit_with, authenticate_uninstall_guard,
        canonical_package_manifest_bytes, compiled_product_identity,
        complete_repair_with_candidate_manifest, complete_uninstall_with, package_manifest_sha256,
        parse_early_process_mode, prepare_uninstall_session_with,
        read_embedded_executable_product_identity, recover_uninstall_state_before_bootstrap_with,
        run_installer_control_with, run_package_manifest_builder, supervise_uninstall_guard_with,
        transition_uninstall_to_destructive_with, uninstall_coordination_dir,
        validate_installer_object_removals, validate_manifest_delete_targets, validate_takeover,
        verify_package_manifest_files, DeviceAutostartSnapshotV1, EarlyProcessModeV1,
        ExecutableIdentityV1, GuardAuthenticationV1, GuardPrepareRequestV1,
        InstallerAuthenticationArgsV1, InstallerControlCommandV1, InstallerControlRuntime,
        InstallerGuardProcessV1, InstallerOwnedObjectV1, ManifestVerificationModeV1,
        PackageManifestEntryV1, PackageManifestV1, ProductIdentityV1, TakeoverActionV1,
        UninstallCompletionV1, UninstallGuardHandshake, UninstallGuardWaitOutcomeV1,
        UninstallPhaseV1, UninstallPrepareRequestV1, UninstallRecoveryOutcomeV1,
        UninstallSessionV1, UninstallSystem, UserDataDispositionV1, ARCHITECTURE,
        AUTOSTART_CLEANUP_FAULT_FILE, COMPILED_BUILD_ID, INSTALLER_SESSION_ARGS_FILE,
        PACKAGE_MANIFEST_FILE, PRODUCT_NAME, UNINSTALL_INTERRUPTED_FAULT_FILE,
        UNINSTALL_SESSION_FILE,
    },
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);
const ROOT_ID: &str = "0123456789abcdef0123456789abcdef";
const NONCE: &str = "fedcba9876543210fedcba9876543210";
const INSTALLER_PID: u32 = 4242;
const INSTALLER_CREATED_AT: u64 = 9_876_543_210;
const OLD_BUILD_ID: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const NEW_BUILD_ID: &str = "2222222222222222222222222222222222222222222222222222222222222222";

struct Fixture {
    root: PathBuf,
    install_dir: PathBuf,
    coordination_dir: PathBuf,
    data_dir: PathBuf,
    manifest: PackageManifestV1,
    manifest_bytes: Vec<u8>,
    manifest_hash: String,
    executable: ExecutableIdentityV1,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = fixture_root(label);
        let install_dir = root.join("install");
        let coordination_dir = root.join("coordination");
        let data_dir = install_dir.join("Data");
        fs::create_dir_all(install_dir.join("assets")).unwrap();
        fs::create_dir_all(&coordination_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(install_dir.join("Emily.exe"), b"exe-v1").unwrap();
        fs::write(install_dir.join("assets/app.js"), b"app-v1").unwrap();
        fs::write(data_dir.join("history.sqlite"), b"user-data").unwrap();

        let product = product(OLD_BUILD_ID, "0.1.0");
        let manifest = PackageManifestV1 {
            schema_version: 1,
            application_id: product.application_id.clone(),
            product_name: product.product_name.clone(),
            build_id: product.build_id.clone(),
            semantic_version: product.semantic_version.clone(),
            architecture: product.architecture.clone(),
            files: vec![
                entry(&install_dir, "Emily.exe"),
                entry(&install_dir, "assets/app.js"),
            ],
        };
        let manifest_bytes = canonical_package_manifest_bytes(&manifest).unwrap();
        let manifest_hash = package_manifest_sha256(&manifest_bytes);
        let executable = ExecutableIdentityV1 {
            install_dir: install_dir.clone(),
            main_executable: install_dir.join("Emily.exe"),
            product,
        };
        Self {
            root,
            install_dir,
            coordination_dir,
            data_dir,
            manifest,
            manifest_bytes,
            manifest_hash,
            executable,
        }
    }

    fn request(&self) -> UninstallPrepareRequestV1 {
        UninstallPrepareRequestV1 {
            authentication: authentication(),
            root_id: ROOT_ID.to_string(),
            executable_identity: self.executable.clone(),
            device_settings_dir: self.root.join("bootstrap"),
            protected_paths: vec![self.coordination_dir.clone(), self.data_dir.clone()],
            user_data_disposition: UserDataDispositionV1::Retain,
            package_manifest_bytes: self.manifest_bytes.clone(),
            expected_package_manifest_sha256: self.manifest_hash.clone(),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct FakeSystem {
    run_value: RefCell<Option<String>>,
    device: RefCell<DeviceAutostartSnapshotV1>,
    process_times: RefCell<BTreeMap<u32, u64>>,
    executable_products: RefCell<BTreeMap<String, ProductIdentityV1>>,
    events: RefCell<Vec<String>>,
    journal_path: RefCell<Option<PathBuf>>,
    require_journal_before_mutation: Cell<bool>,
    fail_next_run_write: Cell<bool>,
    device_write_count: Cell<usize>,
    fail_device_write_at: Cell<Option<usize>>,
}

struct FakeGuardHandshake {
    outcome: UninstallGuardWaitOutcomeV1,
    ready: Cell<bool>,
}

struct FakeInstallerRuntime<'a> {
    system: &'a FakeSystem,
    guard_started: Cell<bool>,
    guard_finished: Cell<bool>,
    guard_aborted: Cell<bool>,
    destructive_commit_prepared: Cell<bool>,
    fail_destructive_commit_prepare: Cell<bool>,
    deleted_bootstrap: RefCell<Vec<PathBuf>>,
}

impl<'a> FakeInstallerRuntime<'a> {
    fn new(system: &'a FakeSystem) -> Self {
        Self {
            system,
            guard_started: Cell::new(false),
            guard_finished: Cell::new(false),
            guard_aborted: Cell::new(false),
            destructive_commit_prepared: Cell::new(false),
            fail_destructive_commit_prepare: Cell::new(false),
            deleted_bootstrap: RefCell::new(Vec::new()),
        }
    }
}

impl InstallerControlRuntime for FakeInstallerRuntime<'_> {
    fn start_guard(
        &self,
        coordination_dir: &Path,
        _authentication: &GuardAuthenticationV1,
        _expected_guard_product: &ProductIdentityV1,
        prepare_request: Option<&GuardPrepareRequestV1>,
    ) -> Result<InstallerGuardProcessV1, String> {
        if let Some(request) = prepare_request {
            prepare_uninstall_session_with(
                coordination_dir,
                request.clone().into_prepare_request()?,
                self.system,
            )?;
        }
        self.guard_started.set(true);
        Ok(InstallerGuardProcessV1 {
            control_executable: coordination_dir.join("controller.exe"),
            guard_executable: coordination_dir.join("guard.exe"),
        })
    }

    fn assert_guard_alive(&self, _authentication: &GuardAuthenticationV1) -> Result<(), String> {
        self.guard_started
            .get()
            .then_some(())
            .ok_or_else(|| "uninstall_guard_not_ready".to_string())
    }

    fn prepare_for_destructive_commit(&self, _session: &UninstallSessionV1) -> Result<(), String> {
        if self.fail_destructive_commit_prepare.get() {
            return Err("uninstall_runtime_cleanup_failed".to_string());
        }
        self.destructive_commit_prepared.set(true);
        Ok(())
    }

    fn signal_guard_finished(
        &self,
        _coordination_dir: &Path,
        _authentication: &GuardAuthenticationV1,
    ) -> Result<(), String> {
        self.guard_finished.set(true);
        Ok(())
    }

    fn signal_guard_aborted(
        &self,
        _coordination_dir: &Path,
        _authentication: &GuardAuthenticationV1,
    ) -> Result<(), String> {
        self.guard_aborted.set(true);
        Ok(())
    }

    fn delete_managed_data(&self, bootstrap_dir: &Path) -> Result<(), String> {
        self.deleted_bootstrap
            .borrow_mut()
            .push(bootstrap_dir.to_path_buf());
        Ok(())
    }
}

impl FakeGuardHandshake {
    fn new(outcome: UninstallGuardWaitOutcomeV1) -> Self {
        Self {
            outcome,
            ready: Cell::new(false),
        }
    }
}

impl UninstallGuardHandshake for FakeGuardHandshake {
    fn signal_ready(&self) -> Result<(), String> {
        self.ready.set(true);
        Ok(())
    }

    fn wait_for_terminal(&self) -> Result<UninstallGuardWaitOutcomeV1, String> {
        Ok(self.outcome)
    }
}

impl FakeSystem {
    fn ready(fixture: &Fixture) -> Self {
        let mut process_times = BTreeMap::new();
        process_times.insert(INSTALLER_PID, INSTALLER_CREATED_AT);
        let mut executable_products = BTreeMap::new();
        executable_products.insert(
            normalize(&fixture.executable.main_executable),
            fixture.executable.product.clone(),
        );
        Self {
            run_value: RefCell::new(Some("old-run-command".to_string())),
            device: RefCell::new(DeviceAutostartSnapshotV1 {
                launch_at_startup: true,
                silent_autostart: true,
            }),
            process_times: RefCell::new(process_times),
            executable_products: RefCell::new(executable_products),
            events: RefCell::new(Vec::new()),
            journal_path: RefCell::new(Some(fixture.coordination_dir.join(UNINSTALL_SESSION_FILE))),
            require_journal_before_mutation: Cell::new(true),
            fail_next_run_write: Cell::new(false),
            device_write_count: Cell::new(0),
            fail_device_write_at: Cell::new(None),
        }
    }

    fn record_mutation(&self, event: String) -> Result<(), String> {
        if self.require_journal_before_mutation.get() {
            let journal = self
                .journal_path
                .borrow()
                .clone()
                .ok_or_else(|| "test_journal_path_missing".to_string())?;
            if !journal.is_file() {
                return Err("journal_not_durable_before_mutation".to_string());
            }
        }
        self.events.borrow_mut().push(event);
        Ok(())
    }
}

impl UninstallSystem for FakeSystem {
    fn read_run_value(&self) -> Result<Option<String>, String> {
        Ok(self.run_value.borrow().clone())
    }

    fn write_run_value(&self, value: Option<&str>) -> Result<(), String> {
        self.record_mutation(format!("run:{}", value.unwrap_or("<none>")))?;
        if self.fail_next_run_write.replace(false) {
            return Err("run_write_failed".to_string());
        }
        *self.run_value.borrow_mut() = value.map(str::to_string);
        Ok(())
    }

    fn read_device_autostart(
        &self,
        _coordination_dir: &Path,
    ) -> Result<DeviceAutostartSnapshotV1, String> {
        Ok(self.device.borrow().clone())
    }

    fn write_device_autostart(
        &self,
        _coordination_dir: &Path,
        value: &DeviceAutostartSnapshotV1,
    ) -> Result<(), String> {
        self.record_mutation(format!(
            "device:{},{}",
            value.launch_at_startup, value.silent_autostart
        ))?;
        let call = self.device_write_count.get() + 1;
        self.device_write_count.set(call);
        if self.fail_device_write_at.get() == Some(call) {
            self.fail_device_write_at.set(None);
            return Err("device_write_failed".to_string());
        }
        *self.device.borrow_mut() = value.clone();
        Ok(())
    }

    fn process_creation_time_100ns(&self, pid: u32) -> Result<Option<u64>, String> {
        Ok(self.process_times.borrow().get(&pid).copied())
    }

    fn executable_product_identity(
        &self,
        path: &Path,
    ) -> Result<Option<ProductIdentityV1>, String> {
        Ok(self
            .executable_products
            .borrow()
            .get(&normalize(path))
            .cloned())
    }
}

#[test]
fn manifest_contract_rejects_unsafe_case_colliding_and_unknown_fields() {
    let fixture = Fixture::new("manifest-contract");
    let parsed: PackageManifestV1 = serde_json::from_slice(&fixture.manifest_bytes).unwrap();
    assert_eq!(parsed, fixture.manifest);
    assert_eq!(
        canonical_package_manifest_bytes(&parsed).unwrap(),
        fixture.manifest_bytes
    );

    for unsafe_path in [
        "C:/absolute.exe",
        "../escape.exe",
        "assets\\app.js",
        "assets//app.js",
        "./Emily.exe",
        "assets/app.js:stream",
        "CON",
        "assets/NUL.txt",
        "assets/trailing. ",
    ] {
        let mut invalid = fixture.manifest.clone();
        invalid.files[0].path = unsafe_path.to_string();
        assert!(
            canonical_package_manifest_bytes(&invalid).is_err(),
            "{unsafe_path}"
        );
    }

    let mut case_collision = fixture.manifest.clone();
    case_collision.files.push(PackageManifestEntryV1 {
        path: "emily.EXE".to_string(),
        bytes: 1,
        sha256: "a".repeat(64),
    });
    assert!(canonical_package_manifest_bytes(&case_collision).is_err());

    let mut unknown = serde_json::to_value(&fixture.manifest).unwrap();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<PackageManifestV1>(unknown).is_err());

    let mut invalid_build = fixture.manifest.clone();
    invalid_build.build_id = "build-old".to_string();
    assert_eq!(
        canonical_package_manifest_bytes(&invalid_build),
        Err("package_identity_invalid".to_string())
    );
}

#[test]
fn manifest_verification_is_phase_specific_and_refuses_tampering() {
    let fixture = Fixture::new("manifest-verification");
    verify_package_manifest_files(
        &fixture.manifest,
        &fixture.install_dir,
        ManifestVerificationModeV1::RequireAll,
    )
    .unwrap();

    fs::write(fixture.install_dir.join("assets/app.js"), b"tampered").unwrap();
    assert_eq!(
        verify_package_manifest_files(
            &fixture.manifest,
            &fixture.install_dir,
            ManifestVerificationModeV1::RequireAll,
        ),
        Err("package_manifest_file_mismatch".to_string())
    );

    fs::write(fixture.install_dir.join("assets/app.js"), b"app-v1").unwrap();
    fs::remove_file(fixture.install_dir.join("Emily.exe")).unwrap();
    assert!(verify_package_manifest_files(
        &fixture.manifest,
        &fixture.install_dir,
        ManifestVerificationModeV1::RequireAll,
    )
    .is_err());
    verify_package_manifest_files(
        &fixture.manifest,
        &fixture.install_dir,
        ManifestVerificationModeV1::AllowMissing,
    )
    .unwrap();

    let mut tampered_bytes = fixture.manifest_bytes.clone();
    let version = b"0.1.0";
    let replacement = b"0.1.1";
    let offset = tampered_bytes
        .windows(version.len())
        .position(|window| window == version)
        .unwrap();
    tampered_bytes[offset..offset + version.len()].copy_from_slice(replacement);
    let mut request = fixture.request();
    request.package_manifest_bytes = tampered_bytes;
    assert_eq!(
        prepare_uninstall_session_with(
            &fixture.coordination_dir,
            request,
            &FakeSystem::ready(&fixture),
        ),
        Err("package_manifest_hash_mismatch".to_string())
    );
}

#[cfg(target_os = "windows")]
#[test]
fn manifest_verification_rejects_windows_reparse_components() {
    use std::os::windows::fs::symlink_dir;

    let fixture = Fixture::new("manifest-reparse");
    let outside = fixture.root.join("outside-assets");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("app.js"), b"app-v1").unwrap();
    fs::remove_dir_all(fixture.install_dir.join("assets")).unwrap();
    if symlink_dir(&outside, fixture.install_dir.join("assets")).is_err() {
        return;
    }

    assert_eq!(
        verify_package_manifest_files(
            &fixture.manifest,
            &fixture.install_dir,
            ManifestVerificationModeV1::RequireAll,
        ),
        Err("package_manifest_reparse_point".to_string())
    );
}

#[test]
fn deletion_requires_manifest_ownership_and_preserves_protected_paths() {
    let fixture = Fixture::new("manifest-deletion");
    validate_manifest_delete_targets(
        &fixture.manifest,
        &fixture.install_dir,
        &[fixture.coordination_dir.clone(), fixture.data_dir.clone()],
        &[
            fixture.install_dir.join("Emily.exe"),
            fixture.install_dir.join("assets/app.js"),
        ],
    )
    .unwrap();

    for invalid in [
        fixture.install_dir.join("extra.log"),
        fixture.install_dir.clone(),
        fixture.data_dir.join("history.sqlite"),
        fixture.install_dir.join("assets/../Emily.exe"),
    ] {
        assert!(validate_manifest_delete_targets(
            &fixture.manifest,
            &fixture.install_dir,
            &[fixture.coordination_dir.clone(), fixture.data_dir.clone()],
            &[invalid],
        )
        .is_err());
    }

    let objects = vec![
        InstallerOwnedObjectV1::StartMenuShortcut {
            path: PathBuf::from(
                r"C:\Users\Test\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Fy1DouyinArchiver.lnk",
            ),
        },
        InstallerOwnedObjectV1::UninstallRegistryValue {
            key: r"Software\Microsoft\Windows\CurrentVersion\Uninstall\lifearchive.douyin-history-archive".to_string(),
            value_name: "DisplayName".to_string(),
        },
        InstallerOwnedObjectV1::SelfUninstaller {
            path: fixture.install_dir.join("uninstall.exe"),
        },
    ];
    assert!(validate_installer_object_removals(
        UninstallPhaseV1::Prepared,
        &fixture.install_dir,
        &objects,
    )
    .is_err());
    validate_installer_object_removals(
        UninstallPhaseV1::DestructiveCommitted,
        &fixture.install_dir,
        &objects,
    )
    .unwrap();

    for invalid in [
        InstallerOwnedObjectV1::StartMenuShortcut {
            path: PathBuf::from(r"C:\Users\Test\Desktop\Other.lnk"),
        },
        InstallerOwnedObjectV1::UninstallRegistryValue {
            key: r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Other".to_string(),
            value_name: "DisplayName".to_string(),
        },
        InstallerOwnedObjectV1::UninstallRegistryValue {
            key: r"Software\Microsoft\Windows\CurrentVersion\Uninstall\lifearchive.douyin-history-archive".to_string(),
            value_name: "ArbitraryValue".to_string(),
        },
        InstallerOwnedObjectV1::SelfUninstaller {
            path: fixture.install_dir.join("other.exe"),
        },
    ] {
        assert_eq!(
            validate_installer_object_removals(
                UninstallPhaseV1::DestructiveCommitted,
                &fixture.install_dir,
                &[invalid],
            ),
            Err("installer_object_not_whitelisted".to_string())
        );
    }
}

#[test]
fn uninstall_identity_and_protected_paths_reject_dot_component_escapes() {
    let fixture = Fixture::new("path-escape");
    let system = FakeSystem::ready(&fixture);

    let mut escaped_executable = fixture.request();
    escaped_executable.executable_identity.main_executable =
        fixture.install_dir.join("assets/../Emily.exe");
    assert_eq!(
        prepare_uninstall_session_with(&fixture.coordination_dir, escaped_executable, &system,),
        Err("uninstall_executable_identity_invalid".to_string())
    );

    let mut escaped_protected = fixture.request();
    escaped_protected.protected_paths = vec![
        fixture.coordination_dir.join("nested/.."),
        fixture.data_dir.clone(),
    ];
    assert_eq!(
        prepare_uninstall_session_with(&fixture.coordination_dir, escaped_protected, &system,),
        Err("uninstall_protected_paths_invalid".to_string())
    );
}

#[test]
fn package_manifest_builder_uses_only_explicit_staging_entries() {
    let fixture = Fixture::new("manifest-builder");
    let staging = fixture.root.join("staging");
    fs::create_dir_all(staging.join("assets")).unwrap();
    fs::write(staging.join("Emily.exe"), b"exe-build").unwrap();
    fs::write(staging.join("assets/app.js"), b"app-build").unwrap();
    fs::write(staging.join("unlisted.bin"), b"must-not-appear").unwrap();
    let request = fixture.root.join("request.json");
    let output = fixture.root.join(PACKAGE_MANIFEST_FILE);
    let hash_output = fixture.root.join("manifest.sha256");
    let build_id_output = fixture.root.join("build-id.sha256");
    fs::write(
        &request,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 1,
            "stagingRoot": staging,
            "entries": [
                {"source": fixture.root.join("staging/Emily.exe"), "destination": "Emily.exe"},
                {"source": fixture.root.join("staging/assets/app.js"), "destination": "assets/app.js"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    run_package_manifest_builder(&[
        OsString::from("--response-file"),
        request.clone().into_os_string(),
        OsString::from("--manifest-output"),
        output.clone().into_os_string(),
        OsString::from("--hash-output"),
        hash_output.clone().into_os_string(),
        OsString::from("--build-id-output"),
        build_id_output.clone().into_os_string(),
    ])
    .unwrap();
    let manifest: PackageManifestV1 = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["Emily.exe", "assets/app.js"]
    );
    assert!(!fs::read_to_string(&output)
        .unwrap()
        .contains("unlisted.bin"));
    assert_eq!(
        fs::read_to_string(hash_output).unwrap(),
        package_manifest_sha256(&fs::read(output).unwrap())
    );
    assert_eq!(
        fs::read_to_string(build_id_output).unwrap(),
        COMPILED_BUILD_ID
    );

    let mut invalid: serde_json::Value =
        serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    invalid["entries"].as_array_mut().unwrap().reverse();
    fs::write(&request, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
    assert!(run_package_manifest_builder(&[
        OsString::from("--response-file"),
        request.into_os_string(),
        OsString::from("--manifest-output"),
        fixture.root.join("invalid.json").into_os_string(),
        OsString::from("--hash-output"),
        fixture.root.join("invalid.sha256").into_os_string(),
        OsString::from("--build-id-output"),
        fixture.root.join("invalid.build-id").into_os_string(),
    ])
    .is_err());
}

#[test]
fn package_manifest_builder_rejects_response_and_output_path_aliases() {
    let fixture = Fixture::new("manifest-builder-output-alias");
    let staging = fixture.root.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let source = staging.join("Emily.exe");
    fs::write(&source, b"exe-build").unwrap();
    let request = fixture.root.join("request.json");
    let hash_output = fixture.root.join("manifest.sha256");
    let build_id_output = fixture.root.join("build-id.sha256");
    fs::write(
        &request,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "stagingRoot": staging,
            "entries": [
                {"source": source, "destination": "Emily.exe"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        run_package_manifest_builder(&[
            OsString::from("--response-file"),
            request.clone().into_os_string(),
            OsString::from("--manifest-output"),
            request.into_os_string(),
            OsString::from("--hash-output"),
            hash_output.into_os_string(),
            OsString::from("--build-id-output"),
            build_id_output.clone().into_os_string(),
        ]),
        Err("package_manifest_builder_path_invalid".to_string())
    );

    let request = fixture.root.join("request-inside-output.json");
    fs::write(
        &request,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "stagingRoot": staging,
            "entries": [
                {"source": source, "destination": "Emily.exe"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        run_package_manifest_builder(&[
            OsString::from("--response-file"),
            request.into_os_string(),
            OsString::from("--manifest-output"),
            staging.join("manifest.json").into_os_string(),
            OsString::from("--hash-output"),
            fixture.root.join("outside.sha256").into_os_string(),
            OsString::from("--build-id-output"),
            build_id_output.clone().into_os_string(),
        ]),
        Err("package_manifest_builder_output_inside_staging".to_string())
    );
}

#[test]
fn package_manifest_builder_rejects_sources_outside_staging_and_duplicate_sources() {
    let fixture = Fixture::new("manifest-builder-source-boundary");
    let staging = fixture.root.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let inside = staging.join("Emily.exe");
    let outside = fixture.root.join("outside.exe");
    fs::write(&inside, b"inside").unwrap();
    fs::write(&outside, b"outside").unwrap();
    let request = fixture.root.join("request.json");
    let output = fixture.root.join("manifest.json");
    let hash_output = fixture.root.join("manifest.sha256");
    let build_id_output = fixture.root.join("build-id.sha256");

    fs::write(
        &request,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "stagingRoot": staging,
            "entries": [
                {"source": outside, "destination": "Emily.exe"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let arguments = [
        OsString::from("--response-file"),
        request.clone().into_os_string(),
        OsString::from("--manifest-output"),
        output.clone().into_os_string(),
        OsString::from("--hash-output"),
        hash_output.clone().into_os_string(),
        OsString::from("--build-id-output"),
        build_id_output.into_os_string(),
    ];
    assert_eq!(
        run_package_manifest_builder(&arguments),
        Err("package_manifest_source_outside_staging".to_string())
    );

    fs::write(
        &request,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "stagingRoot": staging,
            "entries": [
                {"source": inside, "destination": "Emily.exe"},
                {"source": inside, "destination": "assets/app.js"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        run_package_manifest_builder(&arguments),
        Err("package_manifest_source_duplicate".to_string())
    );
}

#[test]
fn prepare_is_durable_before_device_then_registry_and_abort_reverses_order() {
    let fixture = Fixture::new("prepare-abort");
    let system = FakeSystem::ready(&fixture);
    let session =
        prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system)
            .unwrap();
    assert_eq!(session.phase, UninstallPhaseV1::Prepared);
    assert_eq!(
        system.events.borrow().as_slice(),
        ["device:false,false", "run:<none>"]
    );
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .is_file());

    abort_uninstall_before_commit_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    assert_eq!(
        &system.events.borrow()[2..],
        ["run:old-run-command", "device:true,true"]
    );
    assert!(!fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());
}

#[test]
fn failed_compensation_persists_an_autostart_cleanup_fault() {
    let fixture = Fixture::new("compensation-fault");
    let system = FakeSystem::ready(&fixture);
    system.fail_next_run_write.set(true);
    system.fail_device_write_at.set(Some(2));
    assert!(
        prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system,)
            .is_err()
    );
    assert!(fixture
        .coordination_dir
        .join(AUTOSTART_CLEANUP_FAULT_FILE)
        .is_file());
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .is_file());
}

#[test]
fn uninstall_fault_files_are_strict_and_bound_to_the_session() {
    let fixture = Fixture::new("strict-faults");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    let cleanup_fault = fixture.coordination_dir.join(AUTOSTART_CLEANUP_FAULT_FILE);

    fs::write(
        &cleanup_fault,
        serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "nonce": NONCE,
            "phase": "prepared",
            "reason": "rollback_failed",
            "observedAt": "2026-08-12T00:00:00.000Z"
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &fixture.executable,
            &system,
        ),
        Err("uninstall_fault_invalid".to_string())
    );

    fs::write(
        &cleanup_fault,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "nonce": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "phase": "prepared",
            "reason": "rollback_failed",
            "observedAt": "2026-08-12T00:00:00.000Z"
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &fixture.executable,
            &system,
        ),
        Err("uninstall_fault_invalid".to_string())
    );
}

#[test]
fn cleanup_fault_recovery_keeps_autostart_disabled_and_removes_run() {
    let fixture = Fixture::new("cleanup-fault-recovery");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    let fault = fixture.coordination_dir.join(AUTOSTART_CLEANUP_FAULT_FILE);
    fs::write(
        &fault,
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "nonce": NONCE,
            "phase": "prepared",
            "reason": "rollback_failed",
            "observedAt": "2026-08-12T00:00:00.000Z"
        }))
        .unwrap(),
    )
    .unwrap();
    *system.run_value.borrow_mut() = Some("stale-run-command".to_string());
    *system.device.borrow_mut() = DeviceAutostartSnapshotV1 {
        launch_at_startup: true,
        silent_autostart: true,
    };
    system.events.borrow_mut().clear();

    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &fixture.executable,
            &system,
        ),
        Err("autostart_cleanup_fault".to_string())
    );
    assert_eq!(
        system.events.borrow().as_slice(),
        ["device:false,false", "run:<none>"]
    );
    assert_eq!(*system.run_value.borrow(), None);
    assert_eq!(
        *system.device.borrow(),
        DeviceAutostartSnapshotV1::default()
    );
    assert!(fault.exists());
}

#[test]
fn startup_recovery_rolls_back_prepared_and_fails_closed_after_commit() {
    let fixture = Fixture::new("startup-recovery");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    system.events.borrow_mut().clear();
    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &fixture.executable,
            &system,
        )
        .unwrap(),
        UninstallRecoveryOutcomeV1::RolledBack
    );
    assert_eq!(
        system.events.borrow().as_slice(),
        ["run:old-run-command", "device:true,true"]
    );

    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &fixture.executable,
            &system,
        ),
        Err("uninstall_repair_required".to_string())
    );
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_INTERRUPTED_FAULT_FILE)
        .is_file());
}

#[test]
fn startup_recovery_handles_preparing_and_both_terminal_phases_independently() {
    for (phase, expected) in [
        (
            UninstallPhaseV1::Preparing,
            UninstallRecoveryOutcomeV1::RolledBack,
        ),
        (
            UninstallPhaseV1::Completed,
            UninstallRecoveryOutcomeV1::TerminalCleaned,
        ),
        (
            UninstallPhaseV1::RepairCompleted,
            UninstallRecoveryOutcomeV1::TerminalCleaned,
        ),
    ] {
        let fixture = Fixture::new(&format!("recovery-{phase:?}"));
        let system = FakeSystem::ready(&fixture);
        prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system)
            .unwrap();
        let session_path = fixture.coordination_dir.join(UNINSTALL_SESSION_FILE);
        let mut session: serde_json::Value =
            serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
        session["phase"] = serde_json::to_value(phase).unwrap();
        fs::write(&session_path, serde_json::to_vec(&session).unwrap()).unwrap();
        system.events.borrow_mut().clear();

        assert_eq!(
            recover_uninstall_state_before_bootstrap_with(
                &fixture.coordination_dir,
                &fixture.executable,
                &system,
            )
            .unwrap(),
            expected
        );
        assert!(!session_path.exists());
        if phase == UninstallPhaseV1::Preparing {
            assert_eq!(
                system.events.borrow().as_slice(),
                ["run:old-run-command", "device:true,true"]
            );
        } else {
            assert!(system.events.borrow().is_empty());
        }
    }
}

#[test]
fn repair_completed_residue_can_be_cleaned_by_the_newer_executable_identity() {
    let fixture = Fixture::new("repair-terminal-new-identity");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    let session_path = fixture.coordination_dir.join(UNINSTALL_SESSION_FILE);
    let mut session: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
    session["phase"] = serde_json::to_value(UninstallPhaseV1::RepairCompleted).unwrap();
    fs::write(&session_path, serde_json::to_vec(&session).unwrap()).unwrap();
    let new_executable = ExecutableIdentityV1 {
        install_dir: fixture.install_dir.clone(),
        main_executable: fixture.install_dir.join("Emily.exe"),
        product: product(NEW_BUILD_ID, "0.2.0"),
    };
    fs::write(fixture.install_dir.join("Emily.exe"), b"exe-v2").unwrap();
    fs::write(fixture.install_dir.join("assets/app.js"), b"app-v2").unwrap();

    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &new_executable,
            &system,
        )
        .unwrap(),
        UninstallRecoveryOutcomeV1::TerminalCleaned
    );
    assert!(!session_path.exists());
}

#[test]
fn completed_residue_can_be_cleaned_after_same_path_reinstall() {
    let fixture = Fixture::new("completed-terminal-new-identity");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    let session_path = fixture.coordination_dir.join(UNINSTALL_SESSION_FILE);
    let mut session: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
    session["phase"] = serde_json::to_value(UninstallPhaseV1::Completed).unwrap();
    fs::write(&session_path, serde_json::to_vec(&session).unwrap()).unwrap();
    fs::write(fixture.install_dir.join("Emily.exe"), b"exe-v2").unwrap();
    fs::write(fixture.install_dir.join("assets/app.js"), b"app-v2").unwrap();
    let new_executable = ExecutableIdentityV1 {
        install_dir: fixture.install_dir.clone(),
        main_executable: fixture.install_dir.join("Emily.exe"),
        product: product(NEW_BUILD_ID, "0.2.0"),
    };

    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &new_executable,
            &system,
        )
        .unwrap(),
        UninstallRecoveryOutcomeV1::TerminalCleaned
    );
    assert!(!session_path.exists());
}

#[test]
fn guard_authentication_rejects_nonce_pid_reuse_and_invalid_transitions() {
    let fixture = Fixture::new("guard-authentication");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();

    let mut wrong_nonce = authentication();
    wrong_nonce.nonce = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    assert!(
        authenticate_uninstall_guard(&fixture.coordination_dir, &wrong_nonce, &system,).is_err()
    );

    system
        .process_times
        .borrow_mut()
        .insert(INSTALLER_PID, INSTALLER_CREATED_AT + 1);
    assert!(
        authenticate_uninstall_guard(&fixture.coordination_dir, &authentication(), &system,)
            .is_err()
    );
    system
        .process_times
        .borrow_mut()
        .insert(INSTALLER_PID, INSTALLER_CREATED_AT);

    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    assert!(abort_uninstall_before_commit_with(
        &fixture.coordination_dir,
        &authentication(),
        &system,
    )
    .is_err());
    fs::remove_file(fixture.install_dir.join("Emily.exe")).unwrap();
    fs::remove_file(fixture.install_dir.join("assets/app.js")).unwrap();
    complete_uninstall_with(
        &fixture.coordination_dir,
        &authentication(),
        UninstallCompletionV1::Uninstalled,
        &system,
    )
    .unwrap();
    assert!(!fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());
}

#[test]
fn uninstall_completion_requires_all_manifest_owned_files_to_be_removed() {
    let fixture = Fixture::new("uninstall-completion-files");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();

    assert_eq!(
        complete_uninstall_with(
            &fixture.coordination_dir,
            &authentication(),
            UninstallCompletionV1::Uninstalled,
            &system,
        ),
        Err("uninstall_package_files_remain".to_string())
    );
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());

    fs::remove_file(fixture.install_dir.join("Emily.exe")).unwrap();
    fs::remove_file(fixture.install_dir.join("assets/app.js")).unwrap();
    complete_uninstall_with(
        &fixture.coordination_dir,
        &authentication(),
        UninstallCompletionV1::Uninstalled,
        &system,
    )
    .unwrap();
    assert!(!fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());
}

#[test]
fn executable_identity_is_embedded_and_readable_without_executing_the_file() {
    assert_eq!(
        read_embedded_executable_product_identity(&std::env::current_exe().unwrap()).unwrap(),
        Some(compiled_product_identity())
    );

    let fixture = Fixture::new("missing-embedded-identity");
    let unrelated = fixture.root.join("unrelated.exe");
    fs::write(&unrelated, b"not an Emily executable").unwrap();
    assert_eq!(
        read_embedded_executable_product_identity(&unrelated).unwrap(),
        None
    );
}

#[test]
fn long_lived_guard_rolls_back_before_commit_and_fails_closed_after_commit() {
    let before = Fixture::new("guard-abort-before-commit");
    let before_system = FakeSystem::ready(&before);
    prepare_uninstall_session_with(&before.coordination_dir, before.request(), &before_system)
        .unwrap();
    before_system.events.borrow_mut().clear();
    let abort = FakeGuardHandshake::new(UninstallGuardWaitOutcomeV1::Aborted);
    assert_eq!(
        supervise_uninstall_guard_with(
            &before.coordination_dir,
            &authentication(),
            &before_system,
            &abort,
        ),
        Err("uninstall_aborted".to_string())
    );
    assert!(abort.ready.get());
    assert_eq!(
        before_system.events.borrow().as_slice(),
        ["run:old-run-command", "device:true,true"]
    );
    assert!(!before
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());

    let after = Fixture::new("guard-parent-exit-after-commit");
    let after_system = FakeSystem::ready(&after);
    prepare_uninstall_session_with(&after.coordination_dir, after.request(), &after_system)
        .unwrap();
    transition_uninstall_to_destructive_with(
        &after.coordination_dir,
        &authentication(),
        &after_system,
    )
    .unwrap();
    after_system.events.borrow_mut().clear();
    let exited = FakeGuardHandshake::new(UninstallGuardWaitOutcomeV1::InstallerExited);
    assert_eq!(
        supervise_uninstall_guard_with(
            &after.coordination_dir,
            &authentication(),
            &after_system,
            &exited,
        ),
        Err("uninstall_repair_required".to_string())
    );
    assert!(exited.ready.get());
    assert!(after_system.events.borrow().is_empty());
    assert!(after
        .coordination_dir
        .join(UNINSTALL_INTERRUPTED_FAULT_FILE)
        .is_file());
    assert!(after
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .is_file());
}

#[test]
fn takeover_allows_newer_repair_but_rejects_identity_and_policy_changes() {
    let fixture = Fixture::new("takeover");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    let session = transition_uninstall_to_destructive_with(
        &fixture.coordination_dir,
        &authentication(),
        &system,
    )
    .unwrap();

    let candidate = ExecutableIdentityV1 {
        install_dir: fixture.install_dir.clone(),
        main_executable: fixture.install_dir.join("Emily.exe"),
        product: product(NEW_BUILD_ID, "0.2.0"),
    };
    validate_takeover(
        &session,
        &candidate,
        Some(&fixture.executable.product),
        ROOT_ID,
        &[fixture.coordination_dir.clone(), fixture.data_dir.clone()],
        UserDataDispositionV1::Retain,
        TakeoverActionV1::Repair,
    )
    .unwrap();

    for changed in [
        ProductIdentityV1 {
            application_id: "other.application".to_string(),
            ..candidate.product.clone()
        },
        ProductIdentityV1 {
            product_name: "Other Product".to_string(),
            ..candidate.product.clone()
        },
        ProductIdentityV1 {
            architecture: "windows-arm64".to_string(),
            ..candidate.product.clone()
        },
    ] {
        let invalid = ExecutableIdentityV1 {
            product: changed,
            ..candidate.clone()
        };
        assert!(validate_takeover(
            &session,
            &invalid,
            Some(&fixture.executable.product),
            ROOT_ID,
            &[fixture.coordination_dir.clone(), fixture.data_dir.clone()],
            UserDataDispositionV1::Retain,
            TakeoverActionV1::Repair,
        )
        .is_err());
    }
    assert!(validate_takeover(
        &session,
        &candidate,
        Some(&fixture.executable.product),
        ROOT_ID,
        &[fixture.coordination_dir.clone()],
        UserDataDispositionV1::Retain,
        TakeoverActionV1::Repair,
    )
    .is_err());
    assert!(validate_takeover(
        &session,
        &candidate,
        Some(&fixture.executable.product),
        ROOT_ID,
        &[fixture.coordination_dir.clone(), fixture.data_dir.clone()],
        UserDataDispositionV1::Delete,
        TakeoverActionV1::Repair,
    )
    .is_err());
}

#[test]
fn takeover_supports_continue_and_same_or_cross_version_repair_only_with_old_identity() {
    let fixture = Fixture::new("takeover-matrix");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    let session = transition_uninstall_to_destructive_with(
        &fixture.coordination_dir,
        &authentication(),
        &system,
    )
    .unwrap();
    let protected = [fixture.coordination_dir.clone(), fixture.data_dir.clone()];

    for (action, build_id, version) in [
        (TakeoverActionV1::ContinueUninstall, OLD_BUILD_ID, "0.1.0"),
        (TakeoverActionV1::Repair, OLD_BUILD_ID, "0.1.0"),
        (TakeoverActionV1::Repair, NEW_BUILD_ID, "0.2.0"),
    ] {
        let candidate = ExecutableIdentityV1 {
            install_dir: fixture.install_dir.clone(),
            main_executable: fixture.install_dir.join("Emily.exe"),
            product: product(build_id, version),
        };
        validate_takeover(
            &session,
            &candidate,
            Some(&fixture.executable.product),
            ROOT_ID,
            &protected,
            UserDataDispositionV1::Retain,
            action,
        )
        .unwrap();
    }

    let candidate = ExecutableIdentityV1 {
        install_dir: fixture.install_dir.clone(),
        main_executable: fixture.install_dir.join("Emily.exe"),
        product: product(NEW_BUILD_ID, "0.2.0"),
    };
    assert_eq!(
        validate_takeover(
            &session,
            &candidate,
            Some(&product(NEW_BUILD_ID, "0.2.0")),
            ROOT_ID,
            &protected,
            UserDataDispositionV1::Retain,
            TakeoverActionV1::Repair,
        ),
        Err("uninstall_old_executable_identity_mismatch".to_string())
    );

    let wrong_path = ExecutableIdentityV1 {
        main_executable: fixture.install_dir.join("Emily-other.exe"),
        ..candidate.clone()
    };
    assert_eq!(
        validate_takeover(
            &session,
            &wrong_path,
            Some(&fixture.executable.product),
            ROOT_ID,
            &protected,
            UserDataDispositionV1::Retain,
            TakeoverActionV1::Repair,
        ),
        Err("uninstall_candidate_path_mismatch".to_string())
    );
    assert_eq!(
        validate_takeover(
            &session,
            &candidate,
            Some(&fixture.executable.product),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &protected,
            UserDataDispositionV1::Retain,
            TakeoverActionV1::Repair,
        ),
        Err("uninstall_takeover_root_mismatch".to_string())
    );
}

#[test]
fn cross_version_repair_completes_only_after_candidate_manifest_verification() {
    let fixture = Fixture::new("cross-version-repair");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();

    fs::write(fixture.install_dir.join("Emily.exe"), b"exe-v2").unwrap();
    fs::write(fixture.install_dir.join("assets/app.js"), b"app-v2").unwrap();
    let candidate = PackageManifestV1 {
        schema_version: 1,
        application_id: fixture.manifest.application_id.clone(),
        product_name: fixture.manifest.product_name.clone(),
        build_id: NEW_BUILD_ID.to_string(),
        semantic_version: "0.2.0".to_string(),
        architecture: fixture.manifest.architecture.clone(),
        files: vec![
            entry(&fixture.install_dir, "Emily.exe"),
            entry(&fixture.install_dir, "assets/app.js"),
        ],
    };
    let candidate_bytes = canonical_package_manifest_bytes(&candidate).unwrap();
    let candidate_hash = package_manifest_sha256(&candidate_bytes);
    system.executable_products.borrow_mut().insert(
        normalize(&fixture.executable.main_executable),
        candidate.product_identity(),
    );
    *system.run_value.borrow_mut() = Some("unexpected-run-command".to_string());
    *system.device.borrow_mut() = DeviceAutostartSnapshotV1 {
        launch_at_startup: true,
        silent_autostart: true,
    };
    system.events.borrow_mut().clear();
    complete_repair_with_candidate_manifest(
        &fixture.coordination_dir,
        &authentication(),
        &candidate_bytes,
        &candidate_hash,
        &system,
    )
    .unwrap();
    assert_eq!(
        system.events.borrow().as_slice(),
        ["device:false,false", "run:<none>"]
    );
    assert_eq!(*system.run_value.borrow(), None);
    assert_eq!(
        *system.device.borrow(),
        DeviceAutostartSnapshotV1::default()
    );
    assert!(!fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());

    let fixture = Fixture::new("cross-version-repair-tampered");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    let mut tampered = candidate_bytes.clone();
    tampered.push(b' ');
    assert_eq!(
        complete_repair_with_candidate_manifest(
            &fixture.coordination_dir,
            &authentication(),
            &tampered,
            &candidate_hash,
            &system,
        ),
        Err("package_manifest_hash_mismatch".to_string())
    );
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());

    let fixture = Fixture::new("cross-version-repair-exe-identity");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    fs::write(fixture.install_dir.join("Emily.exe"), b"exe-v2").unwrap();
    fs::write(fixture.install_dir.join("assets/app.js"), b"app-v2").unwrap();
    let candidate = PackageManifestV1 {
        files: vec![
            entry(&fixture.install_dir, "Emily.exe"),
            entry(&fixture.install_dir, "assets/app.js"),
        ],
        ..candidate
    };
    let candidate_bytes = canonical_package_manifest_bytes(&candidate).unwrap();
    let candidate_hash = package_manifest_sha256(&candidate_bytes);
    assert_eq!(
        complete_repair_with_candidate_manifest(
            &fixture.coordination_dir,
            &authentication(),
            &candidate_bytes,
            &candidate_hash,
            &system,
        ),
        Err("uninstall_old_executable_identity_mismatch".to_string())
    );
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());
}

#[test]
fn repair_completion_cannot_bypass_candidate_manifest_verification() {
    let fixture = Fixture::new("repair-candidate-required");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();

    assert_eq!(
        complete_uninstall_with(
            &fixture.coordination_dir,
            &authentication(),
            UninstallCompletionV1::Repaired,
            &system,
        ),
        Err("uninstall_repair_candidate_manifest_required".to_string())
    );
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());
}

#[test]
fn early_process_mode_is_strict_and_package_build_identity_matches_build_rs() {
    assert!(matches!(
        parse_early_process_mode(&[OsString::from("--collector"), OsString::from("collect")])
            .unwrap(),
        EarlyProcessModeV1::Collector(_)
    ));
    assert!(matches!(
        parse_early_process_mode(&[OsString::from("--autostart")]).unwrap(),
        EarlyProcessModeV1::Gui
    ));
    assert!(parse_early_process_mode(&[OsString::from("--uninstall-guard")]).is_err());
    assert!(parse_early_process_mode(&[
        OsString::from("--package-manifest-build"),
        OsString::from("extra")
    ])
    .is_err());
    assert!(matches!(
        parse_early_process_mode(&[
            OsString::from("--installer-control"),
            OsString::from("inspect"),
            OsString::from("--coordination-dir"),
            OsString::from(r"C:\Users\test\AppData\Local\LifeArchiveProject\Emily\Uninstall"),
        ])
        .unwrap(),
        EarlyProcessModeV1::InstallerControl(InstallerControlCommandV1::Inspect { .. })
    ));
    assert!(parse_early_process_mode(&[
        OsString::from("--installer-control"),
        OsString::from("inspect"),
        OsString::from("--coordination-dir"),
    ])
    .is_err());

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let identity = package_identity::read_package_identity(&manifest_dir).unwrap();
    let computed = package_identity::compute_build_id(&manifest_dir, &identity, false).unwrap();
    assert_eq!(computed, COMPILED_BUILD_ID);
    assert_eq!(identity.application_id, APPLICATION_ID);
    assert_eq!(identity.product_name, PRODUCT_NAME);
    assert_eq!(identity.architecture, ARCHITECTURE);
    assert_eq!(computed.len(), 64);
}

#[test]
fn coordination_and_device_settings_paths_are_independent_contracts() {
    let local_app_data = PathBuf::from(r"C:\Users\test\AppData\Local");
    assert_eq!(
        uninstall_coordination_dir(&local_app_data).unwrap(),
        local_app_data.join("LifeArchiveProject/Emily/Uninstall")
    );
    assert_eq!(
        uninstall_coordination_dir(Path::new(r"C:\")),
        Err("uninstall_coordination_parent_invalid".to_string())
    );

    let fixture = Fixture::new("device-settings-path");
    let system = FakeSystem::ready(&fixture);
    let session =
        prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system)
            .unwrap();
    assert_eq!(session.device_settings_dir, fixture.root.join("bootstrap"));
    assert_ne!(session.device_settings_dir, fixture.coordination_dir);

    let mut raw: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.coordination_dir.join(UNINSTALL_SESSION_FILE)).unwrap(),
    )
    .unwrap();
    raw["deviceSettingsDir"] = json!(fixture.coordination_dir);
    fs::write(
        fixture.coordination_dir.join(UNINSTALL_SESSION_FILE),
        serde_json::to_vec_pretty(&raw).unwrap(),
    )
    .unwrap();
    assert_eq!(
        recover_uninstall_state_before_bootstrap_with(
            &fixture.coordination_dir,
            &fixture.executable,
            &system,
        ),
        Err("uninstall_device_settings_path_invalid".to_string())
    );
}

#[test]
fn guard_prepare_request_is_strict_and_preserves_the_existing_prepare_contract() {
    let fixture = Fixture::new("guard-prepare-request");
    let expected = fixture.request();
    let request = GuardPrepareRequestV1 {
        schema_version: 1,
        authentication: expected.authentication.clone(),
        root_id: expected.root_id.clone(),
        executable_identity: expected.executable_identity.clone(),
        device_settings_dir: expected.device_settings_dir.clone(),
        protected_paths: expected.protected_paths.clone(),
        user_data_disposition: expected.user_data_disposition,
        package_manifest_bytes: expected.package_manifest_bytes.clone(),
        expected_package_manifest_sha256: expected.expected_package_manifest_sha256.clone(),
    };
    let encoded = serde_json::to_vec(&request).unwrap();
    let decoded = serde_json::from_slice::<GuardPrepareRequestV1>(&encoded).unwrap();
    let converted = decoded.into_prepare_request().unwrap();
    assert_eq!(converted.authentication, expected.authentication);
    assert_eq!(converted.root_id, expected.root_id);
    assert_eq!(converted.executable_identity, expected.executable_identity);
    assert_eq!(converted.device_settings_dir, expected.device_settings_dir);
    assert_eq!(converted.protected_paths, expected.protected_paths);
    assert_eq!(
        converted.user_data_disposition,
        expected.user_data_disposition
    );
    assert_eq!(
        converted.package_manifest_bytes,
        expected.package_manifest_bytes
    );
    assert_eq!(
        converted.expected_package_manifest_sha256,
        expected.expected_package_manifest_sha256
    );

    let mut unknown = serde_json::to_value(&request).unwrap();
    unknown["unexpected"] = json!(true);
    assert!(serde_json::from_value::<GuardPrepareRequestV1>(unknown).is_err());

    let mut unsupported = request;
    unsupported.schema_version = 2;
    assert_eq!(
        unsupported.into_prepare_request().unwrap_err(),
        "uninstall_guard_prepare_request_invalid"
    );
}

#[test]
fn installer_control_cli_is_strict_for_every_lifecycle_transition() {
    let coordination = r"C:\Users\test\AppData\Local\LifeArchiveProject\Emily\Uninstall";
    let bootstrap = r"C:\Users\test\AppData\Roaming\lifearchive.douyin-history-archive";
    let install = r"C:\Users\test\AppData\Local\Programs\Emily";
    let executable = r"C:\Users\test\AppData\Local\Programs\Emily\Emily.exe";
    let manifest = r"C:\Users\test\AppData\Local\Temp\uninstall-package-manifest-v1.json";
    let hash = "a".repeat(64);

    let prepare = vec![
        "--installer-control",
        "prepare",
        "--coordination-dir",
        coordination,
        "--bootstrap-dir",
        bootstrap,
        "--install-dir",
        install,
        "--main-executable",
        executable,
        "--package-manifest",
        manifest,
        "--package-manifest-sha256",
        &hash,
        "--data-disposition",
        "retain",
        "--installer-pid",
        "4242",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    assert!(matches!(
        parse_early_process_mode(&prepare).unwrap(),
        EarlyProcessModeV1::InstallerControl(InstallerControlCommandV1::Prepare { .. })
    ));

    let takeover = vec![
        "--installer-control",
        "takeover",
        "--coordination-dir",
        coordination,
        "--install-dir",
        install,
        "--main-executable",
        executable,
        "--package-manifest",
        manifest,
        "--package-manifest-sha256",
        &hash,
        "--action",
        "repair",
        "--installer-pid",
        "4242",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    assert!(matches!(
        parse_early_process_mode(&takeover).unwrap(),
        EarlyProcessModeV1::InstallerControl(InstallerControlCommandV1::Takeover { .. })
    ));

    for command in [
        "assert-alive",
        "commit",
        "delete-managed-data",
        "complete-uninstall",
        "abort",
    ] {
        let arguments = authenticated_installer_control_arguments(command, coordination);
        assert!(matches!(
            parse_early_process_mode(&arguments).unwrap(),
            EarlyProcessModeV1::InstallerControl(_)
        ));
    }

    let mut repair = authenticated_installer_control_arguments("complete-repair", coordination);
    repair.extend([
        OsString::from("--package-manifest"),
        OsString::from(manifest),
        OsString::from("--package-manifest-sha256"),
        OsString::from(&hash),
    ]);
    assert!(matches!(
        parse_early_process_mode(&repair).unwrap(),
        EarlyProcessModeV1::InstallerControl(InstallerControlCommandV1::CompleteRepair { .. })
    ));

    let mut invalid = prepare.clone();
    invalid.push(OsString::from("--unexpected"));
    assert_eq!(
        parse_early_process_mode(&invalid),
        Err("installer_control_arguments_invalid".to_string())
    );
    let mut invalid = prepare;
    *invalid.last_mut().unwrap() = OsString::from("0");
    assert_eq!(
        parse_early_process_mode(&invalid),
        Err("installer_control_arguments_invalid".to_string())
    );
}

#[test]
fn installer_control_drives_authenticated_commit_data_and_terminal_transitions() {
    let fixture = Fixture::new("installer-control-transitions");
    let system = FakeSystem::ready(&fixture);
    let runtime = FakeInstallerRuntime::new(&system);
    let auth = InstallerAuthenticationArgsV1 {
        coordination_dir: fixture.coordination_dir.clone(),
        authentication: authentication(),
    };

    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    runtime.guard_started.set(true);
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::AssertAlive(auth.clone()),
            &system,
            &runtime,
        ),
        Ok("alive".to_string())
    );
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::DeleteManagedData(auth.clone()),
            &system,
            &runtime,
        ),
        Err("uninstall_phase_transition_invalid".to_string())
    );
    assert!(runtime.deleted_bootstrap.borrow().is_empty());

    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::Commit(auth.clone()),
            &system,
            &runtime,
        ),
        Ok("committed".to_string())
    );
    assert!(runtime.destructive_commit_prepared.get());
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::DeleteManagedData(auth.clone()),
            &system,
            &runtime,
        ),
        Ok("data-retained".to_string())
    );
    assert!(runtime.deleted_bootstrap.borrow().is_empty());

    fs::remove_file(fixture.install_dir.join("Emily.exe")).unwrap();
    fs::remove_file(fixture.install_dir.join("assets/app.js")).unwrap();
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::CompleteUninstall(auth),
            &system,
            &runtime,
        ),
        Ok("completed".to_string())
    );
    assert!(runtime.guard_finished.get());
}

#[test]
fn installer_control_keeps_prepared_phase_when_runtime_cleanup_fails() {
    let fixture = Fixture::new("installer-control-cleanup-failure");
    let system = FakeSystem::ready(&fixture);
    let runtime = FakeInstallerRuntime::new(&system);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    runtime.guard_started.set(true);
    runtime.fail_destructive_commit_prepare.set(true);
    let auth = InstallerAuthenticationArgsV1 {
        coordination_dir: fixture.coordination_dir.clone(),
        authentication: authentication(),
    };

    assert_eq!(
        run_installer_control_with(InstallerControlCommandV1::Commit(auth), &system, &runtime,),
        Err("uninstall_runtime_cleanup_failed".to_string())
    );
    let session: UninstallSessionV1 = serde_json::from_slice(
        &fs::read(fixture.coordination_dir.join(UNINSTALL_SESSION_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(session.phase, UninstallPhaseV1::Prepared);
}

#[test]
fn installer_control_deletes_selected_data_only_after_commit_and_aborts_before_commit() {
    let fixture = Fixture::new("installer-control-delete");
    let system = FakeSystem::ready(&fixture);
    let runtime = FakeInstallerRuntime::new(&system);
    let mut request = fixture.request();
    request.user_data_disposition = UserDataDispositionV1::Delete;
    prepare_uninstall_session_with(&fixture.coordination_dir, request, &system).unwrap();
    runtime.guard_started.set(true);
    let auth = InstallerAuthenticationArgsV1 {
        coordination_dir: fixture.coordination_dir.clone(),
        authentication: authentication(),
    };

    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::DeleteManagedData(auth.clone()),
            &system,
            &runtime,
        ),
        Err("uninstall_phase_transition_invalid".to_string())
    );
    assert!(runtime.deleted_bootstrap.borrow().is_empty());
    run_installer_control_with(
        InstallerControlCommandV1::Commit(auth.clone()),
        &system,
        &runtime,
    )
    .unwrap();
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::DeleteManagedData(auth.clone()),
            &system,
            &runtime,
        ),
        Ok("data-deleted".to_string())
    );
    assert_eq!(
        runtime.deleted_bootstrap.borrow().as_slice(),
        [fixture.root.join("bootstrap")]
    );

    let abort_fixture = Fixture::new("installer-control-abort");
    let abort_system = FakeSystem::ready(&abort_fixture);
    let abort_runtime = FakeInstallerRuntime::new(&abort_system);
    prepare_uninstall_session_with(
        &abort_fixture.coordination_dir,
        abort_fixture.request(),
        &abort_system,
    )
    .unwrap();
    abort_runtime.guard_started.set(true);
    let abort_auth = InstallerAuthenticationArgsV1 {
        coordination_dir: abort_fixture.coordination_dir.clone(),
        authentication: authentication(),
    };
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::Abort(abort_auth),
            &abort_system,
            &abort_runtime,
        ),
        Ok("aborted".to_string())
    );
    assert!(abort_runtime.guard_aborted.get());
    assert!(!abort_fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .exists());
}

#[test]
fn installer_control_abort_after_commit_signals_repair_without_rolling_back() {
    let fixture = Fixture::new("installer-control-abort-after-commit");
    let system = FakeSystem::ready(&fixture);
    let runtime = FakeInstallerRuntime::new(&system);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    runtime.guard_started.set(true);
    system.events.borrow_mut().clear();
    let auth = InstallerAuthenticationArgsV1 {
        coordination_dir: fixture.coordination_dir.clone(),
        authentication: authentication(),
    };

    assert_eq!(
        run_installer_control_with(InstallerControlCommandV1::Abort(auth), &system, &runtime,),
        Ok("repair-required".to_string())
    );
    assert!(runtime.guard_aborted.get());
    assert!(system.events.borrow().is_empty());
    assert!(fixture
        .coordination_dir
        .join(UNINSTALL_SESSION_FILE)
        .is_file());
}

#[test]
fn installer_control_prepare_builds_the_guard_request_from_bootstrap_and_manifest() {
    let fixture = Fixture::new("installer-control-prepare");
    let system = FakeSystem::ready(&fixture);
    let runtime = FakeInstallerRuntime::new(&system);
    let bootstrap_dir = fixture.root.join("bootstrap");
    fs::create_dir_all(&bootstrap_dir).unwrap();
    write_bootstrap_atomic(
        &bootstrap_dir,
        &BootstrapV1::new(fixture.data_dir.clone(), ROOT_ID.to_string()).unwrap(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("package-manifest.json");
    fs::write(&manifest_path, &fixture.manifest_bytes).unwrap();

    let output = run_installer_control_with(
        InstallerControlCommandV1::Prepare {
            coordination_dir: fixture.coordination_dir.clone(),
            bootstrap_dir: bootstrap_dir.clone(),
            install_dir: fixture.install_dir.clone(),
            main_executable: fixture.executable.main_executable.clone(),
            package_manifest: manifest_path,
            package_manifest_sha256: fixture.manifest_hash.clone(),
            user_data_disposition: UserDataDispositionV1::Retain,
            installer_pid: INSTALLER_PID,
        },
        &system,
        &runtime,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["schemaVersion"], 1);
    assert_eq!(output["sessionNonce"].as_str().unwrap().len(), 32);
    assert_eq!(output["installerPid"], INSTALLER_PID);
    assert_eq!(output["installerCreatedAt100ns"], INSTALLER_CREATED_AT);
    assert_eq!(
        output["controlExecutable"],
        json!(fixture.coordination_dir.join("controller.exe"))
    );
    assert_eq!(
        output["guardExecutable"],
        json!(fixture.coordination_dir.join("guard.exe"))
    );
    assert!(runtime.guard_started.get());
    let session_arguments =
        fs::read_to_string(fixture.coordination_dir.join(INSTALLER_SESSION_ARGS_FILE)).unwrap();
    let session_lines = session_arguments.lines().collect::<Vec<_>>();
    assert_eq!(
        &session_lines[3..],
        [
            fixture
                .coordination_dir
                .join("controller.exe")
                .to_string_lossy()
                .as_ref(),
            fixture
                .coordination_dir
                .join("guard.exe")
                .to_string_lossy()
                .as_ref(),
        ]
    );
    let session: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.coordination_dir.join(UNINSTALL_SESSION_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(session["rootId"], ROOT_ID);
    assert_eq!(session["deviceSettingsDir"], json!(bootstrap_dir));
    assert_eq!(session["userDataDisposition"], "retain");
}

#[test]
fn installer_control_prepare_supports_a_never_started_install_without_creating_user_data() {
    let fixture = Fixture::new("installer-control-never-started");
    let system = FakeSystem::ready(&fixture);
    let runtime = FakeInstallerRuntime::new(&system);
    let bootstrap_dir = fixture.root.join("bootstrap");
    let manifest_path = fixture.root.join("package-manifest.json");
    fs::write(&manifest_path, &fixture.manifest_bytes).unwrap();

    let output = run_installer_control_with(
        InstallerControlCommandV1::Prepare {
            coordination_dir: fixture.coordination_dir.clone(),
            bootstrap_dir: bootstrap_dir.clone(),
            install_dir: fixture.install_dir.clone(),
            main_executable: fixture.executable.main_executable.clone(),
            package_manifest: manifest_path,
            package_manifest_sha256: fixture.manifest_hash.clone(),
            user_data_disposition: UserDataDispositionV1::Delete,
            installer_pid: INSTALLER_PID,
        },
        &system,
        &runtime,
    )
    .unwrap();

    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["schemaVersion"], 1);
    assert!(!bootstrap_dir.exists());
    assert_eq!(system.events.borrow().as_slice(), ["run:<none>"]);

    let session: UninstallSessionV1 = serde_json::from_slice(
        &fs::read(fixture.coordination_dir.join(UNINSTALL_SESSION_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(session.root_id, "00000000000000000000000000000000");
    assert_eq!(
        session.old_device_autostart,
        DeviceAutostartSnapshotV1::default()
    );
    assert_eq!(
        session.protected_paths,
        vec![bootstrap_dir.clone(), fixture.coordination_dir.clone()]
    );

    let auth = InstallerAuthenticationArgsV1 {
        coordination_dir: fixture.coordination_dir.clone(),
        authentication: session.authentication,
    };
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::Commit(auth.clone()),
            &system,
            &runtime,
        ),
        Ok("committed".to_string())
    );
    assert_eq!(
        run_installer_control_with(
            InstallerControlCommandV1::DeleteManagedData(auth),
            &system,
            &runtime,
        ),
        Ok("data-deleted".to_string())
    );
    assert!(runtime.deleted_bootstrap.borrow().is_empty());
    assert!(!bootstrap_dir.exists());
}

#[test]
fn installer_control_prepare_rejects_existing_missing_or_invalid_bootstrap() {
    for (name, contents, expected) in [
        ("missing", None, "uninstall_bootstrap_missing"),
        (
            "invalid",
            Some(b"{\"schemaVersion\":999}".as_slice()),
            "uninstall_bootstrap_invalid",
        ),
    ] {
        let fixture = Fixture::new(&format!("installer-control-bootstrap-{name}"));
        let system = FakeSystem::ready(&fixture);
        let runtime = FakeInstallerRuntime::new(&system);
        let bootstrap_dir = fixture.root.join("bootstrap");
        fs::create_dir_all(&bootstrap_dir).unwrap();
        if let Some(contents) = contents {
            fs::write(bootstrap_dir.join("bootstrap.json"), contents).unwrap();
        }
        let manifest_path = fixture.root.join("package-manifest.json");
        fs::write(&manifest_path, &fixture.manifest_bytes).unwrap();

        assert_eq!(
            run_installer_control_with(
                InstallerControlCommandV1::Prepare {
                    coordination_dir: fixture.coordination_dir.clone(),
                    bootstrap_dir,
                    install_dir: fixture.install_dir.clone(),
                    main_executable: fixture.executable.main_executable.clone(),
                    package_manifest: manifest_path,
                    package_manifest_sha256: fixture.manifest_hash.clone(),
                    user_data_disposition: UserDataDispositionV1::Retain,
                    installer_pid: INSTALLER_PID,
                },
                &system,
                &runtime,
            ),
            Err(expected.to_string())
        );
        assert!(!runtime.guard_started.get());
        assert!(system.events.borrow().is_empty());
    }
}

#[test]
fn installer_control_takeover_rotates_authentication_after_validating_the_old_session() {
    let fixture = Fixture::new("installer-control-takeover");
    let system = FakeSystem::ready(&fixture);
    prepare_uninstall_session_with(&fixture.coordination_dir, fixture.request(), &system).unwrap();
    transition_uninstall_to_destructive_with(&fixture.coordination_dir, &authentication(), &system)
        .unwrap();
    fs::write(
        fixture
            .coordination_dir
            .join(UNINSTALL_INTERRUPTED_FAULT_FILE),
        serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "nonce": NONCE,
            "reason": "uninstall_installer_exited",
            "observedAt": "2026-08-12T00:00:00.000Z"
        }))
        .unwrap(),
    )
    .unwrap();
    let manifest_path = fixture.root.join("package-manifest.json");
    fs::write(&manifest_path, &fixture.manifest_bytes).unwrap();
    let runtime = FakeInstallerRuntime::new(&system);

    let output = run_installer_control_with(
        InstallerControlCommandV1::Takeover {
            coordination_dir: fixture.coordination_dir.clone(),
            install_dir: fixture.install_dir.clone(),
            main_executable: fixture.executable.main_executable.clone(),
            package_manifest: manifest_path,
            package_manifest_sha256: fixture.manifest_hash.clone(),
            action: TakeoverActionV1::ContinueUninstall,
            installer_pid: INSTALLER_PID,
        },
        &system,
        &runtime,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_ne!(output["sessionNonce"], NONCE);
    assert_eq!(output["installerPid"], INSTALLER_PID);
    assert!(runtime.guard_started.get());
    let session: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.coordination_dir.join(UNINSTALL_SESSION_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(session["phase"], "destructiveCommitted");
    assert_eq!(
        session["authentication"],
        json!({
            "nonce": output["sessionNonce"],
            "installerPid": INSTALLER_PID,
            "installerCreatedAt100ns": INSTALLER_CREATED_AT
        })
    );
    assert!(!fixture
        .coordination_dir
        .join(UNINSTALL_INTERRUPTED_FAULT_FILE)
        .exists());
}

fn authenticated_installer_control_arguments(command: &str, coordination: &str) -> Vec<OsString> {
    vec![
        OsString::from("--installer-control"),
        OsString::from(command),
        OsString::from("--coordination-dir"),
        OsString::from(coordination),
        OsString::from("--session-nonce"),
        OsString::from(NONCE),
        OsString::from("--installer-pid"),
        OsString::from(INSTALLER_PID.to_string()),
        OsString::from("--installer-created-at-100ns"),
        OsString::from(INSTALLER_CREATED_AT.to_string()),
    ]
}

#[test]
fn nsis_contract() {
    const UPSTREAM_COMMIT: &str = "8909f221d1515955fc843808032bdc5d62209c96";
    const UPSTREAM_CRLF_SHA256: &str =
        "1114a136876d667ca7ba77218d4e966a625d1078dcba18484caae8f7bffb8626";

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let template_path = manifest_dir.join("nsis/installer-template.nsi");
    let template_bytes = fs::read(&template_path).expect("custom NSIS template must exist");
    assert!(
        template_bytes.windows(2).any(|pair| pair == b"\r\n")
            && !template_bytes
                .iter()
                .enumerate()
                .any(|(index, byte)| *byte == b'\n'
                    && (index == 0 || template_bytes[index - 1] != b'\r')),
        "tracked NSIS template must use CRLF line endings"
    );
    let template = String::from_utf8(template_bytes).expect("NSIS template must be UTF-8");
    assert!(template.contains(&format!("; Upstream commit: {UPSTREAM_COMMIT}")));
    assert!(template.contains(&format!("; Upstream CRLF SHA-256: {UPSTREAM_CRLF_SHA256}")));

    let mut upstream = String::new();
    let mut in_emily_block = false;
    for line in template.replace("\r\n", "\n").lines() {
        if line.starts_with("; EMILY BEGIN ") {
            assert!(!in_emily_block, "Emily template blocks must not nest");
            in_emily_block = true;
            continue;
        }
        if line.starts_with("; EMILY END ") {
            assert!(in_emily_block, "Emily template block end must be paired");
            in_emily_block = false;
            continue;
        }
        if !in_emily_block {
            upstream.push_str(line);
            upstream.push_str("\r\n");
        }
    }
    assert!(!in_emily_block, "Emily template block must be closed");
    assert_eq!(
        format!("{:x}", Sha256::digest(upstream.as_bytes())),
        UPSTREAM_CRLF_SHA256
    );

    let install_init = section_between(&template, "Function .onInit", "FunctionEnd");
    assert_before(
        install_init,
        "Call EmilyInspectCoordinationState",
        "${GetOptions} $CMDLINE \"/UPDATE\" $UpdateMode",
    );

    let webview2 = section_between(&template, "Section WebView2", "SectionEnd");
    for expected in [
        "ReadRegStr $R9 HKLM \"SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2APPGUID}\" \"location\"",
        "ReadRegStr $R8 HKCU \"SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2APPGUID}\" \"pv\"",
        "ReadRegStr $R9 HKCU \"SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\${WEBVIEW2APPGUID}\" \"location\"",
        "StrCpy $R9 \"$PROGRAMFILES32\\Microsoft\\EdgeWebView\\Application\"",
        "StrCpy $R9 \"$LOCALAPPDATA\\Microsoft\\EdgeWebView\\Application\"",
        "IfFileExists \"$R9\\$R8\\msedgewebview2.exe\"",
        "IfFileExists \"$R9\\$R8\\msedge.dll\"",
        "IfFileExists \"$R9\\$R8\\icudtl.dat\"",
        "StrCpy $4 \"\"",
        "emily_webview2_try_current_user:",
        "emily_webview2_payload_validated:",
    ] {
        assert!(
            webview2.contains(expected),
            "WebView2 detection must reject stale registry entries when runtime payload is missing: {expected}"
        );
    }
    assert_before(
        webview2,
        "IfFileExists \"$R9\\$R8\\msedgewebview2.exe\"",
        "; Webview2 installation",
    );
    assert_before(
        webview2,
        "emily_webview2_try_current_user:",
        "emily_webview2_payload_validated:",
    );

    let uninstall_init = section_between(&template, "Function un.onInit", "FunctionEnd");
    assert_before(
        uninstall_init,
        "Call un.EmilyInspectCoordinationState",
        "${GetOptions} $CMDLINE \"/UPDATE\" $UpdateMode",
    );

    let uninstall = section_between(&template, "Section Uninstall", "SectionEnd");
    assert_before(
        uninstall,
        "Call un.EmilyPrepareGuard",
        "Call un.EmilyWaitGuardReady",
    );
    assert_before(
        uninstall,
        "Call un.EmilyWaitGuardReady",
        "Call un.EmilyCommitGuard",
    );
    assert_before(
        uninstall,
        "Call un.EmilyCommitGuard",
        "; Copy main executable",
    );
    assert_before(
        uninstall,
        "Call un.EmilyAssertGuardAlive",
        "; Copy main executable",
    );
    assert_before(
        uninstall,
        "; Copy main executable",
        "Call un.EmilyDeleteManagedData",
    );
    assert_before(
        uninstall,
        "Call un.EmilyDeleteManagedData",
        "Call un.EmilyCompleteGuard",
    );
    assert!(uninstall.contains("${If} $EmilyCoordinationState == \"repair_required\""));
    assert!(uninstall.contains("${ElseIf} $UpdateMode = 1"));
    assert_before(
        uninstall,
        "${If} $EmilyCoordinationState == \"repair_required\"",
        "${ElseIf} $UpdateMode = 1",
    );
    assert_before(
        uninstall,
        "Call un.EmilyRepairTakeover",
        "${ElseIf} $UpdateMode = 1",
    );
    assert!(uninstall.contains("Call un.EmilyAbortGuard"));
    assert_before(
        uninstall,
        "Delete \"$INSTDIR\\${EMILY_PACKAGE_MANIFEST_FILE}\"",
        "Call un.EmilyCompleteGuard",
    );
    assert!(
        uninstall.rfind("RMDir \"$INSTDIR\"").unwrap()
            > uninstall.find("Call un.EmilyCompleteGuard").unwrap()
    );
    assert_eq!(uninstall.matches("RMDir \"$INSTDIR\"").count(), 2);

    let install = section_between(&template, "Section Install", "SectionEnd");
    assert_before(
        install,
        "Call EmilyRepairTakeover",
        "; Copy main executable",
    );
    assert_before(install, "WriteUninstaller", "Call EmilyCompleteRepair");
    assert_before(
        install,
        "Call CreateOrUpdateStartMenuShortcut",
        "Call EmilyCompleteRepair",
    );
    assert_before(
        install,
        "Call CreateOrUpdateDesktopShortcut",
        "Call EmilyCompleteRepair",
    );
    assert_before(
        install,
        "!insertmacro NSIS_HOOK_POSTINSTALL",
        "Call EmilyCompleteRepair",
    );

    let hooks = fs::read_to_string(manifest_dir.join("nsis/installer-hooks.nsh")).unwrap();
    assert!(!hooks.contains("--uninstall-managed-data"));
    assert!(hooks.contains("File /oname=$PLUGINSDIR\\emily-installer-control.exe"));
    assert!(hooks.contains("--action continue-uninstall"));
    assert!(hooks.contains("--action repair"));
    assert!(!hooks.contains("nsExec::ExecToStack '\"${MAINBINARYSRCPATH}\" --installer-control"));
    let repair_takeover = section_between(&hooks, "Function un.EmilyRepairTakeover", "FunctionEnd");
    assert!(!repair_takeover.contains("${If} $UpdateMode"));

    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest_dir.join("tauri.conf.json")).unwrap()).unwrap();
    assert_eq!(
        config
            .pointer("/bundle/windows/nsis/template")
            .and_then(|value| value.as_str()),
        Some("nsis/installer-template.nsi")
    );
    assert_eq!(
        config
            .pointer("/bundle/windows/webviewInstallMode/type")
            .and_then(|value| value.as_str()),
        Some("embedBootstrapper")
    );
    assert_eq!(
        config
            .pointer("/bundle/windows/webviewInstallMode/silent")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    let build = config
        .get("build")
        .and_then(|value| value.as_object())
        .unwrap();
    assert_eq!(
        build.get("frontendDist").and_then(|value| value.as_str()),
        Some("../dist")
    );
    assert!(!build.contains_key("beforeBuildCommand"));
    assert!(!build.contains_key("beforeBundleCommand"));
    let config_text = serde_json::to_string(&config).unwrap().to_ascii_lowercase();
    for forbidden in ["node", "npm", "npx"] {
        assert!(
            !config_text.contains(forbidden),
            "production config references {forbidden}"
        );
    }
}

fn section_between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let start = text
        .find(start)
        .unwrap_or_else(|| panic!("missing section start: {start}"));
    let remainder = &text[start..];
    let end = remainder
        .find(end)
        .unwrap_or_else(|| panic!("missing section end: {end}"));
    &remainder[..end]
}

fn assert_before(text: &str, first: &str, second: &str) {
    let first_offset = text
        .find(first)
        .unwrap_or_else(|| panic!("missing ordered marker: {first}"));
    let second_offset = text
        .find(second)
        .unwrap_or_else(|| panic!("missing ordered marker: {second}"));
    assert!(
        first_offset < second_offset,
        "expected `{first}` before `{second}`"
    );
}

fn authentication() -> GuardAuthenticationV1 {
    GuardAuthenticationV1 {
        nonce: NONCE.to_string(),
        installer_pid: INSTALLER_PID,
        installer_created_at_100ns: INSTALLER_CREATED_AT,
    }
}

fn product(build_id: &str, semantic_version: &str) -> ProductIdentityV1 {
    ProductIdentityV1 {
        application_id: APPLICATION_ID.to_string(),
        product_name: PRODUCT_NAME.to_string(),
        build_id: build_id.to_string(),
        semantic_version: semantic_version.to_string(),
        architecture: "windows-x86_64".to_string(),
    }
}

fn entry(root: &Path, relative: &str) -> PackageManifestEntryV1 {
    let bytes = fs::read(root.join(relative)).unwrap();
    PackageManifestEntryV1 {
        path: relative.to_string(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

fn fixture_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "emily-uninstall-{label}-{}-{nanos}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ))
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
