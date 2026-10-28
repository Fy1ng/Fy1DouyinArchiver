use super::{browser_discovery::{discover_system_browser, DiscoveredBrowser}, browser_state::{browser_profile_path, canonical_windows_path, ensure_account_profile_root, ensure_profile_generation, read_browser_selection, read_login_validation, write_browser_selection, write_login_validation, BrowserId, BrowserSelection, LoginValidation}, cli::BrowserPreference};
use chrono::{SecondsFormat, Utc};
use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Clone)]
pub struct BrowserChoice { pub browser_id: BrowserId, pub executable_path: PathBuf, pub profile_path: PathBuf }

pub fn resolve_browser(account_profile_root: &Path, preference: BrowserPreference, headless: bool) -> Result<BrowserChoice, String> {
    ensure_account_profile_root(account_profile_root)?;
    let selection = read_browser_selection(account_profile_root)?;
    if let Some(selection) = selection {
        let selected_preference = match selection.browser_id { BrowserId::Edge => BrowserPreference::Edge, BrowserId::Chrome => BrowserPreference::Chrome };
        if preference != BrowserPreference::Auto && preference != selected_preference { return Err("browser_lock_invalid".into()); }
        let discovered = discover_system_browser(selected_preference)?;
        let expected_profile = browser_profile_path(account_profile_root, selection.browser_id);
        if !same_path(&discovered.executable_path, Path::new(&selection.executable_path)) { return Err("browser_lock_invalid".into()); }
        if headless && read_login_validation(account_profile_root, &selection)?.is_none() { return Err("login_validation_required".into()); }
        return Ok(choice(discovered, expected_profile));
    }
    if headless { return Err("login_validation_required".into()); }
    let discovered = discover_system_browser(preference)?;
    let profile = browser_profile_path(account_profile_root, discovered.browser_id);
    Ok(choice(discovered, profile))
}

pub fn resolve_download_browser(account_profile_root: &Path) -> Result<BrowserChoice, String> {
    resolve_browser(account_profile_root, BrowserPreference::Auto, true)
}

pub fn persist_validation(account_profile_root: &Path, choice: &BrowserChoice, history_url: &str) -> Result<(), String> {
    persist_validation_with_id(account_profile_root, choice, history_url, None)
}

pub fn persist_validation_with_id(account_profile_root: &Path, choice: &BrowserChoice, history_url: &str, validation_id: Option<&str>) -> Result<(), String> {
    fs::create_dir_all(&choice.profile_path).map_err(|error| error.to_string())?;
    let profile_generation = ensure_profile_generation(account_profile_root, choice.browser_id)?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let selection = BrowserSelection { schema_version: 3, browser_id: choice.browser_id, executable_path: display_path(&fs::canonicalize(&choice.executable_path).map_err(|e|e.to_string())?), profile_path: String::new(), profile_generation: Some(profile_generation.clone()), selected_at: now.clone() };
    write_browser_selection(account_profile_root, &selection)?;
    write_login_validation(account_profile_root, &selection, &LoginValidation { schema_version: 4, validation_id: validation_id.map(str::to_string), browser_id: selection.browser_id, executable_path: selection.executable_path.clone(), profile_path: String::new(), profile_generation: Some(profile_generation), validated_at: now, history_url: history_url.to_string() })
}

fn choice(discovered: DiscoveredBrowser, profile_path: PathBuf) -> BrowserChoice { BrowserChoice { browser_id: discovered.browser_id, executable_path: discovered.executable_path, profile_path } }
fn same_path(left: &Path, right: &Path) -> bool { canonical_windows_path(&display_path(left)) == canonical_windows_path(&display_path(right)) }
fn display_path(path: &Path) -> String { let raw=path.to_string_lossy(); if let Some(rest)=raw.strip_prefix(r"\\?\UNC\"){format!(r"\\{rest}")}else if let Some(rest)=raw.strip_prefix(r"\\?\"){rest.to_string()}else{raw.to_string()} }
