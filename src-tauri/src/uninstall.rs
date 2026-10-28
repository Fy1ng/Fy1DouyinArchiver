#[path = "../build_support/uninstall_manifest.rs"]
mod uninstall_manifest;

pub use uninstall_manifest::{
    canonical_package_manifest_bytes, package_manifest_sha256, parse_package_manifest,
    validate_package_manifest, verify_package_manifest_files, ManifestVerificationModeV1,
    PackageManifestEntryV1, PackageManifestV1, ProductIdentityV1, PACKAGE_MANIFEST_FILE,
};

use crate::{
    autostart::{HkcuRunRegistry, RunRegistry},
    bootstrap::{read_bootstrap, write_json_atomically, BootstrapV1},
    data_root::parse_uninstall_managed_data_arguments,
    device_settings::DeviceSettingsStore,
    process_lock::UninstallProcessGuard,
};
use chrono::DateTime;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const UNINSTALL_SESSION_FILE: &str = "uninstall-session.json";
pub const AUTOSTART_CLEANUP_FAULT_FILE: &str = "autostart-cleanup-fault.json";
pub const UNINSTALL_INTERRUPTED_FAULT_FILE: &str = "uninstall-interrupted-fault.json";
pub const INSTALLER_SESSION_FILE: &str = "installer-session-v1.json";
pub const INSTALLER_SESSION_ARGS_FILE: &str = "installer-session-v1.args";
pub const COMPILED_BUILD_ID: &str = env!("EMILY_BUILD_ID");
pub const PRODUCT_NAME: &str = env!("EMILY_PRODUCT_NAME");
pub const ARCHITECTURE: &str = env!("EMILY_ARCHITECTURE");
const APPLICATION_ID: &str = env!("EMILY_APPLICATION_ID");
const SEMANTIC_VERSION: &str = env!("EMILY_SEMANTIC_VERSION");
const CONTRACT_SCHEMA_VERSION: u32 = 1;
const UNINITIALIZED_ROOT_ID: &str = "00000000000000000000000000000000";
const EXECUTABLE_IDENTITY_BEGIN: &[u8] = b"\x1eEMILY_EXECUTABLE_IDENTITY_V1\x1f";
const EXECUTABLE_IDENTITY_END: &[u8] = b"\x1eEND_EMILY_EXECUTABLE_IDENTITY_V1\x1f";
const MAX_EXECUTABLE_IDENTITY_SCAN_BYTES: u64 = 512 * 1024 * 1024;

