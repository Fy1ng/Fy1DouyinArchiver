use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const APPLICATION_ID: &str = "lifearchive.douyin-history-archive";
pub const BOOTSTRAP_FILE: &str = "bootstrap.json";
pub const DATA_ROOT_MARKER_FILE: &str = ".emily-data-root.json";
const BOOTSTRAP_SCHEMA_VERSION: i64 = 1;
const DATA_ROOT_MARKER_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataRootRefV1 {
    pub path: PathBuf,
    pub root_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RetainedLayoutV1 {
    Standard,
    LegacyFlat,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetainedDataRootV1 {
    pub root: DataRootRefV1,
    pub layout: RetainedLayoutV1,
    #[serde(default)]
    pub legacy_entries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapV1 {
    pub schema_version: i64,
    pub current_root: DataRootRefV1,
    pub active_account_id: Option<i64>,
    pub retained_data_roots: Vec<RetainedDataRootV1>,
}

impl BootstrapV1 {
    pub fn new(path: PathBuf, root_id: String) -> Result<Self, String> {
        let value = Self {
            schema_version: BOOTSTRAP_SCHEMA_VERSION,
            current_root: DataRootRefV1 { path, root_id },
            active_account_id: None,
            retained_data_roots: Vec::new(),
        };
        validate_bootstrap(&value)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataRootMarkerV1 {
    pub schema_version: i64,
    pub application_id: String,
    pub root_id: String,
    pub created_at: String,
}

impl DataRootMarkerV1 {
    pub fn new(root_id: String) -> Result<Self, String> {
        if !valid_root_id(&root_id) {
            return Err("invalid_data_root_id".to_string());
        }
        Ok(Self {
            schema_version: DATA_ROOT_MARKER_SCHEMA_VERSION,
            application_id: APPLICATION_ID.to_string(),
            root_id,
            created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        })
    }
}

pub fn parse_bootstrap(raw: &str) -> Result<BootstrapV1, String> {
    let value = serde_json::from_str::<BootstrapV1>(raw)
        .map_err(|error| format!("invalid_bootstrap:{error}"))?;
    validate_bootstrap(&value)?;
    Ok(value)
}

pub fn read_bootstrap(directory: &Path) -> Result<Option<BootstrapV1>, String> {
    let path = directory.join(BOOTSTRAP_FILE);
    let raw = match fs::read_to_string(io_path(&path)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    parse_bootstrap(&raw).map(Some)
}

pub fn write_bootstrap_atomic(directory: &Path, value: &BootstrapV1) -> Result<(), String> {
    validate_bootstrap(value)?;
    write_json_atomically(&directory.join(BOOTSTRAP_FILE), value)
}

pub fn update_active_account(
    directory: &Path,
    active_account_id: Option<i64>,
) -> Result<BootstrapV1, String> {
    if active_account_id.is_some_and(|value| value <= 0) {
        return Err("invalid_active_account_id".to_string());
    }
    let mut bootstrap =
        read_bootstrap(directory)?.ok_or_else(|| "bootstrap_missing".to_string())?;
    bootstrap.active_account_id = active_account_id;
    write_bootstrap_atomic(directory, &bootstrap)?;
    Ok(bootstrap)
}

pub fn write_data_root_marker_atomic(root: &Path, marker: &DataRootMarkerV1) -> Result<(), String> {
    validate_marker(marker)?;
    fs::create_dir_all(io_path(root)).map_err(|error| error.to_string())?;
    write_json_atomically(&root.join(DATA_ROOT_MARKER_FILE), marker)
}

pub fn validate_data_root_marker(root: &Path, expected: &DataRootRefV1) -> Result<(), String> {
    validate_root_ref(expected)?;
    if !same_windows_path(root, &expected.path) {
        return Err("data_root_path_mismatch".to_string());
    }
    let marker = read_json::<DataRootMarkerV1>(&root.join(DATA_ROOT_MARKER_FILE))
        .map_err(|_| "data_root_marker_missing_or_invalid".to_string())?;
    validate_marker(&marker)?;
    if marker.root_id != expected.root_id {
        return Err("data_root_identity_mismatch".to_string());
    }
    Ok(())
}

fn validate_bootstrap(value: &BootstrapV1) -> Result<(), String> {
    if value.schema_version != BOOTSTRAP_SCHEMA_VERSION {
        return Err("unsupported_bootstrap_schema".to_string());
    }
    validate_root_ref(&value.current_root)?;
    if value
        .active_account_id
        .is_some_and(|account_id| account_id <= 0)
    {
        return Err("invalid_active_account_id".to_string());
    }
    let mut root_ids = HashSet::new();
    root_ids.insert(value.current_root.root_id.as_str());
    for retained in &value.retained_data_roots {
        validate_root_ref(&retained.root)?;
        if !root_ids.insert(retained.root.root_id.as_str()) {
            return Err("duplicate_data_root_id".to_string());
        }
        match retained.layout {
            RetainedLayoutV1::Standard if !retained.legacy_entries.is_empty() => {
                return Err("invalid_retained_data_root".to_string());
            }
            RetainedLayoutV1::LegacyFlat => {
                if retained.legacy_entries.is_empty()
                    || retained
                        .legacy_entries
                        .iter()
                        .any(|entry| !valid_legacy_entry(entry))
                {
                    return Err("invalid_retained_data_root".to_string());
                }
            }
            RetainedLayoutV1::Standard => {}
        }
    }
    Ok(())
}

fn validate_root_ref(value: &DataRootRefV1) -> Result<(), String> {
    if !value.path.is_absolute() || value.path.to_str().is_none() {
        return Err("invalid_data_root_path".to_string());
    }
    if !valid_root_id(&value.root_id) {
        return Err("invalid_data_root_id".to_string());
    }
    Ok(())
}

fn validate_marker(value: &DataRootMarkerV1) -> Result<(), String> {
    if value.schema_version != DATA_ROOT_MARKER_SCHEMA_VERSION
        || value.application_id != APPLICATION_ID
        || !valid_root_id(&value.root_id)
        || chrono::DateTime::parse_from_rfc3339(&value.created_at).is_err()
    {
        return Err("invalid_data_root_marker".to_string());
    }
    Ok(())
}

fn valid_root_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_legacy_entry(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path.components().count() == 1
        && !value.contains(['/', '\\'])
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    normalize_windows_path(left) == normalize_windows_path(right)
}

fn normalize_windows_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(io_path(path)).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub(crate) fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "atomic_path_has_no_parent".to_string())?;
    fs::create_dir_all(io_path(parent)).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "invalid_atomic_file_name".to_string())?;
    let temporary = path.with_file_name(format!(".{name}.{}.{nonce}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(io_path(&temporary))
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(io_path(&temporary));
    }
    result
}

#[cfg(target_os = "windows")]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = io_path(source);
    let target = io_path(target);
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|error| error.to_string())
}

#[cfg(target_os = "windows")]
fn io_path(path: &Path) -> PathBuf {
    let value = path.to_string_lossy().replace('/', "\\");
    if value.starts_with(r"\\?\") {
        return PathBuf::from(value);
    }
    if let Some(rest) = value.strip_prefix(r"\\") {
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }
    if path.is_absolute() {
        return PathBuf::from(format!(r"\\?\{value}"));
    }
    path.to_path_buf()
}

#[cfg(not(target_os = "windows"))]
fn io_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}
