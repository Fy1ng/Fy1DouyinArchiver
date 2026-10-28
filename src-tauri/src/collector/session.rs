use super::{
    browser_process::ManagedBrowser,
    browser_state::{ensure_profile_generation, BrowserId},
    cdp::CdpPage,
    deadline::Deadline,
    profile_constraints::apply_profile_constraints,
};
use std::{fs, path::{Path, PathBuf}, time::Duration};

pub struct CdpBrowserSession {
    pub page: CdpPage,
    pub process: ManagedBrowser,
    pub port: u16,
    pub browser_websocket_path: String,
    pub user_data_dir_argument: String,
    pub profile_path: PathBuf,
}

pub fn browser_launch_arguments(profile: &Path, headless: bool) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.to_string_lossy()),
        "--remote-debugging-address=127.0.0.1".into(), "--remote-debugging-port=0".into(),
        "--disable-extensions".into(),
        "--disable-component-extensions-with-background-pages".into(),
        "--disable-default-apps".into(),
        "--disable-sync".into(),
        "--disable-background-mode".into(),
        "--no-first-run".into(), "--no-default-browser-check".into(), "--window-size=960,900".into(),
    ];
    if headless { args.extend(["--headless=new".into(), "--no-startup-window".into(), "--disable-direct-composition".into()]); }
    else { args.push("about:blank".into()); }
    args
}

pub fn prepare_browser_launch(
    account_profile_root: &Path,
    browser_id: BrowserId,
    headless: bool,
) -> Result<(PathBuf, Vec<String>), String> {
    let profile = apply_profile_constraints(account_profile_root, browser_id)?;
    ensure_profile_generation(account_profile_root, browser_id)?;
    let args = browser_launch_arguments(&profile, headless);
    Ok((profile, args))
}

pub async fn launch_browser_session(
    executable: &Path,
    account_profile_root: &Path,
    browser_id: BrowserId,
    headless: bool,
) -> Result<CdpBrowserSession, String> {
    launch_browser_session_with_deadline(executable, account_profile_root, browser_id, headless, Deadline::after(Duration::from_secs(15))).await
}

pub async fn launch_browser_session_with_deadline(
    executable: &Path,
    account_profile_root: &Path,
    browser_id: BrowserId,
    headless: bool,
    deadline: Deadline,
) -> Result<CdpBrowserSession, String> {
    let (profile, args) = prepare_browser_launch(account_profile_root, browser_id, headless)?;
    let port_file = profile.join("DevToolsActivePort"); let _ = fs::remove_file(&port_file);
    let process = ManagedBrowser::launch(executable, &args, headless)?;
    let (port, browser_websocket_path) = loop {
        if deadline.remaining().is_err() { return Err("collector_deadline_exceeded".into()); }
        if let Ok(contents) = fs::read_to_string(&port_file) {
            if let Ok(endpoint) = parse_devtools_active_port(&contents) { break endpoint; }
        }
        deadline.run(async { tokio::time::sleep(Duration::from_millis(50)).await; Ok(()) }).await?;
    };
    let browser_url = format!("ws://127.0.0.1:{port}{browser_websocket_path}");
    let page = loop {
        match CdpPage::connect_with_deadline(&browser_url, port, headless, deadline).await {
            Ok(page) => break page,
            Err(_) if deadline.remaining().is_ok() => deadline.run(async { tokio::time::sleep(Duration::from_millis(50)).await; Ok(()) }).await?,
            Err(_) => return Err("collector_deadline_exceeded".into()),
        }
    };
    Ok(CdpBrowserSession { page, process, port, browser_websocket_path, user_data_dir_argument: args[0].clone(), profile_path: profile })
}

pub async fn launch_download_browser_session(executable: &Path, profile: &Path, browser_id: BrowserId) -> Result<CdpBrowserSession, String> {
    launch_browser_session(executable, profile, browser_id, true).await
}

pub fn parse_devtools_active_port(contents: &str) -> Result<(u16, String), String> {
    let mut lines = contents.lines();
    let port = lines.next().and_then(|line| line.parse::<u16>().ok()).filter(|port| *port > 0).ok_or_else(|| "cdp_start_timeout".to_string())?;
    let path = lines.next().filter(|path| path.starts_with("/devtools/browser/")).ok_or_else(|| "cdp_start_timeout".to_string())?;
    Ok((port, path.to_string()))
}
