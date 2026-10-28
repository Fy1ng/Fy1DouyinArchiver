use super::file_store::{read_json, write_json_atomically};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const LEGACY_SELECTION_SCHEMA_VERSION: i64 = 1;
const PATHLESS_SELECTION_SCHEMA_VERSION: i64 = 2;
const SELECTION_SCHEMA_VERSION: i64 = 3;
const LOGIN_VALIDATION_SCHEMA_VERSION: i64 = 4;
const PROFILE_GENERATION_SCHEMA_VERSION: i64 = 1;
const SELECTION_FILE: &str = "browser-selection.json";
const LOGIN_VALIDATION_FILE: &str = ".login-validated.json";
const PROFILE_GENERATION_FILE: &str = "profile-generation.json";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BrowserId {
    Edge,
    Chrome,
}

impl BrowserId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edge => "edge",
            Self::Chrome => "chrome",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserSelection {
    pub schema_version: i64,
    pub browser_id: BrowserId,
    pub executable_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_generation: Option<String>,
    pub selected_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginValidation {
    pub schema_version: i64,
    #[serde(default)]
    pub validation_id: Option<String>,
    pub browser_id: BrowserId,
    pub executable_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_generation: Option<String>,
    pub validated_at: String,
    pub history_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfileGenerationV1 {
    schema_version: i64,
    browser_id: BrowserId,
    profile_generation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserProfileIdentityV1 {
    pub browser_id: BrowserId,
    pub executable_path: String,
    pub profile_path: String,
    pub profile_generation: String,
}

pub fn browser_profile_path(account_profile_root: &Path, browser_id: BrowserId) -> PathBuf {
    account_profile_root.join(browser_id.as_str())
}

pub fn profile_generation_path(account_profile_root: &Path, browser_id: BrowserId) -> PathBuf {
    browser_profile_path(account_profile_root, browser_id).join(PROFILE_GENERATION_FILE)
}

pub fn ensure_account_profile_root(account_profile_root: &Path) -> Result<(), String> {
    if account_profile_root.exists() {
        return validate_account_root_filesystem_scope(account_profile_root);
    }
    let parent = account_profile_root
        .parent()
        .ok_or_else(|| "browser_profile_scope_mismatch".to_string())?;
    validate_directory(parent).map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    match fs::create_dir(account_profile_root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    validate_account_root_filesystem_scope(account_profile_root)
}

pub fn ensure_profile_generation(
    account_profile_root: &Path,
    browser_id: BrowserId,
) -> Result<String, String> {
    ensure_account_profile_root(account_profile_root)?;
    let profile = browser_profile_path(account_profile_root, browser_id);
    fs::create_dir_all(&profile).map_err(|error| error.to_string())?;
    validate_profile_filesystem_scope(account_profile_root, &profile)?;
    let path = profile_generation_path(account_profile_root, browser_id);
    if path.exists() {
        return read_profile_generation(&path, browser_id);
    }

    let profile_generation = Uuid::new_v4().simple().to_string();
    write_json_atomically(
        &path,
        &ProfileGenerationV1 {
            schema_version: PROFILE_GENERATION_SCHEMA_VERSION,
            browser_id,
            profile_generation: profile_generation.clone(),
        },
    )?;
    read_profile_generation(&path, browser_id)
}

pub fn write_browser_selection(
    account_profile_root: &Path,
    selection: &BrowserSelection,
) -> Result<(), String> {
    validate_selection(account_profile_root, selection)?;
    let profile = browser_profile_path(account_profile_root, selection.browser_id);
    fs::create_dir_all(&profile).map_err(|error| error.to_string())?;
    validate_profile_filesystem_scope(account_profile_root, &profile)?;
    validate_selection_generation(&profile, selection)?;
    write_json_atomically(&account_profile_root.join(SELECTION_FILE), selection)
}

pub fn read_browser_selection(
    account_profile_root: &Path,
) -> Result<Option<BrowserSelection>, String> {
    validate_account_root_filesystem_scope(account_profile_root)?;
    let path = account_profile_root.join(SELECTION_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let selection = read_json::<BrowserSelection>(&path)?;
    validate_selection(account_profile_root, &selection)?;
    let profile = browser_profile_path(account_profile_root, selection.browser_id);
    validate_profile_filesystem_scope(account_profile_root, &profile)
        .map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    if selection.schema_version == SELECTION_SCHEMA_VERSION {
        validate_selection_generation(&profile, &selection)?;
        return Ok(Some(selection));
    }

    let profile_generation = ensure_profile_generation(account_profile_root, selection.browser_id)?;
    let upgraded = BrowserSelection {
        schema_version: SELECTION_SCHEMA_VERSION,
        browser_id: selection.browser_id,
        executable_path: selection.executable_path,
        profile_path: String::new(),
        profile_generation: Some(profile_generation),
        selected_at: selection.selected_at,
    };
    write_browser_selection(account_profile_root, &upgraded)?;
    Ok(Some(upgraded))
}

pub fn clear_browser_selection(account_profile_root: &Path) -> Result<bool, String> {
    validate_account_root_filesystem_scope(account_profile_root)?;
    let path = account_profile_root.join(SELECTION_FILE);
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

pub fn write_login_validation(
    account_profile_root: &Path,
    selection: &BrowserSelection,
    validation: &LoginValidation,
) -> Result<(), String> {
    validate_selection(account_profile_root, selection)?;
    let expected_profile = browser_profile_path(account_profile_root, validation.browser_id);
    validate_selection_generation(&expected_profile, selection)?;
    if !valid_login_validation_version(validation)
        || validation.browser_id != selection.browser_id
        || canonical_windows_path(&validation.executable_path)
            != canonical_windows_path(&selection.executable_path)
        || !(validation.history_url.starts_with("https://")
            || validation.history_url.starts_with("http://"))
        || (validation.schema_version == LOGIN_VALIDATION_SCHEMA_VERSION
            && validation.profile_generation != selection.profile_generation)
    {
        return Err("invalid_login_validation".to_string());
    }
    validate_validation_profile(&expected_profile, selection, validation)?;
    validate_profile_filesystem_scope(account_profile_root, &expected_profile)
        .map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    write_json_atomically(&expected_profile.join(LOGIN_VALIDATION_FILE), validation)
}

pub fn read_login_validation(
    account_profile_root: &Path,
    selection: &BrowserSelection,
) -> Result<Option<LoginValidation>, String> {
    validate_selection(account_profile_root, selection)?;
    let Some(current_selection) = read_browser_selection(account_profile_root)? else {
        return Ok(None);
    };
    if current_selection.browser_id != selection.browser_id
        || canonical_windows_path(&current_selection.executable_path)
            != canonical_windows_path(&selection.executable_path)
    {
        return Ok(None);
    }
    let expected_profile = browser_profile_path(account_profile_root, current_selection.browser_id);
    validate_profile_filesystem_scope(account_profile_root, &expected_profile)
        .map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    let path = expected_profile.join(LOGIN_VALIDATION_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let validation = read_json::<LoginValidation>(&path)?;
    if !valid_login_validation_version(&validation)
        || validation.browser_id != current_selection.browser_id
        || canonical_windows_path(&validation.executable_path)
            != canonical_windows_path(&current_selection.executable_path)
    {
        return Ok(None);
    }
    let expected_profile = browser_profile_path(account_profile_root, validation.browser_id);
    if let Err(error) =
        validate_validation_profile(&expected_profile, &current_selection, &validation)
    {
        return if error == "browser_profile_scope_mismatch" {
            Err(error)
        } else {
            Ok(None)
        };
    }
    let profile_generation = current_selection
        .profile_generation
        .clone()
        .ok_or_else(|| "browser_profile_changed".to_string())?;
    if validation.schema_version == LOGIN_VALIDATION_SCHEMA_VERSION {
        if validation.profile_generation.as_deref() != Some(profile_generation.as_str()) {
            return Err("browser_profile_changed".to_string());
        }
        return Ok(Some(validation));
    }

    let upgraded = LoginValidation {
        schema_version: LOGIN_VALIDATION_SCHEMA_VERSION,
        validation_id: validation.validation_id,
        browser_id: validation.browser_id,
        executable_path: validation.executable_path,
        profile_path: String::new(),
        profile_generation: Some(profile_generation),
        validated_at: validation.validated_at,
        history_url: validation.history_url,
    };
    write_login_validation(account_profile_root, &current_selection, &upgraded)?;
    Ok(Some(upgraded))
}

pub fn read_matching_browser_profile_identity(
    account_profile_root: &Path,
    expected: &BrowserProfileIdentityV1,
) -> Result<BrowserProfileIdentityV1, String> {
    let selection = read_browser_selection(account_profile_root)?
        .ok_or_else(|| "browser_profile_changed".to_string())?;
    let validation = read_login_validation(account_profile_root, &selection)?
        .ok_or_else(|| "browser_profile_changed".to_string())?;
    let profile_generation = ensure_profile_generation(account_profile_root, selection.browser_id)?;
    let current = BrowserProfileIdentityV1 {
        browser_id: selection.browser_id,
        executable_path: selection.executable_path.clone(),
        profile_path: browser_profile_path(account_profile_root, selection.browser_id)
            .to_string_lossy()
            .into_owned(),
        profile_generation,
    };
    if current.browser_id != expected.browser_id
        || canonical_windows_path(&current.executable_path)
            != canonical_windows_path(&expected.executable_path)
        || canonical_windows_path(&current.profile_path)
            != canonical_windows_path(&expected.profile_path)
        || current.profile_generation != expected.profile_generation
        || validation.profile_generation.as_deref() != Some(current.profile_generation.as_str())
    {
        return Err("browser_profile_changed".to_string());
    }
    Ok(current)
}

pub fn remove_login_validation_if_matches(
    account_profile_root: &Path,
    selection: &BrowserSelection,
    validation_id: &str,
) -> Result<bool, String> {
    if !valid_validation_id(validation_id) {
        return Err("invalid_login_validation".to_string());
    }
    let Some(validation) = read_login_validation(account_profile_root, selection)? else {
        return Ok(false);
    };
    if !matches!(validation.schema_version, 2 | 3 | 4)
        || validation.validation_id.as_deref() != Some(validation_id)
    {
        return Ok(false);
    }
    fs::remove_file(
        browser_profile_path(account_profile_root, selection.browser_id)
            .join(LOGIN_VALIDATION_FILE),
    )
    .map_err(|error| error.to_string())?;
    Ok(true)
}

fn valid_login_validation_version(validation: &LoginValidation) -> bool {
    match validation.schema_version {
        1 => {
            validation.validation_id.is_none()
                && !validation.profile_path.is_empty()
                && validation.profile_generation.is_none()
        }
        2 => {
            validation
                .validation_id
                .as_deref()
                .is_some_and(valid_validation_id)
                && !validation.profile_path.is_empty()
                && validation.profile_generation.is_none()
        }
        3 => {
            validation.profile_path.is_empty()
                && validation.profile_generation.is_none()
                && validation
                    .validation_id
                    .as_deref()
                    .is_none_or(valid_validation_id)
        }
        LOGIN_VALIDATION_SCHEMA_VERSION => {
            validation.profile_path.is_empty()
                && validation
                    .profile_generation
                    .as_deref()
                    .is_some_and(valid_validation_id)
                && validation
                    .validation_id
                    .as_deref()
                    .is_none_or(valid_validation_id)
        }
        _ => false,
    }
}

fn valid_validation_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn canonical_windows_path(value: &str) -> String {
    let path_value = if value
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", value.get(8..).unwrap_or_default())
    } else if value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\"))
    {
        value.get(4..).unwrap_or_default().to_string()
    } else {
        value.to_string()
    };
    let mut parts = Vec::new();
    let path = Path::new(&path_value);
    let mut prefix = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = value.as_os_str().to_string_lossy().to_string(),
            Component::RootDir => {}
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
        }
    }
    let mut result = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}\\")
    };
    result.push_str(&parts.join("\\"));
    result.trim_end_matches('\\').to_lowercase()
}

fn validate_selection(
    account_profile_root: &Path,
    selection: &BrowserSelection,
) -> Result<(), String> {
    if selection.executable_path.is_empty() || selection.selected_at.is_empty() {
        return Err("invalid_browser_selection".to_string());
    }
    let expected_profile = browser_profile_path(account_profile_root, selection.browser_id);
    match selection.schema_version {
        LEGACY_SELECTION_SCHEMA_VERSION => {
            if selection.profile_path.is_empty()
                || canonical_windows_path(&selection.profile_path)
                    != canonical_windows_path(&expected_profile.to_string_lossy())
            {
                return Err("browser_profile_scope_mismatch".to_string());
            }
        }
        PATHLESS_SELECTION_SCHEMA_VERSION
            if selection.profile_path.is_empty() && selection.profile_generation.is_none() => {}
        PATHLESS_SELECTION_SCHEMA_VERSION => return Err("invalid_browser_selection".to_string()),
        SELECTION_SCHEMA_VERSION
            if selection.profile_path.is_empty()
                && selection
                    .profile_generation
                    .as_deref()
                    .is_some_and(valid_validation_id) => {}
        SELECTION_SCHEMA_VERSION => return Err("invalid_browser_selection".to_string()),
        _ => return Err("invalid_browser_selection".to_string()),
    }
    if selection.schema_version == LEGACY_SELECTION_SCHEMA_VERSION
        && selection.profile_generation.is_some()
    {
        return Err("invalid_browser_selection".to_string());
    }
    Ok(())
}

fn validate_selection_generation(
    profile: &Path,
    selection: &BrowserSelection,
) -> Result<(), String> {
    if selection.schema_version != SELECTION_SCHEMA_VERSION {
        return Ok(());
    }
    let generation =
        read_profile_generation(&profile.join(PROFILE_GENERATION_FILE), selection.browser_id)?;
    if selection.profile_generation.as_deref() != Some(generation.as_str()) {
        return Err("browser_profile_changed".to_string());
    }
    Ok(())
}

fn read_profile_generation(path: &Path, browser_id: BrowserId) -> Result<String, String> {
    let value = read_json::<ProfileGenerationV1>(path)
        .map_err(|_| "browser_profile_changed".to_string())?;
    if value.schema_version != PROFILE_GENERATION_SCHEMA_VERSION
        || value.browser_id != browser_id
        || !valid_validation_id(&value.profile_generation)
    {
        return Err("browser_profile_changed".to_string());
    }
    Ok(value.profile_generation)
}

fn validate_validation_profile(
    expected_profile: &Path,
    selection: &BrowserSelection,
    validation: &LoginValidation,
) -> Result<(), String> {
    if matches!(
        validation.schema_version,
        3 | LOGIN_VALIDATION_SCHEMA_VERSION
    ) {
        return validation
            .profile_path
            .is_empty()
            .then_some(())
            .ok_or_else(|| "invalid_login_validation".to_string());
    }
    if canonical_windows_path(&validation.profile_path)
        != canonical_windows_path(&expected_profile.to_string_lossy())
    {
        return Err("browser_profile_scope_mismatch".to_string());
    }
    if !selection.profile_path.is_empty()
        && canonical_windows_path(&validation.profile_path)
            != canonical_windows_path(&selection.profile_path)
    {
        return Err("invalid_login_validation".to_string());
    }
    Ok(())
}

pub fn rewrite_browser_state_without_profile_paths(
    account_profile_root: &Path,
) -> Result<(), String> {
    let Some(selection) = read_browser_selection(account_profile_root)? else {
        return Ok(());
    };
    let validation = read_login_validation(account_profile_root, &selection)?;
    let rewritten_selection = BrowserSelection {
        schema_version: SELECTION_SCHEMA_VERSION,
        browser_id: selection.browser_id,
        executable_path: selection.executable_path,
        profile_path: String::new(),
        profile_generation: selection.profile_generation,
        selected_at: selection.selected_at,
    };
    write_browser_selection(account_profile_root, &rewritten_selection)?;
    if let Some(validation) = validation {
        write_login_validation(
            account_profile_root,
            &rewritten_selection,
            &LoginValidation {
                schema_version: LOGIN_VALIDATION_SCHEMA_VERSION,
                validation_id: validation.validation_id,
                browser_id: validation.browser_id,
                executable_path: validation.executable_path,
                profile_path: String::new(),
                profile_generation: rewritten_selection.profile_generation.clone(),
                validated_at: validation.validated_at,
                history_url: validation.history_url,
            },
        )?;
    }
    Ok(())
}

pub fn rewrite_browser_state_for_migration(
    source_account_profile_root: &Path,
    target_account_profile_root: &Path,
) -> Result<(), String> {
    let Some(selection) = read_browser_selection(source_account_profile_root)? else {
        return Ok(());
    };
    let validation = read_login_validation(source_account_profile_root, &selection)?;
    let rewritten_selection = BrowserSelection {
        schema_version: SELECTION_SCHEMA_VERSION,
        browser_id: selection.browser_id,
        executable_path: selection.executable_path,
        profile_path: String::new(),
        profile_generation: selection.profile_generation,
        selected_at: selection.selected_at,
    };
    let target_profile =
        browser_profile_path(target_account_profile_root, rewritten_selection.browser_id);
    fs::create_dir_all(&target_profile).map_err(|error| error.to_string())?;
    validate_profile_filesystem_scope(target_account_profile_root, &target_profile)?;
    let source_generation = rewritten_selection
        .profile_generation
        .clone()
        .ok_or_else(|| "browser_profile_changed".to_string())?;
    let target_generation_path = target_profile.join(PROFILE_GENERATION_FILE);
    if target_generation_path.exists() {
        if read_profile_generation(&target_generation_path, rewritten_selection.browser_id)?
            != source_generation
        {
            return Err("browser_profile_changed".to_string());
        }
    } else {
        write_json_atomically(
            &target_generation_path,
            &ProfileGenerationV1 {
                schema_version: PROFILE_GENERATION_SCHEMA_VERSION,
                browser_id: rewritten_selection.browser_id,
                profile_generation: source_generation,
            },
        )?;
    }
    write_browser_selection(target_account_profile_root, &rewritten_selection)?;
    if let Some(validation) = validation {
        write_login_validation(
            target_account_profile_root,
            &rewritten_selection,
            &LoginValidation {
                schema_version: LOGIN_VALIDATION_SCHEMA_VERSION,
                validation_id: validation.validation_id,
                browser_id: validation.browser_id,
                executable_path: validation.executable_path,
                profile_path: String::new(),
                profile_generation: rewritten_selection.profile_generation.clone(),
                validated_at: validation.validated_at,
                history_url: validation.history_url,
            },
        )?;
    }
    Ok(())
}

pub(crate) fn validate_profile_filesystem_scope(
    account_profile_root: &Path,
    profile: &Path,
) -> Result<(), String> {
    reject_link(account_profile_root)?;
    reject_link(profile)?;
    let account = fs::canonicalize(account_profile_root)
        .map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    let canonical_profile =
        fs::canonicalize(profile).map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    if canonical_profile.parent() != Some(account.as_path()) {
        return Err("browser_profile_scope_mismatch".to_string());
    }
    Ok(())
}

fn validate_account_root_filesystem_scope(account_profile_root: &Path) -> Result<(), String> {
    let parent = account_profile_root
        .parent()
        .ok_or_else(|| "browser_profile_scope_mismatch".to_string())?;
    validate_directory(parent).map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    validate_directory(account_profile_root)
        .map_err(|_| "browser_profile_scope_mismatch".to_string())?;
    fs::canonicalize(account_profile_root)
        .map(|_| ())
        .map_err(|_| "browser_profile_scope_mismatch".to_string())
}

pub(crate) fn validate_directory(path: &Path) -> Result<(), String> {
    reject_link(path)?;
    if !fs::metadata(path)
        .map_err(|error| error.to_string())?
        .is_dir()
    {
        return Err("browser_profile_not_directory".to_string());
    }
    Ok(())
}

pub(crate) fn reject_link(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("browser_profile_unsafe_link".to_string());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("browser_profile_unsafe_link".to_string());
        }
    }
    Ok(())
}
