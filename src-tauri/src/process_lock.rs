use sha2::{Digest, Sha256};
use std::path::Path;

const GUI_MUTEX_NAME: &str = "Local\\LifeArchiveProject.Emily.Gui.v1";
const GUI_ACTIVATION_EVENT_NAME: &str = "Local\\LifeArchiveProject.Emily.Gui.Activate.v1";
const UNINSTALL_MUTEX_NAME: &str = "Local\\LifeArchiveProject.Emily.Uninstall.v1";

#[derive(Debug)]
pub struct GuiActivationEvent {
    inner: NamedAutoResetEvent,
}

impl GuiActivationEvent {
    pub fn create_application() -> Result<Self, String> {
        Self::create_named(GUI_ACTIVATION_EVENT_NAME)
    }

    pub fn open_application() -> Result<Self, String> {
        Self::open_named(GUI_ACTIVATION_EVENT_NAME)
    }

    pub fn create_named(name: &str) -> Result<Self, String> {
        NamedAutoResetEvent::create(name).map(|inner| Self { inner })
    }

    pub fn open_named(name: &str) -> Result<Self, String> {
        NamedAutoResetEvent::open(name).map(|inner| Self { inner })
    }

    pub fn signal(&self) -> Result<(), String> {
        self.inner.signal()
    }

    pub fn wait_timeout(&self, timeout: std::time::Duration) -> Result<bool, String> {
        self.inner.wait_timeout(timeout)
    }
}

#[derive(Debug)]
pub struct GuiInstanceGuard {
    _inner: NamedMutexGuard,
}

impl GuiInstanceGuard {
    pub fn acquire_application() -> Result<Self, String> {
        Self::acquire_named(GUI_MUTEX_NAME)
    }

    pub fn acquire_named(name: &str) -> Result<Self, String> {
        NamedMutexGuard::acquire(name, "gui_instance_already_running")
            .map(|inner| Self { _inner: inner })
    }
}

#[derive(Debug)]
pub struct UninstallProcessGuard {
    _gui: NamedMutexGuard,
    _uninstall: NamedMutexGuard,
}

impl UninstallProcessGuard {
    pub fn acquire() -> Result<Self, String> {
        let gui = NamedMutexGuard::acquire(GUI_MUTEX_NAME, "application_still_running")?;
        let uninstall =
            NamedMutexGuard::acquire(UNINSTALL_MUTEX_NAME, "uninstall_guard_already_running")?;
        Ok(Self {
            _gui: gui,
            _uninstall: uninstall,
        })
    }
}

#[derive(Debug)]
pub struct BootstrapProcessGuard {
    _inner: NamedMutexGuard,
}

impl BootstrapProcessGuard {
    pub fn acquire(bootstrap_dir: &Path) -> Result<Self, String> {
        let normalized = bootstrap_dir
            .to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase();
        if normalized.is_empty() {
            return Err("invalid_bootstrap_path".to_string());
        }
        let digest = format!("{:x}", Sha256::digest(normalized.as_bytes()));
        let name = format!(
            "Local\\LifeArchiveProject.Emily.Bootstrap.{}",
            &digest[..32]
        );
        NamedMutexGuard::acquire(&name, "bootstrap_locked").map(|inner| Self { _inner: inner })
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct NamedMutexGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
impl NamedMutexGuard {
    fn acquire(name: &str, occupied_error: &str) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        };

        if name.is_empty() || name.encode_utf16().any(|value| value == 0) {
            return Err("invalid_mutex_name".to_string());
        }
        let name: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Err(occupied_error.to_string());
        }
        Ok(Self { handle })
    }
}

#[cfg(target_os = "windows")]
impl Drop for NamedMutexGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::{Foundation::CloseHandle, System::Threading::ReleaseMutex};
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct NamedAutoResetEvent {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// Win32 kernel event handles may be waited on and closed from another thread.
#[cfg(target_os = "windows")]
unsafe impl Send for NamedAutoResetEvent {}

#[cfg(target_os = "windows")]
impl NamedAutoResetEvent {
    fn create(name: &str) -> Result<Self, String> {
        use windows_sys::Win32::System::Threading::CreateEventW;

        let name = wide_name(name)?;
        let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(Self { handle })
    }

    fn open(name: &str) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenEventW, EVENT_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE},
        };

        let name = wide_name(name)?;
        let handle = unsafe {
            OpenEventW(
                EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
                0,
                name.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(handle) };
            return Err("gui_activation_event_open_failed".to_string());
        }
        Ok(Self { handle })
    }

    fn signal(&self) -> Result<(), String> {
        use windows_sys::Win32::System::Threading::SetEvent;
        if unsafe { SetEvent(self.handle) } == 0 {
            Err(std::io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }

    fn wait_timeout(&self, timeout: std::time::Duration) -> Result<bool, String> {
        use windows_sys::Win32::{
            Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::WaitForSingleObject,
        };
        let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        match unsafe { WaitForSingleObject(self.handle, timeout_ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(std::io::Error::last_os_error().to_string()),
            _ => Err("gui_activation_event_wait_failed".to_string()),
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for NamedAutoResetEvent {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
fn wide_name(name: &str) -> Result<Vec<u16>, String> {
    use std::os::windows::ffi::OsStrExt;
    if name.is_empty() || name.encode_utf16().any(|value| value == 0) {
        return Err("invalid_event_name".to_string());
    }
    Ok(std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(Some(0))
        .collect())
}

#[cfg(not(target_os = "windows"))]
#[derive(Debug)]
struct NamedMutexGuard;

#[cfg(not(target_os = "windows"))]
impl NamedMutexGuard {
    fn acquire(_name: &str, _occupied_error: &str) -> Result<Self, String> {
        Ok(Self)
    }
}

#[cfg(not(target_os = "windows"))]
#[derive(Debug, Default)]
struct NamedAutoResetEvent {
    signalled: std::sync::Mutex<bool>,
}

#[cfg(not(target_os = "windows"))]
impl NamedAutoResetEvent {
    fn create(_name: &str) -> Result<Self, String> {
        Ok(Self::default())
    }

    fn open(_name: &str) -> Result<Self, String> {
        Ok(Self::default())
    }

    fn signal(&self) -> Result<(), String> {
        *self
            .signalled
            .lock()
            .map_err(|_| "gui_activation_event_poisoned".to_string())? = true;
        Ok(())
    }

    fn wait_timeout(&self, _timeout: std::time::Duration) -> Result<bool, String> {
        let mut signalled = self
            .signalled
            .lock()
            .map_err(|_| "gui_activation_event_poisoned".to_string())?;
        let observed = *signalled;
        *signalled = false;
        Ok(observed)
    }
}
