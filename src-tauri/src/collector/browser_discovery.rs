use super::{browser_state::BrowserId, cli::BrowserPreference};
use std::{env, fs, path::{Path, PathBuf}};
use winreg::{enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY}, RegKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredBrowser {
    pub browser_id: BrowserId,
    pub executable_path: PathBuf,
}

pub fn discover_system_browser(preference: BrowserPreference) -> Result<DiscoveredBrowser, String> {
    discover_from_candidates(
        preference,
        browser_candidates(BrowserId::Edge),
        browser_candidates(BrowserId::Chrome),
    )
}

pub fn discover_from_candidates(
    preference: BrowserPreference,
    edge_candidates: Vec<PathBuf>,
    chrome_candidates: Vec<PathBuf>,
) -> Result<DiscoveredBrowser, String> {
    let choices = match preference {
        BrowserPreference::Auto => vec![(BrowserId::Edge, edge_candidates), (BrowserId::Chrome, chrome_candidates)],
        BrowserPreference::Edge => vec![(BrowserId::Edge, edge_candidates)],
        BrowserPreference::Chrome => vec![(BrowserId::Chrome, chrome_candidates)],
    };
    for (browser_id, candidates) in choices {
        for candidate in candidates {
            if let Some(executable_path) = canonical_file(&candidate) {
                return Ok(DiscoveredBrowser { browser_id, executable_path });
            }
        }
    }
    Err("system_browser_not_found".to_string())
}

fn browser_candidates(browser_id: BrowserId) -> Vec<PathBuf> {
    let mut candidates = registry_candidates(browser_id);
    let (relative, executable) = match browser_id {
        BrowserId::Edge => (["Microsoft", "Edge", "Application"], "msedge.exe"),
        BrowserId::Chrome => (["Google", "Chrome", "Application"], "chrome.exe"),
    };
    let override_key = match browser_id {
        BrowserId::Edge => "EDGE_EXECUTABLE_PATH",
        BrowserId::Chrome => "CHROME_EXECUTABLE_PATH",
    };
    if let Some(value) = env::var_os(override_key) {
        candidates.insert(0, PathBuf::from(value));
    }
    for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = env::var_os(key) {
            candidates.push(PathBuf::from(root).join(relative.join("\\")).join(executable));
        }
    }
    if let Some(root) = env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(root).join(relative.join("\\")).join(executable));
    }
    candidates
}

fn registry_candidates(browser_id: BrowserId) -> Vec<PathBuf> {
    let executable = match browser_id {
        BrowserId::Edge => "msedge.exe",
        BrowserId::Chrome => "chrome.exe",
    };
    let subkey = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{executable}");
    let mut values = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for access in [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY] {
            let root = RegKey::predef(hive);
            if let Ok(key) = root.open_subkey_with_flags(&subkey, access) {
                if let Ok(value) = key.get_value::<String, _>("") {
                    values.push(PathBuf::from(value));
                }
            }
        }
    }
    values
}

fn canonical_file(path: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(path).ok()?;
    fs::metadata(&canonical).ok()?.is_file().then_some(canonical)
}
