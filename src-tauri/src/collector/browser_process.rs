use super::browser_state::{canonical_windows_path, BrowserId};
use serde::{Deserialize, Serialize};
use std::{
    net::{SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

#[cfg(target_os = "windows")]
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr::{null, null_mut}, sync::atomic::{AtomicU64, Ordering}};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE},
    System::{JobObjects::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE}, StationsAndDesktops::{CloseDesktop, CreateDesktopW, HDESK, DESKTOP_CREATEWINDOW, DESKTOP_ENUMERATE, DESKTOP_READOBJECTS, DESKTOP_WRITEOBJECTS}, Threading::{CreateProcessW, ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW}},
    UI::WindowsAndMessaging::SW_HIDE,
};

#[cfg(target_os = "windows")]
static DESKTOP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserProcessRecordV2 {
    pub schema_version: i64,
    pub pid: u32,
    pub creation_time_100ns: String,
    pub browser_id: BrowserId,
    pub executable_path: String,
    pub profile_path: String,
    pub user_data_dir_argument: String,
    pub devtools_port: u16,
    pub browser_websocket_path: String,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProcessIdentity {
    pub pid: u32,
    pub creation_time_100ns: String,
    pub executable_path: String,
    pub command_line: Vec<String>,
}

pub fn local_debug_port_is_reachable(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok()
}

pub fn browser_record_matches_expected_profile(
    record: &BrowserProcessRecordV2,
    expected_profile: &Path,
) -> bool {
    if record.schema_version != 2
        || record.pid == 0
        || record.devtools_port == 0
        || !record.browser_websocket_path.starts_with("/devtools/browser/")
    {
        return false;
    }
    let expected_name = match record.browser_id {
        BrowserId::Edge => "msedge.exe",
        BrowserId::Chrome => "chrome.exe",
    };
    if Path::new(&record.executable_path)
        .file_name()
        .and_then(|value| value.to_str())
        .is_none_or(|value| !value.eq_ignore_ascii_case(expected_name))
    {
        return false;
    }
    let expected_profile = expected_profile.to_string_lossy();
    if canonical_windows_path(&record.profile_path) != canonical_windows_path(&expected_profile) {
        return false;
    }
    let expected_argument = format!("--user-data-dir={expected_profile}");
    if canonical_user_data_argument(&record.user_data_dir_argument)
        != canonical_user_data_argument(&expected_argument)
    {
        return false;
    }
    true
}

pub fn browser_identity_matches(
    record: &BrowserProcessRecordV2,
    identity: &BrowserProcessIdentity,
    expected_profile: &Path,
) -> bool {
    if !browser_record_matches_expected_profile(record, expected_profile)
        || record.pid != identity.pid
        || record.creation_time_100ns != identity.creation_time_100ns
        || canonical_windows_path(&record.executable_path)
            != canonical_windows_path(&identity.executable_path)
    {
        return false;
    }
    let expected_argument = format!("--user-data-dir={}", expected_profile.to_string_lossy());
    identity.command_line.iter().any(|argument| {
        canonical_user_data_argument(argument) == canonical_user_data_argument(&expected_argument)
    })
}

fn canonical_user_data_argument(value: &str) -> Option<String> {
    let path = value.strip_prefix("--user-data-dir=")?.trim_matches('"');
    Some(format!("--user-data-dir={}", canonical_windows_path(path)))
}

#[cfg(target_os = "windows")]
pub struct ManagedBrowser {
    process_handle: HANDLE,
    job_handle: HANDLE,
    desktop_handle: Option<HDESK>,
    pub pid: u32,
    pub creation_time_100ns: String,
    pub desktop_name: Option<String>,
}

#[cfg(target_os = "windows")]
impl ManagedBrowser {
    pub fn launch(executable: &Path, args: &[String], hidden: bool) -> Result<Self, String> {
        let executable_text = executable.to_string_lossy().to_string();
        let mut command_line = quote_windows_arg(&executable_text);
        for argument in args {
            command_line.push(' ');
            command_line.push_str(&quote_windows_arg(argument));
        }
        let mut application = wide(executable.as_os_str());
        let mut command = wide(OsStr::new(&command_line));
        let (desktop_handle, mut desktop_name_wide, desktop_name) = if hidden {
            let (handle, name_wide, name) = create_private_desktop()?;
            (Some(handle), name_wide, Some(name))
        } else {
            (None, Vec::new(), None)
        };
        let desktop_pointer = if desktop_name_wide.is_empty() { null_mut() } else { desktop_name_wide.as_mut_ptr() };
        let startup = startup_info(hidden, desktop_pointer);
        let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | if hidden { CREATE_NO_WINDOW } else { 0 };
        let created = unsafe {
            CreateProcessW(
                application.as_mut_ptr(), command.as_mut_ptr(), null_mut(), null_mut(), 0,
                flags, null_mut(), null_mut(), &startup, &mut process_info,
            )
        };
        if created == 0 {
            close_private_desktop(desktop_handle);
            return Err(std::io::Error::last_os_error().to_string());
        }
        let job = unsafe { CreateJobObjectW(null_mut(), null_mut()) };
        if job.is_null() {
            unsafe { TerminateProcess(process_info.hProcess, 1); CloseHandle(process_info.hThread); CloseHandle(process_info.hProcess); }
            close_private_desktop(desktop_handle);
            return Err("browser_job_create_failed".to_string());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &mut limits as *mut _ as *mut _, std::mem::size_of_val(&limits) as u32)
        };
        let assigned = configured != 0 && unsafe { AssignProcessToJobObject(job, process_info.hProcess) } != 0;
        if !assigned || unsafe { ResumeThread(process_info.hThread) } == u32::MAX {
            unsafe { TerminateProcess(process_info.hProcess, 1); CloseHandle(process_info.hThread); CloseHandle(process_info.hProcess); CloseHandle(job); }
            close_private_desktop(desktop_handle);
            return Err("browser_job_assign_failed".to_string());
        }
        let creation_time = match process_creation_time(process_info.hProcess) {
            Ok(value) => value,
            Err(error) => {
                unsafe { TerminateProcess(process_info.hProcess, 1); CloseHandle(process_info.hThread); CloseHandle(process_info.hProcess); CloseHandle(job); }
                close_private_desktop(desktop_handle);
                return Err(error);
            }
        };
        unsafe { CloseHandle(process_info.hThread); }
        Ok(Self { process_handle: process_info.hProcess, job_handle: job, desktop_handle, pid: process_info.dwProcessId, creation_time_100ns: creation_time, desktop_name })
    }

    pub fn wait(&self, timeout_ms: u32) -> bool {
        unsafe { WaitForSingleObject(self.process_handle, timeout_ms) == 0 }
    }

    pub fn terminate(&self) -> Result<(), String> {
        if unsafe { TerminateJobObject(self.job_handle, 1) } == 0 {
            return Err("browser_job_terminate_failed".to_string());
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn create_private_desktop() -> Result<(HDESK, Vec<u16>, String), String> {
    let sequence = DESKTOP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!("EmilyCollector-{}-{sequence}", std::process::id());
    let name_wide = wide(OsStr::new(&name));
    let access = DESKTOP_CREATEWINDOW | DESKTOP_ENUMERATE | DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS;
    let handle = unsafe { CreateDesktopW(name_wide.as_ptr(), null(), null(), 0, access, null()) };
    if handle.is_null() {
        return Err("browser_desktop_create_failed".to_string());
    }
    Ok((handle, name_wide, name))
}

#[cfg(target_os = "windows")]
fn close_private_desktop(desktop: Option<HDESK>) {
    if let Some(desktop) = desktop {
        unsafe { CloseDesktop(desktop); }
    }
}

#[cfg(target_os = "windows")]
fn startup_info(hidden: bool, desktop: *mut u16) -> STARTUPINFOW {
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    if hidden {
        startup.dwFlags |= STARTF_USESHOWWINDOW;
        startup.wShowWindow = SW_HIDE as u16;
        startup.lpDesktop = desktop;
    }
    startup
}

#[cfg(target_os = "windows")]
impl Drop for ManagedBrowser {
    fn drop(&mut self) {
        unsafe {
            WaitForSingleObject(self.process_handle, 2_000);
            CloseHandle(self.process_handle);
            CloseHandle(self.job_handle);
        }
        close_private_desktop(self.desktop_handle);
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn process_creation_time(process: HANDLE) -> Result<String, String> {
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let ok = unsafe { windows_sys::Win32::System::Threading::GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
    if ok == 0 { return Err("browser_creation_time_failed".to_string()); }
    let value = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
    Ok(value.to_string())
}

#[cfg(target_os = "windows")]
fn wide(value: &OsStr) -> Vec<u16> { value.encode_wide().chain(Some(0)).collect() }

#[cfg(target_os = "windows")]
pub(crate) fn quote_windows_arg(value: &str) -> String {
    if !value.is_empty() && !value.chars().any(|character| " \t\"".contains(character)) { return value.to_string(); }
    let mut output = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        if character == '\\' { slashes += 1; continue; }
        if character == '"' { output.push_str(&"\\".repeat(slashes * 2 + 1)); output.push(character); slashes = 0; continue; }
        output.push_str(&"\\".repeat(slashes)); output.push(character); slashes = 0;
    }
    output.push_str(&"\\".repeat(slashes * 2)); output.push('"'); output
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::startup_info;
    use windows_sys::Win32::{
        System::Threading::STARTF_USESHOWWINDOW,
        UI::WindowsAndMessaging::SW_HIDE,
    };

    #[test]
    fn hidden_browser_uses_windows_hidden_startup_state() {
        let mut desktop = vec![1_u16, 0];
        let hidden = startup_info(true, desktop.as_mut_ptr());
        assert_ne!(hidden.dwFlags & STARTF_USESHOWWINDOW, 0);
        assert_eq!(hidden.wShowWindow, SW_HIDE as u16);
        assert_eq!(hidden.lpDesktop, desktop.as_mut_ptr());

        let visible = startup_info(false, std::ptr::null_mut());
        assert_eq!(visible.dwFlags & STARTF_USESHOWWINDOW, 0);
        assert!(visible.lpDesktop.is_null());
    }
}