#[used]
static EMBEDDED_EXECUTABLE_IDENTITY_V1: &[u8] = concat!(
    "\u{1e}EMILY_EXECUTABLE_IDENTITY_V1\u{1f}",
    env!("EMILY_APPLICATION_ID"),
    "\u{1f}",
    env!("EMILY_PRODUCT_NAME"),
    "\u{1f}",
    env!("EMILY_BUILD_ID"),
    "\u{1f}",
    env!("EMILY_SEMANTIC_VERSION"),
    "\u{1f}",
    env!("EMILY_ARCHITECTURE"),
    "\u{1e}END_EMILY_EXECUTABLE_IDENTITY_V1\u{1f}"
)
.as_bytes();

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutableIdentityV1 {
    pub install_dir: PathBuf,
    pub main_executable: PathBuf,
    pub product: ProductIdentityV1,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceAutostartSnapshotV1 {
    pub launch_at_startup: bool,
    pub silent_autostart: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardAuthenticationV1 {
    pub nonce: String,
    pub installer_pid: u32,
    pub installer_created_at_100ns: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UninstallPhaseV1 {
    Preparing,
    Prepared,
    DestructiveCommitted,
    Completed,
    RepairCompleted,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UserDataDispositionV1 {
    Retain,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninstallCompletionV1 {
    Uninstalled,
    Repaired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverActionV1 {
    ContinueUninstall,
    Repair,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UninstallSessionV1 {
    pub schema_version: u32,
    pub authentication: GuardAuthenticationV1,
    pub root_id: String,
    pub old_run_value: Option<String>,
    pub old_device_autostart: DeviceAutostartSnapshotV1,
    pub phase: UninstallPhaseV1,
    pub device_settings_dir: PathBuf,
    pub install_dir: PathBuf,
    pub main_executable: PathBuf,
    pub application_id: String,
    pub product_name: String,
    pub old_build_id: String,
    pub old_semantic_version: String,
    pub architecture: String,
    pub protected_paths: Vec<PathBuf>,
    pub user_data_disposition: UserDataDispositionV1,
    pub package_manifest: PackageManifestV1,
    pub package_manifest_sha256: String,
    pub prepared_at: String,
    pub updated_at: String,
}

impl UninstallSessionV1 {
    pub fn executable_identity(&self) -> ExecutableIdentityV1 {
        ExecutableIdentityV1 {
            install_dir: self.install_dir.clone(),
            main_executable: self.main_executable.clone(),
            product: ProductIdentityV1 {
                application_id: self.application_id.clone(),
                product_name: self.product_name.clone(),
                build_id: self.old_build_id.clone(),
                semantic_version: self.old_semantic_version.clone(),
                architecture: self.architecture.clone(),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct UninstallPrepareRequestV1 {
    pub authentication: GuardAuthenticationV1,
    pub root_id: String,
    pub executable_identity: ExecutableIdentityV1,
    pub device_settings_dir: PathBuf,
    pub protected_paths: Vec<PathBuf>,
    pub user_data_disposition: UserDataDispositionV1,
    pub package_manifest_bytes: Vec<u8>,
    pub expected_package_manifest_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardPrepareRequestV1 {
    pub schema_version: u32,
    pub authentication: GuardAuthenticationV1,
    pub root_id: String,
    pub executable_identity: ExecutableIdentityV1,
    pub device_settings_dir: PathBuf,
    pub protected_paths: Vec<PathBuf>,
    pub user_data_disposition: UserDataDispositionV1,
    pub package_manifest_bytes: Vec<u8>,
    pub expected_package_manifest_sha256: String,
}

impl GuardPrepareRequestV1 {
    pub fn into_prepare_request(self) -> Result<UninstallPrepareRequestV1, String> {
        if self.schema_version != CONTRACT_SCHEMA_VERSION {
            return Err("uninstall_guard_prepare_request_invalid".to_string());
        }
        Ok(UninstallPrepareRequestV1 {
            authentication: self.authentication,
            root_id: self.root_id,
            executable_identity: self.executable_identity,
            device_settings_dir: self.device_settings_dir,
            protected_paths: self.protected_paths,
            user_data_disposition: self.user_data_disposition,
            package_manifest_bytes: self.package_manifest_bytes,
            expected_package_manifest_sha256: self.expected_package_manifest_sha256,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutostartCleanupFaultV1 {
    pub schema_version: u32,
    pub nonce: String,
    pub phase: UninstallPhaseV1,
    pub reason: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UninstallInterruptedFaultV1 {
    pub schema_version: u32,
    pub nonce: String,
    pub reason: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninstallRecoveryOutcomeV1 {
    Ready,
    RolledBack,
    TerminalCleaned,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum InstallerOwnedObjectV1 {
    StartMenuShortcut { path: PathBuf },
    UninstallRegistryValue { key: String, value_name: String },
    SelfUninstaller { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallGuardArgsV1 {
    pub coordination_dir: PathBuf,
    pub prepare_request: Option<PathBuf>,
    pub authentication: GuardAuthenticationV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallerControlCommandV1 {
    Inspect {
        coordination_dir: PathBuf,
    },
    Prepare {
        coordination_dir: PathBuf,
        bootstrap_dir: PathBuf,
        install_dir: PathBuf,
        main_executable: PathBuf,
        package_manifest: PathBuf,
        package_manifest_sha256: String,
        user_data_disposition: UserDataDispositionV1,
        installer_pid: u32,
    },
    Takeover {
        coordination_dir: PathBuf,
        install_dir: PathBuf,
        main_executable: PathBuf,
        package_manifest: PathBuf,
        package_manifest_sha256: String,
        action: TakeoverActionV1,
        installer_pid: u32,
    },
    AssertAlive(InstallerAuthenticationArgsV1),
    Commit(InstallerAuthenticationArgsV1),
    DeleteManagedData(InstallerAuthenticationArgsV1),
    CompleteUninstall(InstallerAuthenticationArgsV1),
    CompleteRepair {
        authentication: InstallerAuthenticationArgsV1,
        package_manifest: PathBuf,
        package_manifest_sha256: String,
    },
    Abort(InstallerAuthenticationArgsV1),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerAuthenticationArgsV1 {
    pub coordination_dir: PathBuf,
    pub authentication: GuardAuthenticationV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerGuardProcessV1 {
    pub control_executable: PathBuf,
    pub guard_executable: PathBuf,
}

pub trait InstallerControlRuntime {
    fn start_guard(
        &self,
        coordination_dir: &Path,
        authentication: &GuardAuthenticationV1,
        expected_guard_product: &ProductIdentityV1,
        prepare_request: Option<&GuardPrepareRequestV1>,
    ) -> Result<InstallerGuardProcessV1, String>;
    fn assert_guard_alive(&self, authentication: &GuardAuthenticationV1) -> Result<(), String>;
    fn prepare_for_destructive_commit(&self, session: &UninstallSessionV1) -> Result<(), String>;
    fn signal_guard_finished(
        &self,
        coordination_dir: &Path,
        authentication: &GuardAuthenticationV1,
    ) -> Result<(), String>;
    fn signal_guard_aborted(
        &self,
        coordination_dir: &Path,
        authentication: &GuardAuthenticationV1,
    ) -> Result<(), String>;
    fn delete_managed_data(&self, bootstrap_dir: &Path) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerCoordinationStateV1 {
    Clean,
    RolledBack,
    RepairRequired,
}

impl InstallerCoordinationStateV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::RolledBack => "rolled_back",
            Self::RepairRequired => "repair_required",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UninstallGuardEventNamesV1 {
    pub ready: String,
    pub finished: String,
    pub aborted: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UninstallGuardWaitOutcomeV1 {
    Finished,
    Aborted,
    InstallerExited,
}

pub trait UninstallGuardHandshake {
    fn signal_ready(&self) -> Result<(), String>;
    fn wait_for_terminal(&self) -> Result<UninstallGuardWaitOutcomeV1, String>;
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct NativeUninstallGuardHandshake {
    ready: windows_sys::Win32::Foundation::HANDLE,
    finished: windows_sys::Win32::Foundation::HANDLE,
    aborted: windows_sys::Win32::Foundation::HANDLE,
    installer: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
impl NativeUninstallGuardHandshake {
    fn new(authentication: &GuardAuthenticationV1) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
        use windows_sys::Win32::System::Threading::{
            CreateEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let names = uninstall_guard_event_names(&authentication.nonce)?;
        let create_event = |name: &str| -> Result<_, String> {
            let wide = OsStr::new(name)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, wide.as_ptr()) };
            if handle.is_null() {
                Err(std::io::Error::last_os_error().to_string())
            } else if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                Err("uninstall_guard_event_already_exists".to_string())
            } else {
                Ok(handle)
            }
        };

        let ready = create_event(&names.ready)?;
        let finished = match create_event(&names.finished) {
            Ok(handle) => handle,
            Err(error) => {
                unsafe { windows_sys::Win32::Foundation::CloseHandle(ready) };
                return Err(error);
            }
        };
        let aborted = match create_event(&names.aborted) {
            Ok(handle) => handle,
            Err(error) => {
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(finished);
                    windows_sys::Win32::Foundation::CloseHandle(ready);
                }
                return Err(error);
            }
        };
        let installer = unsafe {
            OpenProcess(
                SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                authentication.installer_pid,
            )
        };
        if installer.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(aborted);
                windows_sys::Win32::Foundation::CloseHandle(finished);
                windows_sys::Win32::Foundation::CloseHandle(ready);
            }
            return Err("uninstall_installer_identity_query_failed".to_string());
        }
        if process_creation_time_100ns(authentication.installer_pid)?
            != Some(authentication.installer_created_at_100ns)
        {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(installer);
                windows_sys::Win32::Foundation::CloseHandle(aborted);
                windows_sys::Win32::Foundation::CloseHandle(finished);
                windows_sys::Win32::Foundation::CloseHandle(ready);
            }
            return Err("uninstall_installer_identity_mismatch".to_string());
        }
        Ok(Self {
            ready,
            finished,
            aborted,
            installer,
        })
    }
}

#[cfg(target_os = "windows")]
impl UninstallGuardHandshake for NativeUninstallGuardHandshake {
    fn signal_ready(&self) -> Result<(), String> {
        use windows_sys::Win32::System::Threading::SetEvent;
        if unsafe { SetEvent(self.ready) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    fn wait_for_terminal(&self) -> Result<UninstallGuardWaitOutcomeV1, String> {
        use windows_sys::Win32::{
            Foundation::{WAIT_FAILED, WAIT_OBJECT_0},
            System::Threading::{WaitForMultipleObjects, INFINITE},
        };
        let handles = [self.finished, self.aborted, self.installer];
        let result =
            unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE) };
        match result {
            value if value == WAIT_OBJECT_0 => Ok(UninstallGuardWaitOutcomeV1::Finished),
            value if value == WAIT_OBJECT_0 + 1 => Ok(UninstallGuardWaitOutcomeV1::Aborted),
            value if value == WAIT_OBJECT_0 + 2 => Ok(UninstallGuardWaitOutcomeV1::InstallerExited),
            WAIT_FAILED => Err(std::io::Error::last_os_error().to_string()),
            _ => Err("uninstall_guard_wait_failed".to_string()),
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for NativeUninstallGuardHandshake {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        unsafe {
            CloseHandle(self.installer);
            CloseHandle(self.aborted);
            CloseHandle(self.finished);
            CloseHandle(self.ready);
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
struct NativeUninstallGuardHandshake;

#[cfg(not(target_os = "windows"))]
impl NativeUninstallGuardHandshake {
    fn new(_authentication: &GuardAuthenticationV1) -> Result<Self, String> {
        Err("uninstall_guard_windows_only".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EarlyProcessModeV1 {
    Collector(Vec<OsString>),
    DownloadResolver(Vec<OsString>),
    UninstallManagedData(PathBuf),
    UninstallGuard(UninstallGuardArgsV1),
    InstallerControl(InstallerControlCommandV1),
    PackageManifestBuild(Vec<OsString>),
    Gui,
}

pub trait UninstallSystem {
    fn read_run_value(&self) -> Result<Option<String>, String>;
    fn write_run_value(&self, value: Option<&str>) -> Result<(), String>;
    fn read_device_autostart(
        &self,
        coordination_dir: &Path,
    ) -> Result<DeviceAutostartSnapshotV1, String>;
    fn write_device_autostart(
        &self,
        coordination_dir: &Path,
        value: &DeviceAutostartSnapshotV1,
    ) -> Result<(), String>;
    fn process_creation_time_100ns(&self, pid: u32) -> Result<Option<u64>, String>;
    fn executable_product_identity(&self, path: &Path)
        -> Result<Option<ProductIdentityV1>, String>;
}

#[derive(Debug, Default)]
pub struct NativeUninstallSystem;

impl UninstallSystem for NativeUninstallSystem {
    fn read_run_value(&self) -> Result<Option<String>, String> {
        HkcuRunRegistry::emily().read_value()
    }

    fn write_run_value(&self, value: Option<&str>) -> Result<(), String> {
        let registry = HkcuRunRegistry::emily();
        match value {
            Some(value) => registry.write_value(value),
            None => registry.delete_value(),
        }
    }

    fn read_device_autostart(
        &self,
        device_settings_dir: &Path,
    ) -> Result<DeviceAutostartSnapshotV1, String> {
        let settings = DeviceSettingsStore::open(device_settings_dir.to_path_buf())?.load()?;
        Ok(DeviceAutostartSnapshotV1 {
            launch_at_startup: settings.launch_at_startup,
            silent_autostart: settings.silent_autostart,
        })
    }

    fn write_device_autostart(
        &self,
        device_settings_dir: &Path,
        value: &DeviceAutostartSnapshotV1,
    ) -> Result<(), String> {
        let store = DeviceSettingsStore::open(device_settings_dir.to_path_buf())?;
        let current = store.load()?;
        store.compare_and_swap(current.revision, |settings| {
            settings.launch_at_startup = value.launch_at_startup;
            settings.silent_autostart = value.silent_autostart;
            Ok(())
        })?;
        let observed = store.load()?;
        if observed.launch_at_startup != value.launch_at_startup
            || observed.silent_autostart != value.silent_autostart
        {
            return Err("uninstall_device_settings_verify_failed".to_string());
        }
        Ok(())
    }

    fn process_creation_time_100ns(&self, pid: u32) -> Result<Option<u64>, String> {
        process_creation_time_100ns(pid)
    }

    fn executable_product_identity(
        &self,
        path: &Path,
    ) -> Result<Option<ProductIdentityV1>, String> {
        if !path.exists() {
            return Ok(None);
        }
        read_embedded_executable_product_identity(path)
    }
}

pub fn compiled_product_identity() -> ProductIdentityV1 {
    ProductIdentityV1 {
        application_id: APPLICATION_ID.to_string(),
        product_name: PRODUCT_NAME.to_string(),
        build_id: COMPILED_BUILD_ID.to_string(),
        semantic_version: SEMANTIC_VERSION.to_string(),
        architecture: ARCHITECTURE.to_string(),
    }
}

pub fn read_embedded_executable_product_identity(
    path: &Path,
) -> Result<Option<ProductIdentityV1>, String> {
    let metadata =
        fs::metadata(path).map_err(|_| "uninstall_executable_identity_unavailable".to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_EXECUTABLE_IDENTITY_SCAN_BYTES {
        return Err("uninstall_executable_identity_unavailable".to_string());
    }
    let bytes =
        fs::read(path).map_err(|_| "uninstall_executable_identity_unavailable".to_string())?;
    let mut search_offset = 0usize;
    let mut observed: Option<ProductIdentityV1> = None;
    while let Some(relative_start) =
        find_subslice(&bytes[search_offset..], EXECUTABLE_IDENTITY_BEGIN)
    {
        let body_start = search_offset + relative_start + EXECUTABLE_IDENTITY_BEGIN.len();
        let Some(relative_end) = find_subslice(&bytes[body_start..], EXECUTABLE_IDENTITY_END)
        else {
            search_offset = body_start;
            continue;
        };
        let body = &bytes[body_start..body_start + relative_end];
        let fields = body.split(|byte| *byte == 0x1f).collect::<Vec<_>>();
        if fields.len() == 5 {
            let values = fields
                .iter()
                .map(|field| std::str::from_utf8(field))
                .collect::<Result<Vec<_>, _>>();
            if let Ok(values) = values {
                let candidate = ProductIdentityV1 {
                    application_id: values[0].to_string(),
                    product_name: values[1].to_string(),
                    build_id: values[2].to_string(),
                    semantic_version: values[3].to_string(),
                    architecture: values[4].to_string(),
                };
                if uninstall_manifest::validate_product_identity(&candidate).is_ok() {
                    if observed.as_ref().is_some_and(|value| value != &candidate) {
                        return Err("uninstall_executable_identity_ambiguous".to_string());
                    }
                    observed = Some(candidate);
                }
            }
        }
        search_offset = body_start + relative_end + EXECUTABLE_IDENTITY_END.len();
    }
    Ok(observed)
}

pub fn parse_early_process_mode(arguments: &[OsString]) -> Result<EarlyProcessModeV1, String> {
    if arguments.first().map(OsString::as_os_str) == Some(OsStr::new("--collector")) {
        return Ok(EarlyProcessModeV1::Collector(arguments[1..].to_vec()));
    }
    if arguments.first().map(OsString::as_os_str) == Some(OsStr::new("--download-resolver")) {
        return Ok(EarlyProcessModeV1::DownloadResolver(
            arguments[1..].to_vec(),
        ));
    }
    match parse_uninstall_managed_data_arguments(arguments) {
        Ok(Some(path)) => return Ok(EarlyProcessModeV1::UninstallManagedData(path)),
        Ok(None) => {}
        Err(error) => return Err(error.code().to_string()),
    }
    if arguments.first().map(OsString::as_os_str) == Some(OsStr::new("--uninstall-guard")) {
        return parse_guard_arguments(&arguments[1..]).map(EarlyProcessModeV1::UninstallGuard);
    }
    if arguments.first().map(OsString::as_os_str) == Some(OsStr::new("--installer-control")) {
        return parse_installer_control_arguments(&arguments[1..])
            .map(EarlyProcessModeV1::InstallerControl);
    }
    if arguments.first().map(OsString::as_os_str) == Some(OsStr::new("--package-manifest-build")) {
        if arguments.len() != 9 {
            return Err("package_manifest_builder_arguments_invalid".to_string());
        }
        return Ok(EarlyProcessModeV1::PackageManifestBuild(
            arguments[1..].to_vec(),
        ));
    }
    Ok(EarlyProcessModeV1::Gui)
}

pub fn run_package_manifest_builder(arguments: &[OsString]) -> Result<(), String> {
    uninstall_manifest::build_package_manifest(arguments, &compiled_product_identity())
}

pub fn run_uninstall_guard(arguments: &[OsString]) -> Result<(), String> {
    let args = parse_guard_arguments(arguments)?;
    let _locks = UninstallProcessGuard::acquire()?;
    let handshake = NativeUninstallGuardHandshake::new(&args.authentication)?;
    if let Some(request_path) = &args.prepare_request {
        let request = read_guard_prepare_request(request_path)?;
        if request.authentication != args.authentication {
            return Err("uninstall_guard_authentication_failed".to_string());
        }
        prepare_uninstall_session_with(
            &args.coordination_dir,
            request.into_prepare_request()?,
            &NativeUninstallSystem,
        )?;
        remove_file_if_exists(request_path)?;
    }
    supervise_uninstall_guard_with(
        &args.coordination_dir,
        &args.authentication,
        &NativeUninstallSystem,
        &handshake,
    )
}

pub fn run_installer_control(command: InstallerControlCommandV1) -> Result<String, String> {
    run_installer_control_with(
        command,
        &NativeUninstallSystem,
        &NativeInstallerControlRuntime,
    )
}

#[derive(Debug, Default)]
pub struct NativeInstallerControlRuntime;

impl InstallerControlRuntime for NativeInstallerControlRuntime {
    fn start_guard(
        &self,
        coordination_dir: &Path,
        authentication: &GuardAuthenticationV1,
        expected_guard_product: &ProductIdentityV1,
        prepare_request: Option<&GuardPrepareRequestV1>,
    ) -> Result<InstallerGuardProcessV1, String> {
        start_native_guard(
            coordination_dir,
            authentication,
            expected_guard_product,
            prepare_request,
        )
    }

    fn assert_guard_alive(&self, authentication: &GuardAuthenticationV1) -> Result<(), String> {
        assert_native_guard_alive(authentication)
    }

    fn prepare_for_destructive_commit(&self, _session: &UninstallSessionV1) -> Result<(), String> {
        Err("uninstall_runtime_cleanup_unavailable".to_string())
    }

    fn signal_guard_finished(
        &self,
        coordination_dir: &Path,
        authentication: &GuardAuthenticationV1,
    ) -> Result<(), String> {
        signal_native_guard_event(authentication, NativeGuardEventV1::Finished)?;
        cleanup_native_guard_files(coordination_dir, authentication)
    }

    fn signal_guard_aborted(
        &self,
        coordination_dir: &Path,
        authentication: &GuardAuthenticationV1,
    ) -> Result<(), String> {
        signal_native_guard_event(authentication, NativeGuardEventV1::Aborted)?;
        cleanup_native_guard_files(coordination_dir, authentication)
    }

    fn delete_managed_data(&self, bootstrap_dir: &Path) -> Result<(), String> {
        crate::data_root::uninstall_managed_data(bootstrap_dir)
            .map_err(|error| error.code().to_string())
    }
}

pub fn run_installer_control_with(
    command: InstallerControlCommandV1,
    system: &dyn UninstallSystem,
    runtime: &dyn InstallerControlRuntime,
) -> Result<String, String> {
    match command {
        InstallerControlCommandV1::Inspect { coordination_dir } => {
            inspect_installer_coordination_with(&coordination_dir, system)
                .map(|state| state.as_str().to_string())
        }
        InstallerControlCommandV1::AssertAlive(arguments) => {
            authenticate_installer_command(&arguments, system, runtime)?;
            Ok("alive".to_string())
        }
        InstallerControlCommandV1::Commit(arguments) => {
            let session = authenticate_installer_command(&arguments, system, runtime)?;
            if session.phase != UninstallPhaseV1::Prepared {
                return Err("uninstall_phase_transition_invalid".to_string());
            }
            runtime.prepare_for_destructive_commit(&session)?;
            transition_uninstall_to_destructive_with(
                &arguments.coordination_dir,
                &arguments.authentication,
                system,
            )?;
            Ok("committed".to_string())
        }
        InstallerControlCommandV1::DeleteManagedData(arguments) => {
            let session = authenticate_installer_command(&arguments, system, runtime)?;
            if session.phase != UninstallPhaseV1::DestructiveCommitted {
                return Err("uninstall_phase_transition_invalid".to_string());
            }
            match session.user_data_disposition {
                UserDataDispositionV1::Retain => Ok("data-retained".to_string()),
                UserDataDispositionV1::Delete => {
                    if is_uninitialized_session(&session) {
                        validate_uninitialized_data_absent(&session.device_settings_dir)?;
                    } else {
                        runtime.delete_managed_data(&session.device_settings_dir)?;
                    }
                    Ok("data-deleted".to_string())
                }
            }
        }
        InstallerControlCommandV1::CompleteUninstall(arguments) => {
            authenticate_installer_command(&arguments, system, runtime)?;
            complete_uninstall_with(
                &arguments.coordination_dir,
                &arguments.authentication,
                UninstallCompletionV1::Uninstalled,
                system,
            )?;
            runtime
                .signal_guard_finished(&arguments.coordination_dir, &arguments.authentication)?;
            Ok("completed".to_string())
        }
        InstallerControlCommandV1::CompleteRepair {
            authentication,
            package_manifest,
            package_manifest_sha256,
        } => {
            authenticate_installer_command(&authentication, system, runtime)?;
            let bytes =
                read_installer_file(&package_manifest, "uninstall_candidate_manifest_invalid")?;
            complete_repair_with_candidate_manifest(
                &authentication.coordination_dir,
                &authentication.authentication,
                &bytes,
                &package_manifest_sha256,
                system,
            )?;
            runtime.signal_guard_finished(
                &authentication.coordination_dir,
                &authentication.authentication,
            )?;
            Ok("repair-completed".to_string())
        }
        InstallerControlCommandV1::Abort(arguments) => {
            let session = authenticate_installer_command(&arguments, system, runtime)?;
            let output = if matches!(
                session.phase,
                UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared
            ) {
                abort_uninstall_before_commit_with(
                    &arguments.coordination_dir,
                    &arguments.authentication,
                    system,
                )?;
                "aborted"
            } else if session.phase == UninstallPhaseV1::DestructiveCommitted {
                "repair-required"
            } else {
                return Err("uninstall_phase_transition_invalid".to_string());
            };
            runtime.signal_guard_aborted(&arguments.coordination_dir, &arguments.authentication)?;
            Ok(output.to_string())
        }
        InstallerControlCommandV1::Prepare {
            coordination_dir,
            bootstrap_dir,
            install_dir,
            main_executable,
            package_manifest,
            package_manifest_sha256,
            user_data_disposition,
            installer_pid,
        } => {
            let bootstrap = read_installer_bootstrap(&bootstrap_dir)?;
            let manifest_bytes =
                read_installer_file(&package_manifest, "uninstall_package_manifest_invalid")?;
            let manifest = parse_package_manifest(&manifest_bytes, &package_manifest_sha256)?;
            let authentication = create_installer_authentication(installer_pid, system)?;
            let (root_id, protected_paths) = match bootstrap.as_ref() {
                Some(bootstrap) => {
                    if bootstrap.current_root.root_id == UNINITIALIZED_ROOT_ID {
                        return Err("uninstall_bootstrap_invalid".to_string());
                    }
                    (
                        bootstrap.current_root.root_id.clone(),
                        installer_protected_paths(&coordination_dir, &bootstrap_dir, bootstrap)?,
                    )
                }
                None => (
                    UNINITIALIZED_ROOT_ID.to_string(),
                    uninitialized_protected_paths(&coordination_dir, &bootstrap_dir)?,
                ),
            };
            let request = GuardPrepareRequestV1 {
                schema_version: CONTRACT_SCHEMA_VERSION,
                authentication: authentication.clone(),
                root_id,
                executable_identity: ExecutableIdentityV1 {
                    install_dir,
                    main_executable,
                    product: manifest.product_identity(),
                },
                device_settings_dir: bootstrap_dir,
                protected_paths,
                user_data_disposition,
                package_manifest_bytes: manifest_bytes,
                expected_package_manifest_sha256: package_manifest_sha256,
            };
            let guard_process = runtime.start_guard(
                &coordination_dir,
                &authentication,
                &compiled_product_identity(),
                Some(&request),
            )?;
            authenticate_uninstall_guard(&coordination_dir, &authentication, system)?;
            write_installer_session_output(&coordination_dir, &authentication, &guard_process)
        }
        InstallerControlCommandV1::Takeover {
            coordination_dir,
            install_dir,
            main_executable,
            package_manifest,
            package_manifest_sha256,
            action,
            installer_pid,
        } => {
            let candidate_bytes =
                read_installer_file(&package_manifest, "uninstall_candidate_manifest_invalid")?;
            let candidate_manifest =
                parse_package_manifest(&candidate_bytes, &package_manifest_sha256)?;
            let mut session = read_session(&coordination_dir)?
                .ok_or_else(|| "uninstall_session_missing".to_string())?;
            let interrupted_path = interrupted_fault_path(&coordination_dir);
            let interrupted_bytes =
                read_installer_file(&interrupted_path, "uninstall_fault_invalid")?;
            let interrupted: UninstallInterruptedFaultV1 =
                serde_json::from_slice(&interrupted_bytes)
                    .map_err(|_| "uninstall_fault_invalid".to_string())?;
            validate_interrupted_fault(&interrupted)?;
            if interrupted.nonce != session.authentication.nonce {
                return Err("uninstall_fault_invalid".to_string());
            }
            verify_package_manifest_files(
                &session.package_manifest,
                &session.install_dir,
                ManifestVerificationModeV1::AllowMissing,
            )?;
            let observed_old_product =
                system.executable_product_identity(&session.main_executable)?;
            let candidate = ExecutableIdentityV1 {
                install_dir,
                main_executable,
                product: candidate_manifest.product_identity(),
            };
            validate_takeover(
                &session,
                &candidate,
                observed_old_product.as_ref(),
                &session.root_id,
                &session.protected_paths,
                session.user_data_disposition,
                action,
            )?;
            let authentication = create_installer_authentication(installer_pid, system)?;
            let old_session = session.clone();
            session.authentication = authentication.clone();
            session.updated_at = now_iso();
            write_session(&coordination_dir, &session)?;
            let start =
                runtime.start_guard(&coordination_dir, &authentication, &candidate.product, None);
            let guard_process = match start {
                Ok(path) => path,
                Err(error) => {
                    write_session(&coordination_dir, &old_session)?;
                    return Err(error);
                }
            };
            if let Err(error) = remove_file_if_exists(&interrupted_path) {
                let _ = runtime.signal_guard_aborted(&coordination_dir, &authentication);
                write_session(&coordination_dir, &old_session)?;
                return Err(error);
            }
            authenticate_uninstall_guard(&coordination_dir, &authentication, system)?;
            write_installer_session_output(&coordination_dir, &authentication, &guard_process)
        }
    }
}

fn write_installer_session_output(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    guard_process: &InstallerGuardProcessV1,
) -> Result<String, String> {
    validate_installer_absolute_path(&guard_process.control_executable)?;
    validate_installer_absolute_path(&guard_process.guard_executable)?;
    let output = serde_json::json!({
        "schemaVersion": CONTRACT_SCHEMA_VERSION,
        "sessionNonce": authentication.nonce,
        "installerPid": authentication.installer_pid,
        "installerCreatedAt100ns": authentication.installer_created_at_100ns,
        "controlExecutable": guard_process.control_executable,
        "guardExecutable": guard_process.guard_executable,
    });
    write_json_atomically(&coordination_dir.join(INSTALLER_SESSION_FILE), &output)?;
    write_bytes_atomically(
        &coordination_dir.join(INSTALLER_SESSION_ARGS_FILE),
        format!(
            "{}\n{}\n{}\n{}\n{}\n",
            authentication.nonce,
            authentication.installer_pid,
            authentication.installer_created_at_100ns,
            guard_process.control_executable.display(),
            guard_process.guard_executable.display(),
        )
        .as_bytes(),
    )?;
    serde_json::to_string(&output).map_err(|error| error.to_string())
}

fn create_installer_authentication(
    installer_pid: u32,
    system: &dyn UninstallSystem,
) -> Result<GuardAuthenticationV1, String> {
    let installer_created_at_100ns = system
        .process_creation_time_100ns(installer_pid)
        .map_err(|_| "uninstall_installer_identity_query_failed".to_string())?
        .ok_or_else(|| "uninstall_installer_identity_mismatch".to_string())?;
    let authentication = GuardAuthenticationV1 {
        nonce: Uuid::new_v4().simple().to_string(),
        installer_pid,
        installer_created_at_100ns,
    };
    validate_authentication(&authentication)?;
    Ok(authentication)
}

fn read_installer_bootstrap(bootstrap_dir: &Path) -> Result<Option<BootstrapV1>, String> {
    validate_installer_absolute_path(bootstrap_dir)?;
    let metadata = match fs::symlink_metadata(bootstrap_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("uninstall_bootstrap_invalid".to_string()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("uninstall_bootstrap_invalid".to_string());
    }
    read_bootstrap(bootstrap_dir)
        .map_err(|_| "uninstall_bootstrap_invalid".to_string())?
        .map(Some)
        .ok_or_else(|| "uninstall_bootstrap_missing".to_string())
}

fn uninitialized_protected_paths(
    coordination_dir: &Path,
    bootstrap_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    validate_uninitialized_data_absent(bootstrap_dir)?;
    let mut paths = vec![coordination_dir.to_path_buf(), bootstrap_dir.to_path_buf()];
    paths.sort_by_key(|path| normalized_path(path));
    normalize_paths(&paths)?;
    Ok(paths)
}

fn installer_protected_paths(
    coordination_dir: &Path,
    bootstrap_dir: &Path,
    bootstrap: &BootstrapV1,
) -> Result<Vec<PathBuf>, String> {
    let mut by_normalized = std::collections::BTreeMap::new();
    for path in std::iter::once(coordination_dir)
        .chain(std::iter::once(bootstrap_dir))
        .chain(std::iter::once(bootstrap.current_root.path.as_path()))
        .chain(
            bootstrap
                .retained_data_roots
                .iter()
                .map(|retained| retained.root.path.as_path()),
        )
    {
        validate_installer_absolute_path(path)?;
        by_normalized
            .entry(normalized_path(path))
            .or_insert_with(|| path.to_path_buf());
    }
    let paths = by_normalized.into_values().collect::<Vec<_>>();
    normalize_paths(&paths)?;
    Ok(paths)
}

fn authenticate_installer_command(
    arguments: &InstallerAuthenticationArgsV1,
    system: &dyn UninstallSystem,
    runtime: &dyn InstallerControlRuntime,
) -> Result<UninstallSessionV1, String> {
    runtime.assert_guard_alive(&arguments.authentication)?;
    authenticate_uninstall_guard(
        &arguments.coordination_dir,
        &arguments.authentication,
        system,
    )
}

pub fn inspect_installer_coordination_with(
    coordination_dir: &Path,
    system: &dyn UninstallSystem,
) -> Result<InstallerCoordinationStateV1, String> {
    validate_coordination_dir(coordination_dir)?;
    let session = read_session(coordination_dir)?;
    let cleanup_fault = cleanup_fault_path(coordination_dir).is_file();
    let interrupted_fault = interrupted_fault_path(coordination_dir).is_file();
    let Some(session) = session else {
        if cleanup_fault || interrupted_fault {
            return Err("uninstall_fault_invalid".to_string());
        }
        return Ok(InstallerCoordinationStateV1::Clean);
    };
    if cleanup_fault {
        let fault =
            read_valid_fault::<AutostartCleanupFaultV1>(&cleanup_fault_path(coordination_dir))?;
        validate_cleanup_fault(&fault)?;
        if fault.nonce != session.authentication.nonce || fault.phase != session.phase {
            return Err("uninstall_fault_invalid".to_string());
        }
        write_and_verify_session_device(system, &session, &DeviceAutostartSnapshotV1::default())?;
        write_and_verify_run(system, None)?;
        return Err("autostart_cleanup_fault".to_string());
    }
    if interrupted_fault {
        let fault = read_valid_fault::<UninstallInterruptedFaultV1>(&interrupted_fault_path(
            coordination_dir,
        ))?;
        validate_interrupted_fault(&fault)?;
        if fault.nonce != session.authentication.nonce
            || session.phase != UninstallPhaseV1::DestructiveCommitted
        {
            return Err("uninstall_fault_invalid".to_string());
        }
        return Ok(InstallerCoordinationStateV1::RepairRequired);
    }
    match session.phase {
        UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared => {
            recover_uninstall_state_before_bootstrap_with(
                coordination_dir,
                &session.executable_identity(),
                system,
            )?;
            Ok(InstallerCoordinationStateV1::RolledBack)
        }
        UninstallPhaseV1::DestructiveCommitted => {
            write_interrupted_fault(coordination_dir, &session, "uninstall_interrupted")?;
            Ok(InstallerCoordinationStateV1::RepairRequired)
        }
        UninstallPhaseV1::Completed | UninstallPhaseV1::RepairCompleted => {
            cleanup_terminal_evidence(coordination_dir)?;
            Ok(InstallerCoordinationStateV1::Clean)
        }
    }
}

pub fn uninstall_coordination_dir(local_app_data: &Path) -> Result<PathBuf, String> {
    if !local_app_data.is_absolute()
        || local_app_data.to_str().is_none()
        || is_drive_root(local_app_data)
        || has_ambiguous_path_components(local_app_data)
    {
        return Err("uninstall_coordination_parent_invalid".to_string());
    }
    Ok(local_app_data.join("LifeArchiveProject/Emily/Uninstall"))
}

pub fn uninstall_guard_event_names(nonce: &str) -> Result<UninstallGuardEventNamesV1, String> {
    if !valid_simple_id(nonce) {
        return Err("uninstall_guard_authentication_invalid".to_string());
    }
    let prefix = format!("Local\\LifeArchiveProject.Emily.Uninstall.{nonce}");
    Ok(UninstallGuardEventNamesV1 {
        ready: format!("{prefix}.Ready"),
        finished: format!("{prefix}.Finished"),
        aborted: format!("{prefix}.Aborted"),
    })
}

pub fn supervise_uninstall_guard_with(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    system: &dyn UninstallSystem,
    handshake: &dyn UninstallGuardHandshake,
) -> Result<(), String> {
    authenticate_uninstall_guard(coordination_dir, authentication, system)?;
    handshake.signal_ready()?;
    match handshake.wait_for_terminal()? {
        UninstallGuardWaitOutcomeV1::Finished => {
            let Some(session) = read_session(coordination_dir)? else {
                return Ok(());
            };
            if session.authentication != *authentication {
                return Err("uninstall_guard_authentication_failed".to_string());
            }
            if matches!(
                session.phase,
                UninstallPhaseV1::Completed | UninstallPhaseV1::RepairCompleted
            ) {
                cleanup_terminal_evidence(coordination_dir)
            } else {
                Err("uninstall_guard_finished_before_terminal".to_string())
            }
        }
        outcome => {
            let reason = match outcome {
                UninstallGuardWaitOutcomeV1::Aborted => "uninstall_aborted",
                UninstallGuardWaitOutcomeV1::InstallerExited => "uninstall_installer_exited",
                UninstallGuardWaitOutcomeV1::Finished => unreachable!(),
            };
            let session = read_session(coordination_dir)?.ok_or_else(|| reason.to_string())?;
            if session.authentication != *authentication {
                return Err("uninstall_guard_authentication_failed".to_string());
            }
            match session.phase {
                UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared => {
                    rollback_autostart(coordination_dir, &session, system)?;
                    cleanup_terminal_evidence(coordination_dir)?;
                    Err(reason.to_string())
                }
                UninstallPhaseV1::DestructiveCommitted => {
                    write_interrupted_fault(coordination_dir, &session, reason)?;
                    Err("uninstall_repair_required".to_string())
                }
                UninstallPhaseV1::Completed | UninstallPhaseV1::RepairCompleted => {
                    cleanup_terminal_evidence(coordination_dir)
                }
            }
        }
    }
}

pub fn prepare_uninstall_session_with(
    coordination_dir: &Path,
    request: UninstallPrepareRequestV1,
    system: &dyn UninstallSystem,
) -> Result<UninstallSessionV1, String> {
    validate_coordination_dir(coordination_dir)?;
    reject_existing_uninstall_evidence(coordination_dir)?;
    validate_authentication(&request.authentication)?;
    verify_installer_process(&request.authentication, system)?;
    validate_root_id(&request.root_id)?;
    validate_executable_identity(&request.executable_identity)?;
    let manifest = parse_package_manifest(
        &request.package_manifest_bytes,
        &request.expected_package_manifest_sha256,
    )?;
    if manifest.product_identity() != request.executable_identity.product {
        return Err("uninstall_package_identity_mismatch".to_string());
    }
    validate_protected_paths(
        coordination_dir,
        &request.executable_identity.install_dir,
        &manifest,
        &request.protected_paths,
    )?;
    verify_package_manifest_files(
        &manifest,
        &request.executable_identity.install_dir,
        ManifestVerificationModeV1::RequireAll,
    )?;
    verify_old_executable_identity(&request.executable_identity, system, true)?;
    let old_run_value = system
        .read_run_value()
        .map_err(|_| "uninstall_run_read_failed".to_string())?;
    let uninitialized = request.root_id == UNINITIALIZED_ROOT_ID;
    let old_device_autostart = if uninitialized {
        validate_uninitialized_data_absent(&request.device_settings_dir)?;
        DeviceAutostartSnapshotV1::default()
    } else {
        system
            .read_device_autostart(&request.device_settings_dir)
            .map_err(|_| "uninstall_device_settings_read_failed".to_string())?
    };
    let now = now_iso();
    let mut session = UninstallSessionV1 {
        schema_version: CONTRACT_SCHEMA_VERSION,
        authentication: request.authentication,
        root_id: request.root_id,
        old_run_value,
        old_device_autostart,
        phase: UninstallPhaseV1::Preparing,
        device_settings_dir: request.device_settings_dir,
        install_dir: request.executable_identity.install_dir,
        main_executable: request.executable_identity.main_executable,
        application_id: manifest.application_id.clone(),
        product_name: manifest.product_name.clone(),
        old_build_id: manifest.build_id.clone(),
        old_semantic_version: manifest.semantic_version.clone(),
        architecture: manifest.architecture.clone(),
        protected_paths: request.protected_paths,
        user_data_disposition: request.user_data_disposition,
        package_manifest: manifest,
        package_manifest_sha256: request.expected_package_manifest_sha256,
        prepared_at: now.clone(),
        updated_at: now,
    };
    validate_session(coordination_dir, &session)?;
    write_session(coordination_dir, &session)?;

    if !uninitialized {
        let disabled = DeviceAutostartSnapshotV1::default();
        if let Err(error) = write_and_verify_device(system, &session.device_settings_dir, &disabled)
        {
            remove_file_if_exists(&session_path(coordination_dir))?;
            return Err(error);
        }
    }
    if let Err(error) = write_and_verify_run(system, None) {
        return fail_preparing_with_rollback(coordination_dir, &session, system, error);
    }
    session.phase = UninstallPhaseV1::Prepared;
    session.updated_at = now_iso();
    write_session(coordination_dir, &session)?;
    Ok(session)
}

pub fn authenticate_uninstall_guard(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    system: &dyn UninstallSystem,
) -> Result<UninstallSessionV1, String> {
    validate_authentication(authentication)?;
    let session =
        read_session(coordination_dir)?.ok_or_else(|| "uninstall_session_missing".to_string())?;
    if session.authentication != *authentication {
        return Err("uninstall_guard_authentication_failed".to_string());
    }
    verify_installer_process(authentication, system)?;
    Ok(session)
}

pub fn transition_uninstall_to_destructive_with(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    system: &dyn UninstallSystem,
) -> Result<UninstallSessionV1, String> {
    let mut session = authenticate_uninstall_guard(coordination_dir, authentication, system)?;
    if session.phase != UninstallPhaseV1::Prepared {
        return Err("uninstall_phase_transition_invalid".to_string());
    }
    validate_session(coordination_dir, &session)?;
    verify_package_manifest_files(
        &session.package_manifest,
        &session.install_dir,
        ManifestVerificationModeV1::RequireAll,
    )?;
    verify_old_executable_identity(&session.executable_identity(), system, true)?;
    session.phase = UninstallPhaseV1::DestructiveCommitted;
    session.updated_at = now_iso();
    write_session(coordination_dir, &session)?;
    Ok(session)
}

pub fn abort_uninstall_before_commit_with(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    system: &dyn UninstallSystem,
) -> Result<(), String> {
    let session = authenticate_uninstall_guard(coordination_dir, authentication, system)?;
    if !matches!(
        session.phase,
        UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared
    ) {
        return Err("uninstall_rollback_after_commit_forbidden".to_string());
    }
    rollback_autostart(coordination_dir, &session, system)?;
    cleanup_terminal_evidence(coordination_dir)
}

pub fn complete_uninstall_with(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    completion: UninstallCompletionV1,
    system: &dyn UninstallSystem,
) -> Result<(), String> {
    if completion == UninstallCompletionV1::Repaired {
        return Err("uninstall_repair_candidate_manifest_required".to_string());
    }
    let mut session = authenticate_uninstall_guard(coordination_dir, authentication, system)?;
    if session.phase != UninstallPhaseV1::DestructiveCommitted {
        return Err("uninstall_phase_transition_invalid".to_string());
    }
    if session.package_manifest.files.iter().any(|entry| {
        relative_manifest_path(&entry.path)
            .map(|relative| session.install_dir.join(relative).exists())
            .unwrap_or(true)
    }) {
        return Err("uninstall_package_files_remain".to_string());
    }
    session.phase = match completion {
        UninstallCompletionV1::Uninstalled => UninstallPhaseV1::Completed,
        UninstallCompletionV1::Repaired => unreachable!(),
    };
    session.updated_at = now_iso();
    write_session(coordination_dir, &session)?;
    cleanup_terminal_evidence(coordination_dir)
}

pub fn complete_repair_with_candidate_manifest(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    candidate_manifest_bytes: &[u8],
    expected_candidate_manifest_sha256: &str,
    system: &dyn UninstallSystem,
) -> Result<(), String> {
    let mut session = authenticate_uninstall_guard(coordination_dir, authentication, system)?;
    if session.phase != UninstallPhaseV1::DestructiveCommitted {
        return Err("uninstall_phase_transition_invalid".to_string());
    }
    let candidate =
        parse_package_manifest(candidate_manifest_bytes, expected_candidate_manifest_sha256)?;
    if candidate.application_id != session.application_id
        || candidate.product_name != session.product_name
        || candidate.architecture != session.architecture
    {
        return Err("uninstall_candidate_identity_mismatch".to_string());
    }
    verify_package_manifest_files(
        &candidate,
        &session.install_dir,
        ManifestVerificationModeV1::RequireAll,
    )?;
    verify_old_executable_identity(
        &ExecutableIdentityV1 {
            install_dir: session.install_dir.clone(),
            main_executable: session.main_executable.clone(),
            product: candidate.product_identity(),
        },
        system,
        true,
    )?;
    write_and_verify_session_device(system, &session, &DeviceAutostartSnapshotV1::default())?;
    write_and_verify_run(system, None)?;
    session.phase = UninstallPhaseV1::RepairCompleted;
    session.updated_at = now_iso();
    write_session(coordination_dir, &session)?;
    cleanup_terminal_evidence(coordination_dir)
}

pub fn recover_uninstall_state_before_bootstrap(
    coordination_dir: &Path,
    executable_identity: &ExecutableIdentityV1,
) -> Result<UninstallRecoveryOutcomeV1, String> {
    recover_uninstall_state_before_bootstrap_with(
        coordination_dir,
        executable_identity,
        &NativeUninstallSystem,
    )
}

pub fn recover_uninstall_state_before_bootstrap_with(
    coordination_dir: &Path,
    executable_identity: &ExecutableIdentityV1,
    system: &dyn UninstallSystem,
) -> Result<UninstallRecoveryOutcomeV1, String> {
    validate_coordination_dir(coordination_dir)?;
    if cleanup_fault_path(coordination_dir).is_file() {
        let fault =
            read_valid_fault::<AutostartCleanupFaultV1>(&cleanup_fault_path(coordination_dir))?;
        validate_cleanup_fault(&fault)?;
        let session =
            read_session(coordination_dir)?.ok_or_else(|| "uninstall_fault_invalid".to_string())?;
        if fault.nonce != session.authentication.nonce || fault.phase != session.phase {
            return Err("uninstall_fault_invalid".to_string());
        }
        write_and_verify_session_device(system, &session, &DeviceAutostartSnapshotV1::default())?;
        write_and_verify_run(system, None)?;
        return Err("autostart_cleanup_fault".to_string());
    }
    if interrupted_fault_path(coordination_dir).is_file() {
        let fault = read_valid_fault::<UninstallInterruptedFaultV1>(&interrupted_fault_path(
            coordination_dir,
        ))?;
        validate_interrupted_fault(&fault)?;
        let session =
            read_session(coordination_dir)?.ok_or_else(|| "uninstall_fault_invalid".to_string())?;
        if fault.nonce != session.authentication.nonce
            || session.phase != UninstallPhaseV1::DestructiveCommitted
        {
            return Err("uninstall_fault_invalid".to_string());
        }
        return Err("uninstall_repair_required".to_string());
    }
    let Some(session) = read_session(coordination_dir)? else {
        return Ok(UninstallRecoveryOutcomeV1::Ready);
    };
    validate_session(coordination_dir, &session)?;
    let session_identity = session.executable_identity();
    let same_installation_path = same_path(&session.install_dir, &executable_identity.install_dir)
        && same_path(
            &session.main_executable,
            &executable_identity.main_executable,
        );
    let same_product_family = session.application_id == executable_identity.product.application_id
        && session.product_name == executable_identity.product.product_name
        && session.architecture == executable_identity.product.architecture;
    let terminal_phase = matches!(
        session.phase,
        UninstallPhaseV1::Completed | UninstallPhaseV1::RepairCompleted
    );
    let requires_exact_old_identity = !terminal_phase;
    if !same_installation_path
        || !same_product_family
        || (requires_exact_old_identity && session_identity.product != executable_identity.product)
    {
        return Err("uninstall_current_executable_mismatch".to_string());
    }
    let require_old_executable = matches!(
        session.phase,
        UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared
    );
    if require_old_executable {
        verify_old_executable_identity(&session_identity, system, true)?;
    }
    if !terminal_phase {
        verify_package_manifest_files(
            &session.package_manifest,
            &session.install_dir,
            if require_old_executable {
                ManifestVerificationModeV1::RequireAll
            } else {
                ManifestVerificationModeV1::AllowMissing
            },
        )?;
    }
    match session.phase {
        UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared => {
            rollback_autostart(coordination_dir, &session, system)?;
            cleanup_terminal_evidence(coordination_dir)?;
            Ok(UninstallRecoveryOutcomeV1::RolledBack)
        }
        UninstallPhaseV1::DestructiveCommitted => {
            write_interrupted_fault(coordination_dir, &session, "uninstall_interrupted")?;
            Err("uninstall_repair_required".to_string())
        }
        UninstallPhaseV1::Completed | UninstallPhaseV1::RepairCompleted => {
            cleanup_terminal_evidence(coordination_dir)?;
            Ok(UninstallRecoveryOutcomeV1::TerminalCleaned)
        }
    }
}

pub fn validate_takeover(
    session: &UninstallSessionV1,
    candidate: &ExecutableIdentityV1,
    observed_old_product: Option<&ProductIdentityV1>,
    root_id: &str,
    protected_paths: &[PathBuf],
    user_data_disposition: UserDataDispositionV1,
    _action: TakeoverActionV1,
) -> Result<(), String> {
    if session.phase != UninstallPhaseV1::DestructiveCommitted {
        return Err("uninstall_takeover_not_committed".to_string());
    }
    validate_executable_identity(candidate)?;
    let old = session.executable_identity();
    validate_executable_identity(&old)?;
    if !same_path(&candidate.install_dir, &old.install_dir)
        || !same_path(&candidate.main_executable, &old.main_executable)
    {
        return Err("uninstall_candidate_path_mismatch".to_string());
    }
    if root_id != session.root_id {
        return Err("uninstall_takeover_root_mismatch".to_string());
    }
    if candidate.product.application_id != old.product.application_id
        || candidate.product.product_name != old.product.product_name
        || candidate.product.architecture != old.product.architecture
    {
        return Err("uninstall_candidate_identity_mismatch".to_string());
    }
    if observed_old_product.is_some_and(|observed| observed != &old.product) {
        return Err("uninstall_old_executable_identity_mismatch".to_string());
    }
    if normalize_paths(protected_paths)? != normalize_paths(&session.protected_paths)?
        || user_data_disposition != session.user_data_disposition
    {
        return Err("uninstall_takeover_policy_mismatch".to_string());
    }
    validate_session_without_coordination(session)
}

pub fn validate_manifest_delete_targets(
    manifest: &PackageManifestV1,
    install_dir: &Path,
    protected_paths: &[PathBuf],
    targets: &[PathBuf],
) -> Result<(), String> {
    validate_package_manifest(manifest)?;
    if !install_dir.is_absolute()
        || has_ambiguous_path_components(install_dir)
        || targets.is_empty()
    {
        return Err("uninstall_delete_target_invalid".to_string());
    }
    let protected = normalize_paths(protected_paths)?;
    let owned = manifest
        .files
        .iter()
        .map(|entry| {
            Ok::<_, String>(normalized_path(
                &install_dir.join(relative_manifest_path(&entry.path)?),
            ))
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    for owned_path in &owned {
        if protected
            .iter()
            .any(|value| paths_overlap(owned_path, value))
        {
            return Err("uninstall_manifest_protected_overlap".to_string());
        }
    }
    let mut seen = BTreeSet::new();
    for target in targets {
        if !target.is_absolute() || has_ambiguous_path_components(target) {
            return Err("uninstall_delete_target_invalid".to_string());
        }
        let normalized = normalized_path(target);
        if !seen.insert(normalized.clone())
            || !owned.contains(&normalized)
            || protected
                .iter()
                .any(|value| paths_overlap(&normalized, value))
        {
            return Err("uninstall_delete_target_not_owned".to_string());
        }
    }
    Ok(())
}

pub fn validate_installer_object_removals(
    phase: UninstallPhaseV1,
    install_dir: &Path,
    objects: &[InstallerOwnedObjectV1],
) -> Result<(), String> {
    if phase != UninstallPhaseV1::DestructiveCommitted
        || !install_dir.is_absolute()
        || has_ambiguous_path_components(install_dir)
        || objects.is_empty()
    {
        return Err("installer_object_removal_not_committed".to_string());
    }
    const UNINSTALL_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\lifearchive.douyin-history-archive";
    let shortcut_suffix = format!(
        r"\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\{PRODUCT_NAME}.lnk"
    )
    .to_lowercase();
    for object in objects {
        match object {
            InstallerOwnedObjectV1::StartMenuShortcut { path } => {
                if !path.is_absolute()
                    || !normalized_path(path).ends_with(&shortcut_suffix)
                {
                    return Err("installer_object_not_whitelisted".to_string());
                }
            }
            InstallerOwnedObjectV1::UninstallRegistryValue { key, value_name } => {
                const VALUES: [&str; 10] = [
                    "DisplayName",
                    "DisplayVersion",
                    "Publisher",
                    "UninstallString",
                    "QuietUninstallString",
                    "DisplayIcon",
                    "InstallLocation",
                    "NoModify",
                    "NoRepair",
                    "EstimatedSize",
                ];
                if key != UNINSTALL_KEY || !VALUES.contains(&value_name.as_str()) {
                    return Err("installer_object_not_whitelisted".to_string());
                }
            }
            InstallerOwnedObjectV1::SelfUninstaller { path } => {
                if !path.is_absolute()
                    || has_ambiguous_path_components(path)
                    || !same_path(path, &install_dir.join("uninstall.exe"))
                    || path.file_name().and_then(|value| value.to_str()) != Some("uninstall.exe")
                {
                    return Err("installer_object_not_whitelisted".to_string());
                }
            }
        }
    }
    Ok(())
}

fn parse_guard_arguments(arguments: &[OsString]) -> Result<UninstallGuardArgsV1, String> {
    if arguments.len() != 10
        || arguments[0] != "--coordination-dir"
        || arguments[2] != "--prepare-request"
        || arguments[4] != "--session-nonce"
        || arguments[6] != "--installer-pid"
        || arguments[8] != "--installer-created-at-100ns"
    {
        return Err("uninstall_guard_arguments_invalid".to_string());
    }
    let coordination_dir = PathBuf::from(&arguments[1]);
    let prepare_request = arguments[3]
        .to_str()
        .ok_or_else(|| "uninstall_guard_arguments_invalid".to_string())?;
    let prepare_request = if prepare_request == "-" {
        None
    } else {
        let path = PathBuf::from(prepare_request);
        validate_installer_absolute_path(&path)
            .map_err(|_| "uninstall_guard_arguments_invalid".to_string())?;
        Some(path)
    };
    let nonce = arguments[5]
        .to_str()
        .ok_or_else(|| "uninstall_guard_arguments_invalid".to_string())?
        .to_string();
    let installer_pid = parse_decimal_u32(&arguments[7])?;
    let installer_created_at_100ns = parse_decimal_u64(&arguments[9])?;
    validate_coordination_dir_path(&coordination_dir)?;
    let authentication = GuardAuthenticationV1 {
        nonce,
        installer_pid,
        installer_created_at_100ns,
    };
    validate_authentication(&authentication)?;
    Ok(UninstallGuardArgsV1 {
        coordination_dir,
        prepare_request,
        authentication,
    })
}

fn parse_installer_control_arguments(
    arguments: &[OsString],
) -> Result<InstallerControlCommandV1, String> {
    let Some(command) = arguments.first().and_then(|value| value.to_str()) else {
        return Err("installer_control_arguments_invalid".to_string());
    };
    match command {
        "inspect" => {
            if arguments.len() != 3 || arguments[1] != "--coordination-dir" {
                return Err("installer_control_arguments_invalid".to_string());
            }
            let coordination_dir = installer_path(&arguments[2])?;
            validate_coordination_dir_path(&coordination_dir)?;
            Ok(InstallerControlCommandV1::Inspect { coordination_dir })
        }
        "prepare" => parse_prepare_installer_control(&arguments[1..]),
        "takeover" => parse_takeover_installer_control(&arguments[1..]),
        "assert-alive" | "commit" | "delete-managed-data" | "complete-uninstall" | "abort" => {
            let authentication = parse_installer_authentication(&arguments[1..])?;
            Ok(match command {
                "assert-alive" => InstallerControlCommandV1::AssertAlive(authentication),
                "commit" => InstallerControlCommandV1::Commit(authentication),
                "delete-managed-data" => {
                    InstallerControlCommandV1::DeleteManagedData(authentication)
                }
                "complete-uninstall" => {
                    InstallerControlCommandV1::CompleteUninstall(authentication)
                }
                "abort" => InstallerControlCommandV1::Abort(authentication),
                _ => unreachable!(),
            })
        }
        "complete-repair" => parse_complete_repair_installer_control(&arguments[1..]),
        _ => Err("installer_control_arguments_invalid".to_string()),
    }
}

fn parse_prepare_installer_control(
    arguments: &[OsString],
) -> Result<InstallerControlCommandV1, String> {
    const KEYS: [&str; 8] = [
        "--coordination-dir",
        "--bootstrap-dir",
        "--install-dir",
        "--main-executable",
        "--package-manifest",
        "--package-manifest-sha256",
        "--data-disposition",
        "--installer-pid",
    ];
    let values = exact_installer_values(arguments, &KEYS)?;
    let coordination_dir = installer_path(values[0])?;
    let bootstrap_dir = installer_path(values[1])?;
    let install_dir = installer_path(values[2])?;
    let main_executable = installer_path(values[3])?;
    let package_manifest = installer_path(values[4])?;
    let package_manifest_sha256 = installer_sha256(values[5])?;
    let user_data_disposition = installer_data_disposition(values[6])?;
    let installer_pid = installer_u32(values[7])?;
    validate_coordination_dir_path(&coordination_dir)?;
    validate_installer_absolute_path(&bootstrap_dir)?;
    validate_installer_absolute_path(&install_dir)?;
    validate_installer_absolute_path(&main_executable)?;
    validate_installer_absolute_path(&package_manifest)?;
    if installer_pid == 0 {
        return Err("installer_control_arguments_invalid".to_string());
    }
    Ok(InstallerControlCommandV1::Prepare {
        coordination_dir,
        bootstrap_dir,
        install_dir,
        main_executable,
        package_manifest,
        package_manifest_sha256,
        user_data_disposition,
        installer_pid,
    })
}

fn parse_takeover_installer_control(
    arguments: &[OsString],
) -> Result<InstallerControlCommandV1, String> {
    const KEYS: [&str; 7] = [
        "--coordination-dir",
        "--install-dir",
        "--main-executable",
        "--package-manifest",
        "--package-manifest-sha256",
        "--action",
        "--installer-pid",
    ];
    let values = exact_installer_values(arguments, &KEYS)?;
    let coordination_dir = installer_path(values[0])?;
    let install_dir = installer_path(values[1])?;
    let main_executable = installer_path(values[2])?;
    let package_manifest = installer_path(values[3])?;
    let package_manifest_sha256 = installer_sha256(values[4])?;
    let action = match values[5].to_str() {
        Some("continue-uninstall") => TakeoverActionV1::ContinueUninstall,
        Some("repair") => TakeoverActionV1::Repair,
        _ => return Err("installer_control_arguments_invalid".to_string()),
    };
    let installer_pid = installer_u32(values[6])?;
    validate_coordination_dir_path(&coordination_dir)?;
    validate_installer_absolute_path(&install_dir)?;
    validate_installer_absolute_path(&main_executable)?;
    validate_installer_absolute_path(&package_manifest)?;
    if installer_pid == 0 {
        return Err("installer_control_arguments_invalid".to_string());
    }
    Ok(InstallerControlCommandV1::Takeover {
        coordination_dir,
        install_dir,
        main_executable,
        package_manifest,
        package_manifest_sha256,
        action,
        installer_pid,
    })
}

fn parse_installer_authentication(
    arguments: &[OsString],
) -> Result<InstallerAuthenticationArgsV1, String> {
    const KEYS: [&str; 4] = [
        "--coordination-dir",
        "--session-nonce",
        "--installer-pid",
        "--installer-created-at-100ns",
    ];
    let values = exact_installer_values(arguments, &KEYS)?;
    let coordination_dir = installer_path(values[0])?;
    validate_coordination_dir_path(&coordination_dir)?;
    let authentication = GuardAuthenticationV1 {
        nonce: installer_string(values[1])?,
        installer_pid: installer_u32(values[2])?,
        installer_created_at_100ns: installer_u64(values[3])?,
    };
    validate_authentication(&authentication)?;
    Ok(InstallerAuthenticationArgsV1 {
        coordination_dir,
        authentication,
    })
}

fn parse_complete_repair_installer_control(
    arguments: &[OsString],
) -> Result<InstallerControlCommandV1, String> {
    if arguments.len() != 12 {
        return Err("installer_control_arguments_invalid".to_string());
    }
    let authentication = parse_installer_authentication(&arguments[..8])?;
    if arguments[8] != "--package-manifest" || arguments[10] != "--package-manifest-sha256" {
        return Err("installer_control_arguments_invalid".to_string());
    }
    let package_manifest = installer_path(&arguments[9])?;
    validate_installer_absolute_path(&package_manifest)?;
    let package_manifest_sha256 = installer_sha256(&arguments[11])?;
    Ok(InstallerControlCommandV1::CompleteRepair {
        authentication,
        package_manifest,
        package_manifest_sha256,
    })
}

fn exact_installer_values<'a>(
    arguments: &'a [OsString],
    keys: &[&str],
) -> Result<Vec<&'a OsString>, String> {
    if arguments.len() != keys.len() * 2 {
        return Err("installer_control_arguments_invalid".to_string());
    }
    let mut values = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        if arguments[index * 2] != *key {
            return Err("installer_control_arguments_invalid".to_string());
        }
        values.push(&arguments[index * 2 + 1]);
    }
    Ok(values)
}

fn installer_path(value: &OsString) -> Result<PathBuf, String> {
    value
        .to_str()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "installer_control_arguments_invalid".to_string())
}

fn installer_string(value: &OsString) -> Result<String, String> {
    value
        .to_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "installer_control_arguments_invalid".to_string())
}

fn installer_sha256(value: &OsString) -> Result<String, String> {
    let value = installer_string(value)?;
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(value)
    } else {
        Err("installer_control_arguments_invalid".to_string())
    }
}

fn installer_u32(value: &OsString) -> Result<u32, String> {
    parse_decimal_u32(value).map_err(|_| "installer_control_arguments_invalid".to_string())
}

fn installer_u64(value: &OsString) -> Result<u64, String> {
    parse_decimal_u64(value).map_err(|_| "installer_control_arguments_invalid".to_string())
}

fn installer_data_disposition(value: &OsString) -> Result<UserDataDispositionV1, String> {
    match value.to_str() {
        Some("retain") => Ok(UserDataDispositionV1::Retain),
        Some("delete") => Ok(UserDataDispositionV1::Delete),
        _ => Err("installer_control_arguments_invalid".to_string()),
    }
}

fn validate_installer_absolute_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.to_str().is_none()
        || is_drive_root(path)
        || has_ambiguous_path_components(path)
    {
        Err("installer_control_arguments_invalid".to_string())
    } else {
        Ok(())
    }
}

fn validate_session(coordination_dir: &Path, session: &UninstallSessionV1) -> Result<(), String> {
    validate_session_without_coordination(session)?;
    if same_path(&session.device_settings_dir, coordination_dir) {
        return Err("uninstall_device_settings_path_invalid".to_string());
    }
    validate_protected_paths(
        coordination_dir,
        &session.install_dir,
        &session.package_manifest,
        &session.protected_paths,
    )?;
    if is_uninitialized_session(session) {
        let expected =
            uninitialized_protected_paths(coordination_dir, &session.device_settings_dir)?;
        if normalize_paths(&session.protected_paths)? != normalize_paths(&expected)? {
            return Err("uninstall_session_invalid".to_string());
        }
    }
    Ok(())
}

fn validate_session_without_coordination(session: &UninstallSessionV1) -> Result<(), String> {
    if session.schema_version != CONTRACT_SCHEMA_VERSION
        || !valid_root_id(&session.root_id)
        || DateTime::parse_from_rfc3339(&session.prepared_at).is_err()
        || DateTime::parse_from_rfc3339(&session.updated_at).is_err()
    {
        return Err("uninstall_session_invalid".to_string());
    }
    if is_uninitialized_session(session)
        && session.old_device_autostart != DeviceAutostartSnapshotV1::default()
    {
        return Err("uninstall_session_invalid".to_string());
    }
    validate_authentication(&session.authentication)?;
    validate_device_settings_dir(&session.device_settings_dir, &session.install_dir)?;
    let executable = session.executable_identity();
    validate_executable_identity(&executable)?;
    validate_package_manifest(&session.package_manifest)?;
    if session.package_manifest.product_identity() != executable.product
        || canonical_package_manifest_bytes(&session.package_manifest)
            .map(|bytes| package_manifest_sha256(&bytes))?
            != session.package_manifest_sha256
    {
        return Err("uninstall_session_identity_mismatch".to_string());
    }
    Ok(())
}

fn validate_executable_identity(identity: &ExecutableIdentityV1) -> Result<(), String> {
    uninstall_manifest::validate_product_identity(&identity.product)?;
    if !identity.install_dir.is_absolute()
        || !identity.main_executable.is_absolute()
        || identity.install_dir.to_str().is_none()
        || identity.main_executable.to_str().is_none()
        || has_ambiguous_path_components(&identity.install_dir)
        || has_ambiguous_path_components(&identity.main_executable)
        || !path_is_within(&identity.main_executable, &identity.install_dir)
        || same_path(&identity.main_executable, &identity.install_dir)
    {
        return Err("uninstall_executable_identity_invalid".to_string());
    }
    Ok(())
}

fn validate_protected_paths(
    coordination_dir: &Path,
    install_dir: &Path,
    manifest: &PackageManifestV1,
    protected_paths: &[PathBuf],
) -> Result<(), String> {
    let normalized = normalize_paths(protected_paths)?;
    if !normalized.contains(&normalized_path(coordination_dir)) {
        return Err("uninstall_coordination_path_not_protected".to_string());
    }
    let install = normalized_path(install_dir);
    if normalized.contains(&install) {
        return Err("uninstall_install_root_protected".to_string());
    }
    for entry in &manifest.files {
        let target = normalized_path(&install_dir.join(relative_manifest_path(&entry.path)?));
        if normalized.iter().any(|value| paths_overlap(&target, value)) {
            return Err("uninstall_manifest_protected_overlap".to_string());
        }
    }
    Ok(())
}

fn normalize_paths(paths: &[PathBuf]) -> Result<BTreeSet<String>, String> {
    if paths.is_empty() {
        return Err("uninstall_protected_paths_invalid".to_string());
    }
    let mut normalized = BTreeSet::new();
    for path in paths {
        if !path.is_absolute()
            || path.to_str().is_none()
            || is_drive_root(path)
            || has_ambiguous_path_components(path)
        {
            return Err("uninstall_protected_paths_invalid".to_string());
        }
        if !normalized.insert(normalized_path(path)) {
            return Err("uninstall_protected_paths_invalid".to_string());
        }
    }
    Ok(normalized)
}

fn rollback_autostart(
    coordination_dir: &Path,
    session: &UninstallSessionV1,
    system: &dyn UninstallSystem,
) -> Result<(), String> {
    let run_result = write_and_verify_run(system, session.old_run_value.as_deref());
    let device_result =
        write_and_verify_session_device(system, session, &session.old_device_autostart);
    match (run_result, device_result) {
        (Ok(()), Ok(())) => Ok(()),
        (run, device) => {
            let reason = format!(
                "run={};device={}",
                run.err().unwrap_or_else(|| "ok".to_string()),
                device.err().unwrap_or_else(|| "ok".to_string())
            );
            write_cleanup_fault(coordination_dir, session, &reason)?;
            Err("autostart_cleanup_fault".to_string())
        }
    }
}

fn fail_preparing_with_rollback(
    coordination_dir: &Path,
    session: &UninstallSessionV1,
    system: &dyn UninstallSystem,
    original_error: String,
) -> Result<UninstallSessionV1, String> {
    match rollback_autostart(coordination_dir, session, system) {
        Ok(()) => {
            remove_file_if_exists(&session_path(coordination_dir))?;
            Err(original_error)
        }
        Err(error) => Err(error),
    }
}

fn write_and_verify_run(system: &dyn UninstallSystem, value: Option<&str>) -> Result<(), String> {
    system
        .write_run_value(value)
        .map_err(|_| "uninstall_run_write_failed".to_string())?;
    let observed = system
        .read_run_value()
        .map_err(|_| "uninstall_run_verify_failed".to_string())?;
    if observed.as_deref() != value {
        return Err("uninstall_run_verify_failed".to_string());
    }
    Ok(())
}

fn write_and_verify_device(
    system: &dyn UninstallSystem,
    device_settings_dir: &Path,
    value: &DeviceAutostartSnapshotV1,
) -> Result<(), String> {
    system
        .write_device_autostart(device_settings_dir, value)
        .map_err(|_| "uninstall_device_settings_write_failed".to_string())?;
    let observed = system
        .read_device_autostart(device_settings_dir)
        .map_err(|_| "uninstall_device_settings_verify_failed".to_string())?;
    if observed != *value {
        return Err("uninstall_device_settings_verify_failed".to_string());
    }
    Ok(())
}

fn write_and_verify_session_device(
    system: &dyn UninstallSystem,
    session: &UninstallSessionV1,
    value: &DeviceAutostartSnapshotV1,
) -> Result<(), String> {
    if is_uninitialized_session(session) {
        if value != &DeviceAutostartSnapshotV1::default() {
            return Err("uninstall_session_invalid".to_string());
        }
        return validate_uninitialized_data_absent(&session.device_settings_dir);
    }
    write_and_verify_device(system, &session.device_settings_dir, value)
}

fn is_uninitialized_session(session: &UninstallSessionV1) -> bool {
    session.root_id == UNINITIALIZED_ROOT_ID
}

fn validate_uninitialized_data_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        _ => Err("uninstall_uninitialized_data_changed".to_string()),
    }
}

fn verify_installer_process(
    authentication: &GuardAuthenticationV1,
    system: &dyn UninstallSystem,
) -> Result<(), String> {
    let observed = system
        .process_creation_time_100ns(authentication.installer_pid)
        .map_err(|_| "uninstall_installer_identity_query_failed".to_string())?;
    if observed != Some(authentication.installer_created_at_100ns) {
        return Err("uninstall_installer_identity_mismatch".to_string());
    }
    Ok(())
}

fn verify_old_executable_identity(
    identity: &ExecutableIdentityV1,
    system: &dyn UninstallSystem,
    required: bool,
) -> Result<(), String> {
    let observed = system.executable_product_identity(&identity.main_executable)?;
    match observed {
        Some(observed) if observed == identity.product => Ok(()),
        Some(_) => Err("uninstall_old_executable_identity_mismatch".to_string()),
        None if !required => Ok(()),
        None => Err("uninstall_old_executable_missing".to_string()),
    }
}

fn write_cleanup_fault(
    coordination_dir: &Path,
    session: &UninstallSessionV1,
    reason: &str,
) -> Result<(), String> {
    write_json_atomically(
        &cleanup_fault_path(coordination_dir),
        &AutostartCleanupFaultV1 {
            schema_version: CONTRACT_SCHEMA_VERSION,
            nonce: session.authentication.nonce.clone(),
            phase: session.phase,
            reason: sanitize_reason(reason),
            observed_at: now_iso(),
        },
    )
}

fn write_interrupted_fault(
    coordination_dir: &Path,
    session: &UninstallSessionV1,
    reason: &str,
) -> Result<(), String> {
    write_json_atomically(
        &interrupted_fault_path(coordination_dir),
        &UninstallInterruptedFaultV1 {
            schema_version: CONTRACT_SCHEMA_VERSION,
            nonce: session.authentication.nonce.clone(),
            reason: sanitize_reason(reason),
            observed_at: now_iso(),
        },
    )
}

fn read_valid_fault<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|_| "uninstall_fault_invalid".to_string())
}

fn read_guard_prepare_request(path: &Path) -> Result<GuardPrepareRequestV1, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "uninstall_guard_prepare_request_invalid".to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("uninstall_guard_prepare_request_invalid".to_string());
    }
    serde_json::from_slice(
        &fs::read(path).map_err(|_| "uninstall_guard_prepare_request_invalid".to_string())?,
    )
    .map_err(|_| "uninstall_guard_prepare_request_invalid".to_string())
}

fn read_installer_file(path: &Path, error_code: &str) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error_code.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err(error_code.to_string());
    }
    fs::read(path).map_err(|_| error_code.to_string())
}

fn validate_cleanup_fault(fault: &AutostartCleanupFaultV1) -> Result<(), String> {
    if fault.schema_version != CONTRACT_SCHEMA_VERSION
        || !valid_simple_id(&fault.nonce)
        || !matches!(
            fault.phase,
            UninstallPhaseV1::Preparing | UninstallPhaseV1::Prepared
        )
        || DateTime::parse_from_rfc3339(&fault.observed_at).is_err()
        || sanitize_reason(&fault.reason) != fault.reason
    {
        return Err("uninstall_fault_invalid".to_string());
    }
    Ok(())
}

fn validate_interrupted_fault(fault: &UninstallInterruptedFaultV1) -> Result<(), String> {
    if fault.schema_version != CONTRACT_SCHEMA_VERSION
        || !valid_simple_id(&fault.nonce)
        || DateTime::parse_from_rfc3339(&fault.observed_at).is_err()
        || sanitize_reason(&fault.reason) != fault.reason
    {
        return Err("uninstall_fault_invalid".to_string());
    }
    Ok(())
}

fn reject_existing_uninstall_evidence(coordination_dir: &Path) -> Result<(), String> {
    if [
        session_path(coordination_dir),
        cleanup_fault_path(coordination_dir),
        interrupted_fault_path(coordination_dir),
    ]
    .iter()
    .any(|path| path.exists())
    {
        return Err("uninstall_recovery_required".to_string());
    }
    Ok(())
}

fn read_session(coordination_dir: &Path) -> Result<Option<UninstallSessionV1>, String> {
    let path = session_path(coordination_dir);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let session = serde_json::from_slice::<UninstallSessionV1>(&bytes)
        .map_err(|_| "uninstall_session_invalid".to_string())?;
    validate_session(coordination_dir, &session)?;
    Ok(Some(session))
}

fn write_session(coordination_dir: &Path, session: &UninstallSessionV1) -> Result<(), String> {
    validate_session(coordination_dir, session)?;
    write_json_atomically(&session_path(coordination_dir), session)
}

fn cleanup_terminal_evidence(coordination_dir: &Path) -> Result<(), String> {
    remove_file_if_exists(&cleanup_fault_path(coordination_dir))?;
    remove_file_if_exists(&interrupted_fault_path(coordination_dir))?;
    remove_file_if_exists(&session_path(coordination_dir))
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn session_path(coordination_dir: &Path) -> PathBuf {
    coordination_dir.join(UNINSTALL_SESSION_FILE)
}

fn cleanup_fault_path(coordination_dir: &Path) -> PathBuf {
    coordination_dir.join(AUTOSTART_CLEANUP_FAULT_FILE)
}

fn interrupted_fault_path(coordination_dir: &Path) -> PathBuf {
    coordination_dir.join(UNINSTALL_INTERRUPTED_FAULT_FILE)
}

fn validate_coordination_dir(coordination_dir: &Path) -> Result<(), String> {
    validate_coordination_dir_path(coordination_dir)?;
    fs::create_dir_all(coordination_dir).map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(coordination_dir).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata_is_reparse(&metadata) {
        return Err("uninstall_coordination_dir_invalid".to_string());
    }
    Ok(())
}

fn validate_coordination_dir_path(coordination_dir: &Path) -> Result<(), String> {
    if !coordination_dir.is_absolute()
        || coordination_dir.to_str().is_none()
        || is_drive_root(coordination_dir)
    {
        return Err("uninstall_coordination_dir_invalid".to_string());
    }
    Ok(())
}

fn validate_device_settings_dir(
    device_settings_dir: &Path,
    install_dir: &Path,
) -> Result<(), String> {
    if !device_settings_dir.is_absolute()
        || device_settings_dir.to_str().is_none()
        || is_drive_root(device_settings_dir)
        || has_ambiguous_path_components(device_settings_dir)
        || same_path(device_settings_dir, install_dir)
    {
        return Err("uninstall_device_settings_path_invalid".to_string());
    }
    Ok(())
}

fn validate_authentication(authentication: &GuardAuthenticationV1) -> Result<(), String> {
    if !valid_simple_id(&authentication.nonce)
        || authentication.installer_pid == 0
        || authentication.installer_created_at_100ns == 0
    {
        return Err("uninstall_guard_authentication_invalid".to_string());
    }
    Ok(())
}

fn validate_root_id(value: &str) -> Result<(), String> {
    if valid_simple_id(value) {
        Ok(())
    } else {
        Err("uninstall_root_id_invalid".to_string())
    }
}

fn valid_root_id(value: &str) -> bool {
    valid_simple_id(value)
}

fn valid_simple_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn relative_manifest_path(value: &str) -> Result<PathBuf, String> {
    let probe = PackageManifestV1 {
        schema_version: 1,
        application_id: APPLICATION_ID.to_string(),
        product_name: PRODUCT_NAME.to_string(),
        build_id: COMPILED_BUILD_ID.to_string(),
        semantic_version: SEMANTIC_VERSION.to_string(),
        architecture: ARCHITECTURE.to_string(),
        files: vec![PackageManifestEntryV1 {
            path: value.to_string(),
            bytes: 0,
            sha256: "0".repeat(64),
        }],
    };
    validate_package_manifest(&probe)?;
    Ok(PathBuf::from(value))
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&(right.to_string() + "\\"))
        || right.starts_with(&(left.to_string() + "\\"))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn has_ambiguous_path_components(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    let path = normalized_path(path);
    let root = normalized_path(root);
    path == root || path.starts_with(&(root + "\\"))
}

fn same_path(left: &Path, right: &Path) -> bool {
    normalized_path(left) == normalized_path(right)
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn is_drive_root(path: &Path) -> bool {
    let value = normalized_path(path);
    value.len() == 2 && value.as_bytes()[1] == b':'
}

fn parse_decimal_u32(value: &OsStr) -> Result<u32, String> {
    let value = value
        .to_str()
        .ok_or_else(|| "uninstall_guard_arguments_invalid".to_string())?;
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err("uninstall_guard_arguments_invalid".to_string());
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "uninstall_guard_arguments_invalid".to_string())
}

fn parse_decimal_u64(value: &OsStr) -> Result<u64, String> {
    let value = value
        .to_str()
        .ok_or_else(|| "uninstall_guard_arguments_invalid".to_string())?;
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err("uninstall_guard_arguments_invalid".to_string());
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "uninstall_guard_arguments_invalid".to_string())
}

fn sanitize_reason(reason: &str) -> String {
    let value = reason
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect::<String>();
    if value.is_empty() {
        "uninstall_failure".to_string()
    } else {
        value
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Copy)]
enum NativeGuardEventV1 {
    Finished,
    Aborted,
}

const NATIVE_GUARD_START_TIMEOUT: Duration = Duration::from_secs(15);

fn guard_executable_path(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
) -> PathBuf {
    coordination_dir.join(format!("uninstall-guard-{}.exe", authentication.nonce))
}

fn controller_executable_path(coordination_dir: &Path) -> PathBuf {
    coordination_dir.join("uninstall-controller.exe")
}

fn guard_prepare_request_path(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
) -> PathBuf {
    coordination_dir.join(format!("uninstall-prepare-{}.json", authentication.nonce))
}

#[cfg(target_os = "windows")]
fn start_native_guard(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
    expected_guard_product: &ProductIdentityV1,
    prepare_request: Option<&GuardPrepareRequestV1>,
) -> Result<InstallerGuardProcessV1, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    validate_coordination_dir(coordination_dir)?;
    validate_authentication(authentication)?;
    uninstall_manifest::validate_product_identity(expected_guard_product)?;
    let source = std::env::current_exe()
        .map_err(|_| "installer_control_executable_unavailable".to_string())?;
    let observed = read_embedded_executable_product_identity(&source)?
        .ok_or_else(|| "installer_control_executable_identity_missing".to_string())?;
    if &observed != expected_guard_product {
        return Err("installer_control_executable_identity_mismatch".to_string());
    }

    let controller_path = controller_executable_path(coordination_dir);
    let guard_path = guard_executable_path(coordination_dir, authentication);
    let request_path = guard_prepare_request_path(coordination_dir, authentication);
    remove_file_if_exists(&guard_path)?;
    remove_file_if_exists(&request_path)?;
    if !same_path(&source, &controller_path) {
        remove_file_if_exists(&controller_path)?;
        fs::copy(&source, &controller_path)
            .map_err(|_| "uninstall_controller_copy_failed".to_string())?;
    }
    let controller = read_embedded_executable_product_identity(&controller_path)?
        .ok_or_else(|| "installer_control_executable_identity_missing".to_string())?;
    if controller != observed {
        let _ = remove_file_if_exists(&controller_path);
        return Err("installer_control_executable_identity_mismatch".to_string());
    }
    fs::copy(&controller_path, &guard_path)
        .map_err(|_| "uninstall_guard_copy_failed".to_string())?;
    let copied = read_embedded_executable_product_identity(&guard_path)?
        .ok_or_else(|| "installer_control_executable_identity_missing".to_string())?;
    if copied != observed {
        let _ = remove_file_if_exists(&guard_path);
        return Err("installer_control_executable_identity_mismatch".to_string());
    }
    if let Some(request) = prepare_request {
        if request.authentication != *authentication {
            let _ = remove_file_if_exists(&guard_path);
            return Err("uninstall_guard_authentication_failed".to_string());
        }
        write_json_atomically(&request_path, request)?;
    }

    let request_argument = prepare_request
        .map(|_| request_path.as_os_str())
        .unwrap_or_else(|| OsStr::new("-"));
    let mut child = Command::new(&guard_path)
        .args([
            OsStr::new("--uninstall-guard"),
            OsStr::new("--coordination-dir"),
            coordination_dir.as_os_str(),
            OsStr::new("--prepare-request"),
            request_argument,
            OsStr::new("--session-nonce"),
            OsStr::new(&authentication.nonce),
            OsStr::new("--installer-pid"),
            OsStr::new(&authentication.installer_pid.to_string()),
            OsStr::new("--installer-created-at-100ns"),
            OsStr::new(&authentication.installer_created_at_100ns.to_string()),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|_| "uninstall_guard_start_failed".to_string())?;
    let outcome = wait_for_native_guard_ready(authentication, &mut child);
    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = remove_file_if_exists(&request_path);
        let _ = remove_file_if_exists(&guard_path);
    }
    outcome.map(|_| InstallerGuardProcessV1 {
        control_executable: controller_path,
        guard_executable: guard_path,
    })
}

#[cfg(not(target_os = "windows"))]
fn start_native_guard(
    _coordination_dir: &Path,
    _authentication: &GuardAuthenticationV1,
    _expected_guard_product: &ProductIdentityV1,
    _prepare_request: Option<&GuardPrepareRequestV1>,
) -> Result<InstallerGuardProcessV1, String> {
    Err("uninstall_guard_windows_only".to_string())
}

#[cfg(target_os = "windows")]
fn wait_for_native_guard_ready(
    authentication: &GuardAuthenticationV1,
    child: &mut Child,
) -> Result<(), String> {
    let deadline = Instant::now() + NATIVE_GUARD_START_TIMEOUT;
    loop {
        match assert_native_guard_alive(authentication) {
            Ok(()) => return Ok(()),
            Err(error) if error == "uninstall_guard_not_ready" => {}
            Err(error) => return Err(error),
        }
        if child
            .try_wait()
            .map_err(|_| "uninstall_guard_start_failed".to_string())?
            .is_some()
        {
            return Err("uninstall_guard_exited_before_ready".to_string());
        }
        if Instant::now() >= deadline {
            return Err("uninstall_guard_start_timeout".to_string());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn cleanup_native_guard_files(
    coordination_dir: &Path,
    authentication: &GuardAuthenticationV1,
) -> Result<(), String> {
    let guard_path = guard_executable_path(coordination_dir, authentication);
    let request_path = guard_prepare_request_path(coordination_dir, authentication);
    remove_file_if_exists(&request_path)?;
    remove_file_if_exists(&coordination_dir.join(INSTALLER_SESSION_FILE))?;
    remove_file_if_exists(&coordination_dir.join(INSTALLER_SESSION_ARGS_FILE))?;
    for _ in 0..100 {
        match fs::remove_file(&guard_path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
    Err("uninstall_guard_cleanup_failed".to_string())
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::{fs::OpenOptions, io::Write};
    let parent = path
        .parent()
        .ok_or_else(|| "installer_control_output_invalid".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "installer_control_output_invalid".to_string())?;
    let temporary = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        replace_installer_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(target_os = "windows")]
fn replace_installer_file(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_installer_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target).map_err(|error| error.to_string())?;
    }
    fs::rename(source, target).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn assert_native_guard_alive(authentication: &GuardAuthenticationV1) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenEventW, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE},
    };
    let names = uninstall_guard_event_names(&authentication.nonce)?;
    let name = OsStr::new(&names.ready)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let event = unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if event.is_null() {
        return Err("uninstall_guard_not_ready".to_string());
    }
    let result = unsafe { WaitForSingleObject(event, 0) };
    unsafe { CloseHandle(event) };
    if result == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err("uninstall_guard_not_ready".to_string())
    }
}

#[cfg(not(target_os = "windows"))]
fn assert_native_guard_alive(_authentication: &GuardAuthenticationV1) -> Result<(), String> {
    Err("uninstall_guard_windows_only".to_string())
}

#[cfg(target_os = "windows")]
fn signal_native_guard_event(
    authentication: &GuardAuthenticationV1,
    event: NativeGuardEventV1,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE},
    };
    let names = uninstall_guard_event_names(&authentication.nonce)?;
    let name = match event {
        NativeGuardEventV1::Finished => names.finished,
        NativeGuardEventV1::Aborted => names.aborted,
    };
    let name = OsStr::new(&name)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if handle.is_null() {
        return Err("uninstall_guard_not_ready".to_string());
    }
    let signalled = unsafe { SetEvent(handle) } != 0;
    unsafe { CloseHandle(handle) };
    signalled
        .then_some(())
        .ok_or_else(|| "uninstall_guard_signal_failed".to_string())
}

#[cfg(not(target_os = "windows"))]
fn signal_native_guard_event(
    _authentication: &GuardAuthenticationV1,
    _event: NativeGuardEventV1,
) -> Result<(), String> {
    Err("uninstall_guard_windows_only".to_string())
}

#[cfg(target_os = "windows")]
fn process_creation_time_100ns(pid: u32) -> Result<Option<u64>, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME},
        System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            Ok(None)
        } else {
            Err(error.to_string())
        };
    }
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let result =
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    unsafe { CloseHandle(process) };
    if result == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(Some(
        ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
    ))
}

#[cfg(not(target_os = "windows"))]
fn process_creation_time_100ns(_pid: u32) -> Result<Option<u64>, String> {
    Ok(None)
}

#[cfg(target_os = "windows")]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NONCE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn native_guard_rechecks_process_creation_and_rejects_event_reuse() {
        let pid = std::process::id();
        let created_at = process_creation_time_100ns(pid).unwrap().unwrap();
        let authentication = GuardAuthenticationV1 {
            nonce: test_nonce(),
            installer_pid: pid,
            installer_created_at_100ns: created_at,
        };
        let first = NativeUninstallGuardHandshake::new(&authentication).unwrap();
        assert_eq!(
            NativeUninstallGuardHandshake::new(&authentication).unwrap_err(),
            "uninstall_guard_event_already_exists"
        );
        drop(first);

        let reused = GuardAuthenticationV1 {
            nonce: test_nonce(),
            installer_created_at_100ns: created_at + 1,
            ..authentication
        };
        assert_eq!(
            NativeUninstallGuardHandshake::new(&reused).unwrap_err(),
            "uninstall_installer_identity_mismatch"
        );
    }

    fn test_nonce() -> String {
        format!(
            "{:032x}",
            ((std::process::id() as u128) << 64)
                | NONCE_SEQUENCE.fetch_add(1, Ordering::Relaxed) as u128
        )
    }
}
