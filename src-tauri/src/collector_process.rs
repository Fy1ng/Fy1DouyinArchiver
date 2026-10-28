use crate::{
    collector::browser_state::canonical_windows_path,
    sync_orchestration::{is_simple_uuid, SyncPhase},
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(target_os = "windows")]
use crate::{
    collector::{
        browser_process::{process_creation_time, quote_windows_arg},
        file_store::write_json_atomically,
    },
    CollectorInvocation,
};

#[cfg(target_os = "windows")]
use std::{
    ffi::OsStr,
    os::windows::{ffi::OsStrExt, process::ExitStatusExt},
    path::Path,
    process::ExitStatus,
    ptr::null_mut,
};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess,
            WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
            INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
        },
    },
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectorProcessIdentityV1 {
    pub schema_version: i64,
    pub pid: u32,
    pub creation_time_100ns: String,
    pub executable_path: String,
    pub arguments_sha256: String,
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: SyncPhase,
    pub phase_attempt_id: String,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorProcessExpectation {
    pub account_id: i64,
    pub sync_job_id: i64,
    pub phase: SyncPhase,
    pub phase_attempt_id: String,
    pub executable_path: String,
    pub arguments_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectorProcessObservation {
    pub pid: u32,
    pub creation_time_100ns: String,
    pub executable_path: String,
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginCollectorProcessIdentityV1 {
    pub schema_version: i64,
    pub pid: u32,
    pub creation_time_100ns: String,
    pub executable_path: String,
    pub arguments_sha256: String,
    pub login_attempt_id: String,
    pub profile_key: String,
    pub target_account_id: Option<i64>,
    pub started_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCollectorProcessExpectation {
    pub login_attempt_id: String,
    pub profile_key: String,
    pub target_account_id: Option<i64>,
    pub executable_path: String,
    pub arguments_sha256: String,
}

impl CollectorProcessExpectation {
    #[cfg(target_os = "windows")]
    pub fn for_invocation(
        account_id: i64,
        sync_job_id: i64,
        phase: SyncPhase,
        phase_attempt_id: &str,
        invocation: &CollectorInvocation,
    ) -> Result<Self, String> {
        let expectation = Self {
            account_id,
            sync_job_id,
            phase,
            phase_attempt_id: phase_attempt_id.to_string(),
            executable_path: canonical_windows_path(&invocation.program.to_string_lossy()),
            arguments_sha256: collector_arguments_sha256(&invocation.args)?,
        };
        if !invocation.program.is_absolute() || !valid_expectation(&expectation) {
            return Err("invalid_collector_process_expectation".to_string());
        }
        Ok(expectation)
    }
}

impl LoginCollectorProcessExpectation {
    #[cfg(target_os = "windows")]
    pub fn for_invocation(
        login_attempt_id: &str,
        profile_key: &str,
        target_account_id: Option<i64>,
        invocation: &CollectorInvocation,
    ) -> Result<Self, String> {
        let expectation = Self {
            login_attempt_id: login_attempt_id.to_string(),
            profile_key: profile_key.to_string(),
            target_account_id,
            executable_path: canonical_windows_path(&invocation.program.to_string_lossy()),
            arguments_sha256: collector_arguments_sha256(&invocation.args)?,
        };
        if !invocation.program.is_absolute() || !valid_login_expectation(&expectation) {
            return Err("invalid_login_collector_process_expectation".to_string());
        }
        Ok(expectation)
    }
}

#[cfg(target_os = "windows")]
pub struct OwnedCollectorProcess {
    process_handle: HANDLE,
    primary_thread: Option<HANDLE>,
    job_handle: HANDLE,
    pid: u32,
}

// Windows kernel handles are process-wide; callers serialize access through a Mutex.
#[cfg(target_os = "windows")]
unsafe impl Send for OwnedCollectorProcess {}

#[cfg(target_os = "windows")]
impl OwnedCollectorProcess {
    pub fn launch_suspended(
        invocation: &CollectorInvocation,
        identity_path: &Path,
        expectation: &CollectorProcessExpectation,
    ) -> Result<Self, String> {
        let actual = CollectorProcessExpectation::for_invocation(
            expectation.account_id,
            expectation.sync_job_id,
            expectation.phase,
            &expectation.phase_attempt_id,
            invocation,
        )?;
        if actual != *expectation {
            return Err("collector_process_expectation_mismatch".to_string());
        }
        Self::launch_with_identity(invocation, identity_path, |pid, creation_time_100ns| {
            CollectorProcessIdentityV1 {
                schema_version: 1,
                pid,
                creation_time_100ns,
                executable_path: expectation.executable_path.clone(),
                arguments_sha256: expectation.arguments_sha256.clone(),
                account_id: expectation.account_id,
                sync_job_id: expectation.sync_job_id,
                phase: expectation.phase,
                phase_attempt_id: expectation.phase_attempt_id.clone(),
                started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            }
        })
    }

    pub fn launch_login_suspended(
        invocation: &CollectorInvocation,
        identity_path: &Path,
        expectation: &LoginCollectorProcessExpectation,
    ) -> Result<Self, String> {
        let actual = LoginCollectorProcessExpectation::for_invocation(
            &expectation.login_attempt_id,
            &expectation.profile_key,
            expectation.target_account_id,
            invocation,
        )?;
        if actual != *expectation {
            return Err("login_collector_process_expectation_mismatch".to_string());
        }
        Self::launch_with_identity(invocation, identity_path, |pid, creation_time_100ns| {
            LoginCollectorProcessIdentityV1 {
                schema_version: 1,
                pid,
                creation_time_100ns,
                executable_path: expectation.executable_path.clone(),
                arguments_sha256: expectation.arguments_sha256.clone(),
                login_attempt_id: expectation.login_attempt_id.clone(),
                profile_key: expectation.profile_key.clone(),
                target_account_id: expectation.target_account_id,
                started_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            }
        })
    }

    fn launch_with_identity<T, F>(
        invocation: &CollectorInvocation,
        identity_path: &Path,
        build_identity: F,
    ) -> Result<Self, String>
    where
        T: Serialize,
        F: FnOnce(u32, String) -> T,
    {
        if let Some(parent) = identity_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let executable_text = invocation.program.to_string_lossy().to_string();
        let mut command_line = quote_windows_arg(&executable_text);
        for argument in &invocation.args {
            command_line.push(' ');
            command_line.push_str(&quote_windows_arg(argument));
        }
        let mut application = wide(invocation.program.as_os_str());
        let mut command = wide(OsStr::new(&command_line));
        let current_directory = invocation
            .current_dir
            .as_ref()
            .map(|value| wide(value.as_os_str()));
        let current_directory_pointer = current_directory
            .as_ref()
            .map_or(null_mut(), |value| value.as_ptr() as *mut u16);
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let created = unsafe {
            CreateProcessW(
                application.as_mut_ptr(),
                command.as_mut_ptr(),
                null_mut(),
                null_mut(),
                0,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
                null_mut(),
                current_directory_pointer,
                &startup,
                &mut process_info,
            )
        };
        if created == 0 {
            return Err(format!(
                "collector_spawn_failed:{}",
                std::io::Error::last_os_error()
            ));
        }

        let job_handle = unsafe { CreateJobObjectW(null_mut(), null_mut()) };
        if job_handle.is_null() {
            close_failed_launch(&process_info, None);
            return Err("collector_job_create_failed".to_string());
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job_handle,
                JobObjectExtendedLimitInformation,
                &mut limits as *mut _ as *mut _,
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if configured == 0 {
            close_failed_launch(&process_info, Some(job_handle));
            return Err("collector_job_configure_failed".to_string());
        }
        if unsafe { AssignProcessToJobObject(job_handle, process_info.hProcess) } == 0 {
            close_failed_launch(&process_info, Some(job_handle));
            return Err("collector_job_assign_failed".to_string());
        }
        let creation_time_100ns = match process_creation_time(process_info.hProcess) {
            Ok(value) => value,
            Err(error) => {
                close_failed_launch(&process_info, Some(job_handle));
                return Err(error);
            }
        };
        let identity = build_identity(process_info.dwProcessId, creation_time_100ns);
        if let Err(error) = write_json_atomically(identity_path, &identity) {
            close_failed_launch(&process_info, Some(job_handle));
            return Err(error);
        }

        Ok(Self {
            process_handle: process_info.hProcess,
            primary_thread: Some(process_info.hThread),
            job_handle,
            pid: process_info.dwProcessId,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn resume(&mut self) -> Result<(), String> {
        let thread = self
            .primary_thread
            .take()
            .ok_or_else(|| "collector_process_already_resumed".to_string())?;
        let result = unsafe { ResumeThread(thread) };
        unsafe { CloseHandle(thread) };
        if result == u32::MAX {
            self.kill()?;
            return Err("collector_process_resume_failed".to_string());
        }
        Ok(())
    }

    pub fn try_wait(&self) -> Result<Option<ExitStatus>, String> {
        if unsafe { WaitForSingleObject(self.process_handle, 0) } == WAIT_TIMEOUT {
            return Ok(None);
        }
        self.exit_status().map(Some)
    }

    pub fn wait(&self) -> Result<ExitStatus, String> {
        if self.primary_thread.is_some() {
            return Err("collector_process_not_resumed".to_string());
        }
        unsafe { WaitForSingleObject(self.process_handle, INFINITE) };
        self.exit_status()
    }

    pub fn kill(&self) -> Result<(), String> {
        if unsafe { TerminateJobObject(self.job_handle, 1) } == 0 {
            return Err("collector_process_terminate_failed".to_string());
        }
        Ok(())
    }

    fn exit_status(&self) -> Result<ExitStatus, String> {
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.process_handle, &mut code) } == 0 {
            return Err("collector_exit_status_failed".to_string());
        }
        Ok(ExitStatus::from_raw(code))
    }
}

#[cfg(target_os = "windows")]
impl Drop for OwnedCollectorProcess {
    fn drop(&mut self) {
        if let Some(thread) = self.primary_thread.take() {
            unsafe {
                TerminateJobObject(self.job_handle, 1);
                CloseHandle(thread);
            }
        }
        unsafe {
            CloseHandle(self.job_handle);
            CloseHandle(self.process_handle);
        }
    }
}

#[cfg(target_os = "windows")]
fn close_failed_launch(process: &PROCESS_INFORMATION, job: Option<HANDLE>) {
    unsafe {
        if let Some(job) = job {
            TerminateJobObject(job, 1);
            CloseHandle(job);
        } else {
            TerminateProcess(process.hProcess, 1);
        }
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
}

#[cfg(target_os = "windows")]
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

pub fn collector_arguments_sha256(arguments: &[String]) -> Result<String, String> {
    let bytes = serde_json::to_vec(arguments).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn collector_process_identity_matches(
    record: &CollectorProcessIdentityV1,
    observation: &CollectorProcessObservation,
    expectation: &CollectorProcessExpectation,
) -> bool {
    collector_record_matches_expectation(record, expectation)
        && record.pid == observation.pid
        && record.creation_time_100ns == observation.creation_time_100ns
        && canonical_windows_path(&record.executable_path)
            == canonical_windows_path(&observation.executable_path)
        && collector_arguments_sha256(&observation.arguments)
            .is_ok_and(|hash| hash == record.arguments_sha256)
}

pub fn terminate_collector_if_matches<F>(
    record: &CollectorProcessIdentityV1,
    observation: &CollectorProcessObservation,
    expectation: &CollectorProcessExpectation,
    terminate: F,
) -> Result<bool, String>
where
    F: FnOnce(u32) -> Result<(), String>,
{
    if !collector_process_identity_matches(record, observation, expectation) {
        return Ok(false);
    }
    terminate(record.pid)?;
    Ok(true)
}

pub fn login_collector_process_identity_matches(
    record: &LoginCollectorProcessIdentityV1,
    observation: &CollectorProcessObservation,
    expectation: &LoginCollectorProcessExpectation,
) -> bool {
    login_collector_record_matches_expectation(record, expectation)
        && record.pid == observation.pid
        && record.creation_time_100ns == observation.creation_time_100ns
        && canonical_windows_path(&record.executable_path)
            == canonical_windows_path(&observation.executable_path)
        && collector_arguments_sha256(&observation.arguments)
            .is_ok_and(|hash| hash == record.arguments_sha256)
}

pub fn collector_record_matches_expectation(
    record: &CollectorProcessIdentityV1,
    expectation: &CollectorProcessExpectation,
) -> bool {
    valid_record(record)
        && valid_expectation(expectation)
        && record.account_id == expectation.account_id
        && record.sync_job_id == expectation.sync_job_id
        && record.phase == expectation.phase
        && record.phase_attempt_id == expectation.phase_attempt_id
        && canonical_windows_path(&record.executable_path)
            == canonical_windows_path(&expectation.executable_path)
        && record.arguments_sha256 == expectation.arguments_sha256
}

pub fn login_collector_record_matches_expectation(
    record: &LoginCollectorProcessIdentityV1,
    expectation: &LoginCollectorProcessExpectation,
) -> bool {
    valid_login_record(record)
        && valid_login_expectation(expectation)
        && record.login_attempt_id == expectation.login_attempt_id
        && record.profile_key == expectation.profile_key
        && record.target_account_id == expectation.target_account_id
        && canonical_windows_path(&record.executable_path)
            == canonical_windows_path(&expectation.executable_path)
        && record.arguments_sha256 == expectation.arguments_sha256
}

pub fn terminate_login_collector_if_matches<F>(
    record: &LoginCollectorProcessIdentityV1,
    observation: &CollectorProcessObservation,
    expectation: &LoginCollectorProcessExpectation,
    terminate: F,
) -> Result<bool, String>
where
    F: FnOnce(u32) -> Result<(), String>,
{
    if !login_collector_process_identity_matches(record, observation, expectation) {
        return Ok(false);
    }
    terminate(record.pid)?;
    Ok(true)
}

fn valid_record(record: &CollectorProcessIdentityV1) -> bool {
    record.schema_version == 1
        && record.pid != 0
        && !record.creation_time_100ns.is_empty()
        && !record.executable_path.is_empty()
        && is_sha256(&record.arguments_sha256)
        && record.account_id > 0
        && record.sync_job_id > 0
        && is_simple_uuid(&record.phase_attempt_id)
        && DateTime::parse_from_rfc3339(&record.started_at).is_ok()
}

fn valid_expectation(expectation: &CollectorProcessExpectation) -> bool {
    expectation.account_id > 0
        && expectation.sync_job_id > 0
        && is_simple_uuid(&expectation.phase_attempt_id)
        && !expectation.executable_path.is_empty()
        && is_sha256(&expectation.arguments_sha256)
}

fn valid_login_record(record: &LoginCollectorProcessIdentityV1) -> bool {
    record.schema_version == 1
        && record.pid != 0
        && !record.creation_time_100ns.is_empty()
        && !record.executable_path.is_empty()
        && is_sha256(&record.arguments_sha256)
        && is_simple_uuid(&record.login_attempt_id)
        && is_simple_uuid(&record.profile_key)
        && record.target_account_id.is_none_or(|value| value > 0)
        && DateTime::parse_from_rfc3339(&record.started_at).is_ok()
}

fn valid_login_expectation(expectation: &LoginCollectorProcessExpectation) -> bool {
    is_simple_uuid(&expectation.login_attempt_id)
        && is_simple_uuid(&expectation.profile_key)
        && expectation.target_account_id.is_none_or(|value| value > 0)
        && !expectation.executable_path.is_empty()
        && is_sha256(&expectation.arguments_sha256)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
