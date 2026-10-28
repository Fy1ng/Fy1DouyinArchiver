use chrono::Utc;
use douyin_history_archive::{
    collector::{
        archive_snapshot::{
            click_favorite_folders_tab, click_favorite_tab, read_archive_wheel_target,
            read_raw_archive_snapshot, read_remote_folder_page_snapshot, select_remote_folder,
            ArchiveSnapshotOptions,
        },
        browser_discovery::discover_system_browser,
        browser_process::BrowserProcessRecordV2,
        browser_runtime::{persist_validation_with_id, resolve_browser},
        browser_state::read_browser_selection,
        cli::{BrowserPreference, CollectorCommand, RecoverLoginOptions},
        contracts::CollectorDiagnostic,
        deadline::Deadline,
        login_actions::{click_login_action_once, process_optional_save_login, LoginAction},
        runner::run_command,
        session::{launch_browser_session, CdpBrowserSession},
        session_probe::{session_probe_from_value, SessionProbeV1, DOUYIN_HISTORY_URL},
        snapshot::read_raw_snapshot,
    },
    settings_store::ArchiveListRangeV1,
    sync::{browser_executable_path_sha256, ArchiveRunModeV1, SyncExecutionEnvelopeV1, SyncSourceV1},
    sync_orchestration::{SyncPhase, SyncPhaseEnvelopeV1},
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

const DOUYIN_HOME_URL: &str = "https://www.douyin.com/";
const SESSION_PROBE_SCRIPT: &str = include_str!("../collector-scripts/session-probe.js");
const VALIDATION_ID: &str = "11111111111111111111111111111111";
const PHASE_ATTEMPT_ID: &str = "22222222222222222222222222222222";
const REMOTE_FOLDER_RESPONSE: &str = r#"{"status_code":0,"collects_list":[{"collects_id_str":"90071992547409931","collects_name":"\u7a0d\u540e\u89c2\u770b"},{"collects_id_str":"90071992547409932","collects_name":"\u5de5\u4f5c\u8d44\u6599"}],"cursor":0,"has_more":0}"#;
const FAVORITE_FLOW_HTML: &str = r###"<!doctype html>
<html><head><title>favorite-flow</title></head><body>
<main id="favorite-view" style="height:240px;overflow-y:auto">
  <div id="favorite-tabs">
    <button id="default-tab">收藏</button>
    <button id="folders-tab">收藏夹</button>
  </div>
  <div id="folder-strip" hidden></div>
  <div id="content-list"></div>
</main>
<script>
(() => {
  const view = document.querySelector('#favorite-view');
  const folderStrip = document.querySelector('#folder-strip');
  const contentList = document.querySelector('#content-list');
  const folders = [
    { collects_id_str: '90071992547409931', collects_name: '稍后观看' },
    { collects_id_str: '90071992547409932', collects_name: '工作资料' }
  ];
  let currentFolder = 'default';
  let currentCount = 20;
  const recordInfo = (index) => ({
    aweme_id: `775${String(index).padStart(16, '0')}`,
    aweme_type: index % 2 === 0 ? 0 : 68,
    create_time: 1786420800000 - index * 1000,
    author: { nickname: `作者 ${index}` },
    desc: `收藏内容 ${index}`,
    video: { cover_url_list: [`https://example.com/cover-${index}.webp`] },
    statistics: { digg_count: 3000 + index }
  });
  const renderRecords = (showFolders = false) => {
    folderStrip.hidden = !showFolders;
    contentList.hidden = false;
    const infos = Array.from({ length: currentCount }, (_, index) => recordInfo(index));
    const list = document.createElement('div');
    list.className = 'modern-record-list';
    for (const info of infos) {
      const card = document.createElement('article');
      card.className = 'modern-record';
      card.style.height = '80px';
      const anchor = document.createElement('a');
      anchor.href = `https://www.douyin.com/${info.aweme_type === 68 ? 'note' : 'video'}/${info.aweme_id}`;
      anchor.textContent = info.desc;
      card.appendChild(anchor);
      list.appendChild(card);
    }
    const entries = infos.map((info) => ({ props: { awemeInfo: info } }));
    list.firstElementChild.__reactFiber$flowRecords = {
      memoizedProps: { aweme_list: entries }, pendingProps: null,
      alternate: null, return: null
    };
    contentList.replaceChildren(list);
  };
  const renderFolders = () => {
    folderStrip.hidden = false;
    contentList.hidden = true;
    const entries = folders.map((folder) => ({ props: { item: folder } }));
    folderStrip.replaceChildren(...folders.map((folder, index) => {
      const button = document.createElement('button');
      button.className = 'modern-folder';
      button.dataset.collectsId = folder.collects_id_str;
      button.textContent = folder.collects_name;
      button.setAttribute('aria-selected', String(index === 0));
      button.addEventListener('click', () => {
        for (const candidate of folderStrip.children) candidate.setAttribute('aria-selected', String(candidate === button));
        currentFolder = folder.collects_id_str;
        currentCount = 20;
        renderRecords(true);
      });
      return button;
    }));
    folderStrip.firstElementChild.__reactFiber$flowFolders = {
      memoizedProps: { collects_list: entries }, pendingProps: null,
      alternate: null, return: null
    };
  };
  document.querySelector('#default-tab').addEventListener('click', () => {
    currentFolder = 'default'; currentCount = 20; renderRecords(false);
  });
  document.querySelector('#folders-tab').addEventListener('click', () => {
    renderFolders();
    void fetch('/aweme/v1/web/collects/list/');
  });
  view.addEventListener('wheel', () => {
    if (currentFolder === 'default' && currentCount === 20) {
      currentCount = 40; renderRecords(false);
    }
  });
  renderRecords(false);
  globalThis.__favoriteFlowState = () => ({ currentFolder, currentCount });
})();
</script></body></html>"###;

struct FixtureServer {
    address: std::net::SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
    task: JoinHandle<()>,
}

impl FixtureServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_server = Arc::clone(&requests);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let task = tokio::spawn(async move {
            while !stop.load(Ordering::Acquire) {
                let Ok(Ok((mut stream, _))) =
                    tokio::time::timeout(Duration::from_millis(100), listener.accept()).await
                else {
                    continue;
                };
                let requests = Arc::clone(&requests_for_server);
                tokio::spawn(async move {
                    let mut request = [0_u8; 4096];
                    let size = stream.read(&mut request).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..size]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_string();
                    requests.lock().unwrap().push(path.clone());
                    let (content_type, body) = if path.contains("aweme/v1/web/collects/list") {
                        ("application/json", REMOTE_FOLDER_RESPONSE.as_bytes())
                    } else if path.contains("favorite-flow") {
                        ("text/html; charset=utf-8", FAVORITE_FLOW_HTML.as_bytes())
                    } else if path.contains("get-qrcode")
                        || path.contains("captcha/image")
                    {
                        (
                            "image/svg+xml",
                            br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="black"/></svg>"#.as_slice(),
                        )
                    } else if path.contains("play") {
                        ("video/mp4", b"fixture-media".as_slice())
                    } else {
                        (
                            "text/html; charset=utf-8",
                            br#"<!doctype html><title>rust-cdp-fixture</title><main data-value="42">ready</main>"#.as_slice(),
                        )
                    };
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                });
            }
        });
        Self {
            address,
            requests,
            stopped,
            task,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }

    fn request_paths(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    async fn stop(self) {
        self.stopped.store(true, Ordering::Release);
        self.task.await.unwrap();
    }
}

#[test]
fn network_response_metadata_routes_detail_json_without_media_body_downloads() {
    use douyin_history_archive::collector::cdp::NetworkResponse;
    use serde_json::json;

    let detail = NetworkResponse::from_event_params(&json!({
        "requestId": "detail-1", "type": "Fetch",
        "response": {"url": "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=7123456789012345678", "status": 200, "mimeType": "application/json"}
    })).expect("detail response");
    assert!(detail.is_json_detail_for("7123456789012345678"));

    let media = NetworkResponse::from_event_params(&json!({
        "requestId": "media-1", "type": "Media",
        "response": {"url": "http://127.0.0.1/media/video.mp4", "status": 200, "mimeType": "video/mp4"}
    })).expect("media response");
    assert!(!media.is_json_detail_for("7123456789012345678"));
}

#[cfg(target_os = "windows")]
fn browser_process_tree(root_pid: u32) -> HashSet<u32> {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, false);
    let root_started_at = system
        .process(Pid::from_u32(root_pid))
        .expect("browser root process must exist")
        .start_time();
    let mut pids = HashSet::from([root_pid]);
    loop {
        let previous_len = pids.len();
        for (pid, process) in system.processes() {
            if process.start_time() >= root_started_at
                && process
                    .parent()
                    .is_some_and(|parent| pids.contains(&parent.as_u32()))
            {
                pids.insert(pid.as_u32());
            }
        }
        if pids.len() == previous_len {
            return pids;
        }
    }
}

#[cfg(target_os = "windows")]
fn visible_top_level_windows_for_processes(pids: HashSet<u32>) -> Vec<(u32, isize)> {
    use windows_sys::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
        },
    };

    struct Search {
        pids: HashSet<u32>,
        windows: Vec<(u32, isize)>,
    }

    unsafe extern "system" fn visit(window: HWND, parameter: LPARAM) -> BOOL {
        let search = &mut *(parameter as *mut Search);
        let mut owner_pid = 0;
        GetWindowThreadProcessId(window, &mut owner_pid);
        if search.pids.contains(&owner_pid) && IsWindowVisible(window) != 0 {
            search.windows.push((owner_pid, window as isize));
        }
        1
    }

    let mut search = Search {
        pids,
        windows: Vec::new(),
    };
    unsafe {
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
    }
    search.windows
}

fn selected_browser() -> BrowserPreference {
    match std::env::var("EMILY_TEST_BROWSER")
        .unwrap_or_else(|_| "edge".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "edge" => BrowserPreference::Edge,
        "chrome" => BrowserPreference::Chrome,
        value => panic!("EMILY_TEST_BROWSER must be edge or chrome, got {value}"),
    }
}

fn temporary_root(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "emily-rust-cdp-{label}-{}-{suffix}",
        std::process::id()
    ))
}

async fn wait_for_path(path: &Path, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(target_os = "windows")]
async fn assert_processes_stopped(pids: &HashSet<u32>) {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::All, false);
        let running = pids
            .iter()
            .copied()
            .filter(|pid| system.process(Pid::from_u32(*pid)).is_some())
            .collect::<Vec<_>>();
        if running.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "browser processes still running: {running:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn assert_debug_port_closed(port: u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "debug port {port} remained open"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn close_session_and_assert_cleanup(session: CdpBrowserSession, account_root: &Path) {
    let port = session.port;
    let profile_path = session.profile_path.clone();
    let pids = browser_process_tree(session.process.pid);
    session
        .page
        .close_with_deadline(Deadline::after(Duration::from_secs(5)))
        .await
        .expect("close browser");
    assert!(
        session.process.wait(15_000),
        "browser process did not exit normally"
    );
    drop(session);
    assert_processes_stopped(&pids).await;
    assert_debug_port_closed(port).await;
    let preferences: Value = serde_json::from_slice(
        &std::fs::read(profile_path.join("Default").join("Preferences"))
            .expect("browser preferences"),
    )
    .expect("valid browser preferences");
    assert_eq!(
        preferences
            .pointer("/profile/exit_type")
            .and_then(Value::as_str),
        Some("Normal"),
        "Browser.close must flush the profile before the owning job is released",
    );
    std::fs::remove_dir_all(account_root).expect("profile must not remain locked");
    assert!(!account_root.exists());
}

async fn run_window_constraints(
    preference: BrowserPreference,
    headless: bool,
    server: &FixtureServer,
) {
    let browser = discover_system_browser(preference).expect("system browser");
    let account_root = temporary_root(if headless { "hidden" } else { "visible" });
    let session = launch_browser_session(
        &browser.executable_path,
        &account_root,
        browser.browser_id,
        headless,
    )
    .await
    .expect("launch CDP");
    if headless {
        assert!(session
            .process
            .desktop_name
            .as_deref()
            .is_some_and(|name| name.starts_with("EmilyCollector-")));
        tokio::time::sleep(Duration::from_millis(750)).await;
        assert_eq!(
            visible_top_level_windows_for_processes(browser_process_tree(session.process.pid)),
            Vec::<(u32, isize)>::new()
        );
    } else {
        assert!(session.process.desktop_name.is_none());
        let targets = reqwest::get(format!("http://127.0.0.1:{}/json/list", session.port))
            .await
            .expect("query visible browser targets")
            .text()
            .await
            .expect("decode visible browser targets");
        let targets: Value = serde_json::from_str(&targets).expect("parse visible browser targets");
        let page_count = targets
            .as_array()
            .expect("target list")
            .iter()
            .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
            .count();
        assert_eq!(
            page_count, 1,
            "visible login must keep exactly one controlled page target"
        );
    }
    session
        .page
        .navigate(&server.url("/"))
        .await
        .expect("navigate");
    let title: String = session
        .page
        .evaluate("document.title")
        .await
        .expect("evaluate");
    assert_eq!(title, "rust-cdp-fixture");
    close_session_and_assert_cleanup(session, &account_root).await;
}

async fn install_home_login_fixture(session: &CdpBrowserSession) {
    session
        .page
        .block_resources(false, true)
        .await
        .expect("enable login resource policy");
    session
        .page
        .navigate(DOUYIN_HOME_URL)
        .await
        .expect("navigate Douyin home");
    let location: String = session.page.evaluate("location.href").await.unwrap();
    assert!(
        location.starts_with(DOUYIN_HOME_URL),
        "Douyin home redirected outside the accepted origin: {location}"
    );
    session
        .page
        .evaluate::<Value>(
            r##"(() => {
              history.replaceState(null, "", "/");
              globalThis.SSR_RENDER_DATA = null;
              globalThis.__INITIAL_STATE__ = { user: { isLogin: false, userInfo: {} } };
              globalThis._ROUTER_DATA = null;
              globalThis.__NEXT_DATA__ = null;
              globalThis.RENDER_DATA = null;
              globalThis.__fixtureLoginClicks = 0;
              globalThis.__fixtureSaveClicks = 0;
              document.body.innerHTML = `
                <main data-emily-login-root="true" style="position:fixed;inset:0;background:white">
                  <button id="one-click" style="position:absolute;left:40px;top:40px;width:140px;height:44px">一键登录</button>
                </main>`;
              document.querySelector("#one-click").addEventListener("click", () => {
                globalThis.__fixtureLoginClicks += 1;
                history.replaceState(null, "", "/jingxuan");
                globalThis.__INITIAL_STATE__ = {
                  user: {
                    isLogin: true,
                    userInfo: {
                      uid: "fixture-uid",
                      secUid: "fixture-sec-uid",
                      nickname: "fixture-account"
                    }
                  }
                };
                document.body.innerHTML = `
                  <div class="trust-login-dialog-content" style="position:fixed;left:20px;top:20px;width:320px;height:180px;background:white">
                    <div class="trust-login-dialog-title">是否保存登录信息？（5）</div>
                    <div class="trust-login-dialog-info">下次登录更便捷，可在个人中心关闭</div>
                    <div class="trust-login-dialog-button">
                      <button class="semi-button semi-button-secondary semi-button-size-small trust-login-dialog-button-cancel" type="button" aria-disabled="false" style="width:120px;height:44px"><span>取消</span></button>
                      <button id="save" class="semi-button semi-button-primary semi-button-size-small trust-login-dialog-button-confirm" type="button" aria-disabled="false" style="width:120px;height:44px"><span>保存</span></button>
                    </div>
                  </div>`;
                const saveButton = document.querySelector("#save");
                saveButton.addEventListener("click", (event) => {
                  globalThis.__fixtureSaveClicks += 1;
                  event.currentTarget.style.pointerEvents = "none";
                  const dialog = event.currentTarget.closest('.trust-login-dialog-content');
                  setTimeout(() => dialog.remove(), 300);
                });
              });
              return { href: location.href };
            })()"##,
        )
        .await
        .expect("install home login fixture");
}

async fn verify_home_login_actions(session: &CdpBrowserSession) {
    let initial: Value = session.page.evaluate(SESSION_PROBE_SCRIPT).await.unwrap();
    assert!(matches!(
        session_probe_from_value(&initial).unwrap(),
        SessionProbeV1::Unauthenticated { .. }
    ));
    assert!(click_login_action_once(
        &session.page,
        LoginAction::OneClickLogin,
        Deadline::after(Duration::from_secs(5)),
    )
    .await
    .unwrap());
    let login_clicks: u64 = session
        .page
        .evaluate("globalThis.__fixtureLoginClicks")
        .await
        .unwrap();
    assert_eq!(login_clicks, 1);
    let authenticated: Value = session.page.evaluate(SESSION_PROBE_SCRIPT).await.unwrap();
    let SessionProbeV1::Authenticated(identity) = session_probe_from_value(&authenticated).unwrap()
    else {
        panic!("expected authenticated fixture identity");
    };
    assert_eq!(identity.uid.as_deref(), Some("fixture-uid"));
    assert_eq!(identity.sec_uid.as_deref(), Some("fixture-sec-uid"));
    assert!(process_optional_save_login(
        &session.page,
        Duration::from_secs(2),
        Deadline::after(Duration::from_secs(5)),
    )
    .await
    .unwrap());
    let save_clicks: u64 = session
        .page
        .evaluate("globalThis.__fixtureSaveClicks")
        .await
        .unwrap();
    assert_eq!(save_clicks, 1);
}

async fn verify_identity_conflict(session: &CdpBrowserSession) {
    session
        .page
        .evaluate::<Value>(
            r#"(() => {
              globalThis.SSR_RENDER_DATA = {
                app: { user: { isLogin: true, info: { uid: "fixture-left" } } }
              };
              globalThis.__INITIAL_STATE__ = {
                user: { isLogin: true, userInfo: { uid: "fixture-right" } }
              };
              return true;
            })()"#,
        )
        .await
        .unwrap();
    let conflicting: Value = session.page.evaluate(SESSION_PROBE_SCRIPT).await.unwrap();
    assert!(matches!(
        session_probe_from_value(&conflicting).unwrap(),
        SessionProbeV1::Conflicting { .. }
    ));
}

async fn verify_login_resource_pass_through(session: &CdpBrowserSession, server: &FixtureServer) {
    session
        .page
        .navigate(&server.url("/resources"))
        .await
        .unwrap();
    let before = session.page.fetch_stats();
    let script = format!(
        r#"(async () => {{
          const urls = [
            "{}",
            "{}"
          ];
          const loaded = await Promise.all(urls.map((src) => new Promise((resolve) => {{
            const image = new Image();
            image.onload = () => resolve(true);
            image.onerror = () => resolve(false);
            image.src = src;
            document.body.appendChild(image);
          }})));
          const video = document.createElement("video");
          video.src = "{}";
          document.body.appendChild(video);
          video.load();
          await new Promise((resolve) => setTimeout(resolve, 500));
          return loaded;
        }})()"#,
        server.url("/aweme/v1/web/get-qrcode/"),
        server.url("/captcha/image"),
        server.url("/aweme/v1/play/")
    );
    let loaded: Vec<bool> = session.page.evaluate(&script).await.unwrap();
    assert_eq!(loaded, vec![true, true]);
    let after = session.page.fetch_stats();
    assert!(after.paused > before.paused, "media request was not paused");
    assert_eq!(after.paused, after.decided);
    assert_eq!(after.duplicates, 0);
    assert_eq!(after.pending, 0);
    let paths = server.request_paths();
    assert!(paths.iter().any(|path| path.contains("get-qrcode")));
    assert!(paths.iter().any(|path| path.contains("captcha/image")));
    assert!(!paths.iter().any(|path| path.contains("aweme/v1/play")));
}

async fn install_archive_snapshot_fixture(session: &CdpBrowserSession) {
    session
        .page
        .call_function::<Value>(
            r#"function () {
              document.body.innerHTML = `
                <nav data-e2e="douyin-navigation" style="height:120px;overflow-y:scroll">
                  <div style="height:5000px"></div>
                </nav>
                <nav data-e2e="favorite-collection-list"></nav>
                <main class="parent-route-container route-scroll-container" style="height:240px;overflow-y:auto">
                  <div data-e2e="user-favorite-list">
                    <ul data-e2e="scroll-list"></ul>
                  </div>
                </main>`;
              const list = document.querySelector('ul[data-e2e="scroll-list"]');
              const archiveRoot = list.parentElement;
              const folders = [
                { collectionId: "folder-a", name: "收藏夹 A" },
                { collectionId: "folder-b", name: "收藏夹 B" }
              ];
              const folderRoot = document.querySelector('[data-e2e="favorite-collection-list"]');
              for (const [index, folder] of folders.entries()) {
                const button = document.createElement("button");
                button.dataset.e2e = "favorite-collection-item";
                button.dataset.collectionId = folder.collectionId;
                button.textContent = folder.name;
                button.setAttribute("aria-selected", String(index === 0));
                button.addEventListener("click", () => {
                  for (const candidate of folderRoot.children) {
                    candidate.setAttribute("aria-selected", String(candidate === button));
                  }
                });
                folderRoot.appendChild(button);
              }
              const folderElements = folders.map((folder) => ({ props: { folderInfo: folder } }));
              folderRoot.firstElementChild.__reactFiber$archiveFolders = {
                memoizedProps: { folders: folderElements },
                pendingProps: null,
                alternate: null,
                return: null
              };
              globalThis.__setArchiveSource = (source) => {
                archiveRoot.dataset.e2e = source === "likes" ? "user-like-list" : "user-favorite-list";
              };
              globalThis.__setArchiveCount = (count, sentinelText = "", articleIndex = -1) => {
                const infos = Array.from({ length: count }, (_, index) => {
                  const awemeId = `765${String(index).padStart(16, "0")}`;
                  return {
                    awemeId,
                    awemeType: index === articleIndex ? 163 : index % 2 === 0 ? 0 : 68,
                    createTime: 1786420800000 - index * 1000,
                    authorInfo: { nickname: `作者 ${index}` },
                    itemTitle: `标题 ${index}`,
                    desc: `描述 ${index}`,
                    video: { coverUrlList: [`https://example.com/${awemeId}.webp`] },
                    stats: { diggCount: 1000 + index }
                  };
                });
                list.replaceChildren(...infos.map((info) => {
                  const card = document.createElement("li");
                  card.className = "archive-card";
                  card.style.height = "80px";
                  const anchor = document.createElement("a");
                  const path = info.awemeType === 163 ? "article" : info.awemeType === 68 ? "note" : "video";
                  anchor.href = `https://www.douyin.com/${path}/${info.awemeId}`;
                  anchor.textContent = info.itemTitle;
                  card.appendChild(anchor);
                  return card;
                }));
                if (sentinelText) {
                  const sentinel = document.createElement("li");
                  sentinel.dataset.fixtureSentinel = "true";
                  sentinel.textContent = sentinelText;
                  sentinel.style.height = "40px";
                  list.appendChild(sentinel);
                }
                const sparse = infos.map((info) => ({ props: { itemInfo: { awemeId: info.awemeId } } }));
                const complete = infos.map((info) => ({ props: { itemInfo: info } }));
                list.firstElementChild.__reactFiber$archiveItems = {
                  memoizedProps: { items: sparse },
                  pendingProps: null,
                  alternate: {
                    memoizedProps: { items: complete },
                    pendingProps: null
                  },
                  return: null
                };
              };
              globalThis.__setArchiveCount(20);
              return true;
            }"#,
            Value::Null,
        )
        .await
        .expect("install archive snapshot fixture");
}

async fn verify_archive_snapshot_growth(session: &CdpBrowserSession) {
    install_archive_snapshot_fixture(session).await;
    let options = ArchiveSnapshotOptions {
        source: SyncSourceV1::Favorites,
        discover_remote_folders: true,
        requested_remote_folder_id: None,
    };
    for count in [20_usize, 40, 60] {
        session
            .page
            .evaluate::<Value>(&format!("globalThis.__setArchiveCount({count})"))
            .await
            .unwrap();
        let snapshot = read_raw_archive_snapshot(&session.page, &options)
            .await
            .expect("archive snapshot");
        assert_eq!(snapshot.items.len(), count);
        assert_eq!(snapshot.dom_card_count, count as i64);
        assert_eq!(snapshot.items[0].source_position, 0);
        assert_eq!(snapshot.items[count - 1].source_position, count as i64 - 1);
        assert_eq!(snapshot.items[0].author_name, Some(json!("作者 0")));
        assert_eq!(snapshot.items[0].item_title, Some(json!("标题 0")));
        assert_eq!(snapshot.items[0].like_count, Some(json!(1000)));
        assert_eq!(snapshot.remote_folders.len(), 2);
        assert_eq!(snapshot.dom_folder_count, 2);
        assert_eq!(snapshot.active_remote_folder_id, Some(json!("folder-a")));
        assert_eq!(snapshot.active_remote_folder_name, Some(json!("收藏夹 A")));
        let route_scroll_height: f64 = session
            .page
            .evaluate("document.querySelector('.route-scroll-container').scrollHeight")
            .await
            .unwrap();
        assert_eq!(snapshot.scroll_height, route_scroll_height);
    }

    session
        .page
        .evaluate::<Value>("globalThis.__setArchiveCount(20, '加载中')")
        .await
        .unwrap();
    let loading = read_raw_archive_snapshot(&session.page, &options)
        .await
        .expect("archive snapshot ignores loading sentinel");
    assert_eq!(loading.items.len(), 20);
    assert_eq!(loading.dom_card_count, 20);
    assert!(loading.loading_visible);

    session
        .page
        .evaluate::<Value>("globalThis.__setArchiveCount(20, '暂时没有更多了')")
        .await
        .unwrap();
    let no_more = read_raw_archive_snapshot(&session.page, &options)
        .await
        .expect("archive snapshot ignores no-more sentinel");
    assert_eq!(no_more.items.len(), 20);
    assert!(no_more.no_more_visible);

    session
        .page
        .evaluate::<Value>("globalThis.__setArchiveCount(20, '', 4)")
        .await
        .unwrap();
    let with_article = read_raw_archive_snapshot(&session.page, &options)
        .await
        .expect("archive snapshot includes article cards");
    assert_eq!(with_article.items.len(), 20);
    assert_eq!(
        with_article.items[4].href,
        "https://www.douyin.com/article/7650000000000000004"
    );
    let article_scroll: Value = session
        .page
        .call_function(
            include_str!("../collector-scripts/continuous-scroll.js"),
            json!({
                "startRecordCount": 19,
                "source": "favorites",
                "scrollSpeed": 900,
                "maxDurationMs": 100
            }),
        )
        .await
        .expect("article counts as a favorite record");
    assert_eq!(article_scroll["recordCount"], json!(20));

    session
        .page
        .evaluate::<Value>("globalThis.__setArchiveCount(20)")
        .await
        .unwrap();
    session
        .page
        .evaluate::<Value>("setTimeout(() => globalThis.__setArchiveCount(40), 100)")
        .await
        .unwrap();
    let scroll: Value = session
        .page
        .call_function(
            include_str!("../collector-scripts/continuous-scroll.js"),
            json!({
                "startRecordCount": 20,
                "recordSelector": "ul[data-e2e=\"scroll-list\"] > li.archive-card",
                "source": "favorites",
                "scrollSpeed": 900,
                "maxDurationMs": 2_000
            }),
        )
        .await
        .expect("archive continuous scroll");
    assert_eq!(scroll["reason"], json!("list-growth"));
    assert_eq!(scroll["recordCount"], json!(40));

    session
        .page
        .evaluate::<Value>(
            r#"(() => {
              globalThis.__setArchiveCount(20);
              const route = document.querySelector('.route-scroll-container');
              const navigation = document.querySelector('[data-e2e="douyin-navigation"]');
              route.scrollTop = 0;
              navigation.scrollTop = 0;
              return true;
            })()"#,
        )
        .await
        .unwrap();
    let _: Value = session
        .page
        .call_function(
            include_str!("../collector-scripts/continuous-scroll.js"),
            json!({
                "startRecordCount": 20,
                "source": "favorites",
                "scrollSpeed": 900,
                "maxDurationMs": 350
            }),
        )
        .await
        .expect("favorite source-local scroll");
    let favorite_scroll: Value = session
        .page
        .evaluate(
            r#"({
              route: document.querySelector('.route-scroll-container').scrollTop,
              navigation: document.querySelector('[data-e2e="douyin-navigation"]').scrollTop
            })"#,
        )
        .await
        .unwrap();
    assert!(favorite_scroll["route"].as_f64().unwrap_or(0.0) > 0.0);
    assert_eq!(favorite_scroll["navigation"], json!(0));

    session
        .page
        .evaluate::<Value>(
            r#"(() => {
              globalThis.__setArchiveSource('likes');
              const route = document.querySelector('.route-scroll-container');
              const navigation = document.querySelector('[data-e2e="douyin-navigation"]');
              route.scrollTop = 0;
              navigation.scrollTop = 0;
              return true;
            })()"#,
        )
        .await
        .unwrap();
    let likes_options = ArchiveSnapshotOptions {
        source: SyncSourceV1::Likes,
        discover_remote_folders: false,
        requested_remote_folder_id: None,
    };
    let likes_snapshot = read_raw_archive_snapshot(&session.page, &likes_options)
        .await
        .expect("likes source-local snapshot");
    assert_eq!(likes_snapshot.items.len(), 20);
    let _: Value = session
        .page
        .call_function(
            include_str!("../collector-scripts/continuous-scroll.js"),
            json!({
                "startRecordCount": 20,
                "source": "likes",
                "scrollSpeed": 900,
                "maxDurationMs": 350
            }),
        )
        .await
        .expect("likes source-local scroll");
    let likes_scroll: Value = session
        .page
        .evaluate(
            r#"({
              route: document.querySelector('.route-scroll-container').scrollTop,
              navigation: document.querySelector('[data-e2e="douyin-navigation"]').scrollTop
            })"#,
        )
        .await
        .unwrap();
    assert!(likes_scroll["route"].as_f64().unwrap_or(0.0) > 0.0);
    assert_eq!(likes_scroll["navigation"], json!(0));

    session
        .page
        .evaluate::<Value>("globalThis.__setArchiveSource('favorites')")
        .await
        .unwrap();

    select_remote_folder(&session.page, "folder-b")
        .await
        .expect("select remote folder");
    let selected = read_raw_archive_snapshot(
        &session.page,
        &ArchiveSnapshotOptions {
            source: SyncSourceV1::RemoteFolder,
            discover_remote_folders: false,
            requested_remote_folder_id: Some("folder-b".to_string()),
        },
    )
    .await
    .expect("selected folder snapshot");
    assert_eq!(selected.active_remote_folder_id, Some(json!("folder-b")));
    assert_eq!(selected.active_remote_folder_name, Some(json!("收藏夹 B")));
}

async fn install_modern_favorite_fixture(session: &CdpBrowserSession) {
    session
        .page
        .call_function::<Value>(
            r#"function () {
              document.body.innerHTML = `
                <main id="favorite-view" style="height:240px;overflow-y:auto">
                  <div id="favorite-tabs">
                    <button id="default-tab">收藏</button>
                    <button id="folders-tab">收藏夹</button>
                  </div>
                  <div id="folder-strip"></div>
                  <div id="content-list"></div>
                </main>`;
              const view = document.querySelector('#favorite-view');
              const folderStrip = document.querySelector('#folder-strip');
              const contentList = document.querySelector('#content-list');
              const folders = [
                { collects_id_str: "90071992547409931", collects_name: "稍后观看" },
                { collects_id_str: "90071992547409932", collects_name: "工作资料" }
              ];
              let currentFolder = "default";
              let currentCount = 20;
              const recordInfo = (index) => ({
                aweme_id: `775${String(index).padStart(16, "0")}`,
                aweme_type: index % 2 === 0 ? 0 : 68,
                create_time: 1786420800000 - index * 1000,
                author: { nickname: `作者 ${index}` },
                desc: `收藏内容 ${index}`,
                video: { cover_url_list: [`https://example.com/cover-${index}.webp`] },
                statistics: { digg_count: 3000 + index }
              });
              const renderRecords = (showFolders = false) => {
                folderStrip.hidden = !showFolders;
                contentList.hidden = false;
                const infos = Array.from({ length: currentCount }, (_, index) => recordInfo(index));
                const list = document.createElement('div');
                list.className = 'modern-record-list';
                for (const info of infos) {
                  const card = document.createElement('article');
                  card.className = 'modern-record';
                  card.style.height = '80px';
                  const anchor = document.createElement('a');
                  anchor.href = `https://www.douyin.com/${info.aweme_type === 68 ? "note" : "video"}/${info.aweme_id}`;
                  anchor.textContent = info.desc;
                  card.appendChild(anchor);
                  list.appendChild(card);
                }
                const complete = infos.map((info) => ({ props: { awemeInfo: info } }));
                list.firstElementChild.__reactFiber$modernRecords = {
                  memoizedProps: { aweme_list: complete },
                  pendingProps: null,
                  alternate: { memoizedProps: { aweme_list: complete }, pendingProps: null },
                  return: null
                };
                contentList.replaceChildren(list);
              };
              const renderFolders = () => {
                folderStrip.hidden = false;
                contentList.hidden = true;
                const entries = folders.map((folder) => ({ props: { item: folder } }));
                folderStrip.replaceChildren(...folders.map((folder, index) => {
                  const button = document.createElement('button');
                  button.className = 'modern-folder';
                  button.dataset.collectsId = folder.collects_id_str;
                  button.textContent = folder.collects_name;
                  button.setAttribute('aria-selected', String(index === 0));
                  button.addEventListener('click', () => {
                    for (const candidate of folderStrip.children) candidate.setAttribute('aria-selected', String(candidate === button));
                    currentFolder = folder.collects_id_str;
                    currentCount = 20;
                    renderRecords(true);
                  });
                  return button;
                }));
                folderStrip.firstElementChild.__reactFiber$modernFolders = {
                  memoizedProps: { collects_list: entries },
                  pendingProps: null,
                  alternate: null,
                  return: null
                };
              };
              document.querySelector('#default-tab').addEventListener('click', () => {
                currentFolder = 'default';
                currentCount = 20;
                renderRecords();
              });
              document.querySelector('#folders-tab').addEventListener('click', renderFolders);
              view.addEventListener('wheel', () => {
                if (currentFolder === 'default' && currentCount === 20) {
                  currentCount = 40;
                  renderRecords();
                }
              });
              globalThis.__modernFavoriteState = () => ({ currentFolder, currentCount });
              renderRecords();
              return true;
            }"#,
            Value::Null,
        )
        .await
        .expect("install modern favorite fixture");
}

async fn verify_delayed_favorite_mount_and_hot_region_guard(session: &CdpBrowserSession) {
    session
        .page
        .call_function::<Value>(
            r#"function () {
              document.body.innerHTML = `
                <main id="delayed-favorite" style="height:240px;overflow-y:auto">
                  <div id="loading">加载中</div>
                  <section class="hot-list">
                    ${Array.from({ length: 10 }, (_, index) => `<a href="https://www.douyin.com/video/799${String(index).padStart(16, "0")}">热门 ${index}</a>`).join("")}
                  </section>
                  <div id="real-list"></div>
                </main>`;
              const root = document.querySelector('#delayed-favorite');
              const mount = () => {
                if (document.querySelector('.delayed-real-list')) return;
                const list = document.createElement('div');
                list.className = 'delayed-real-list';
                const infos = Array.from({ length: 2 }, (_, index) => ({
                  awemeId: `778${String(index).padStart(16, "0")}`,
                  awemeType: 0,
                  createTime: 1786420800000 - index * 1000,
                  authorInfo: { nickname: `作者 ${index}` },
                  itemTitle: `真实收藏 ${index}`,
                  stats: { diggCount: 1 }
                }));
                for (const info of infos) {
                  const card = document.createElement('article');
                  const anchor = document.createElement('a');
                  anchor.href = `https://www.douyin.com/video/${info.awemeId}`;
                  anchor.textContent = info.itemTitle;
                  card.appendChild(anchor);
                  list.appendChild(card);
                }
                list.firstElementChild.__reactFiber$delayed = {
                  memoizedProps: { items: infos.map((info) => ({ props: { itemInfo: info } })) },
                  pendingProps: null, alternate: null, return: null
                };
                document.querySelector('#loading').remove();
                document.querySelector('#real-list').replaceChildren(list);
              };
              root.addEventListener('wheel', mount, { once: true });
              return true;
            }"#,
            Value::Null,
        )
        .await
        .expect("install delayed favorite fixture");
    let initial = read_raw_archive_snapshot(
        &session.page,
        &ArchiveSnapshotOptions {
            source: SyncSourceV1::Favorites,
            discover_remote_folders: false,
            requested_remote_folder_id: None,
        },
    )
    .await;
    assert_eq!(initial.unwrap_err(), "archive_list_not_found");
    let target = read_archive_wheel_target(&session.page, SyncSourceV1::Favorites)
        .await
        .expect("delayed favorite wheel target");
    assert!(!target.list_found);
    session
        .page
        .dispatch_mouse_wheel(target.x, target.y, 120.0)
        .await
        .expect("wake delayed favorite list");
    session.page.wait_for_timeout(50).await;
    let mounted = read_raw_archive_snapshot(
        &session.page,
        &ArchiveSnapshotOptions {
            source: SyncSourceV1::Favorites,
            discover_remote_folders: false,
            requested_remote_folder_id: None,
        },
    )
    .await
    .expect("mounted favorite snapshot");
    assert_eq!(mounted.items.len(), 2);
    assert_eq!(mounted.items[0].aweme_id, json!("7780000000000000000"));
}

async fn verify_modern_favorite_snapshot_and_selection(session: &CdpBrowserSession) {
    install_modern_favorite_fixture(session).await;
    click_favorite_tab(&session.page)
        .await
        .expect("activate default favorites tab");
    let options = ArchiveSnapshotOptions {
        source: SyncSourceV1::Favorites,
        discover_remote_folders: true,
        requested_remote_folder_id: None,
    };
    let snapshot = read_raw_archive_snapshot(&session.page, &options)
        .await
        .expect("modern favorite snapshot");
    assert_eq!(snapshot.items.len(), 20);
    assert!(snapshot.remote_folders.is_empty());

    let wheel_target = read_archive_wheel_target(&session.page, SyncSourceV1::Favorites)
        .await
        .expect("modern favorite wheel target");
    assert!(wheel_target.list_found);
    assert!(wheel_target.can_scroll);
    session
        .page
        .dispatch_mouse_wheel(wheel_target.x, wheel_target.y, 1_200.0)
        .await
        .expect("dispatch modern favorite wheel");
    session.page.wait_for_timeout(100).await;
    let after_wheel = read_archive_wheel_target(&session.page, SyncSourceV1::Favorites)
        .await
        .expect("modern favorite wheel growth");
    assert_eq!(after_wheel.record_count, 40);

    click_favorite_folders_tab(&session.page)
        .await
        .expect("activate favorite folders tab");
    let folder_page = read_remote_folder_page_snapshot(&session.page)
        .await
        .expect("read remote folder page");
    assert_eq!(folder_page.folders.len(), 2);
    assert_eq!(
        folder_page.folders[1].remote_folder_id,
        json!("90071992547409932")
    );

    select_remote_folder(&session.page, "90071992547409932")
        .await
        .expect("select modern named folder");
    let state: Value = session
        .page
        .evaluate("globalThis.__modernFavoriteState()")
        .await
        .unwrap();
    assert_eq!(state["currentFolder"], json!("90071992547409932"));

}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires selected system Edge or Chrome"]
async fn selected_browser_runs_the_favorite_collection_flow_end_to_end() {
    let preference = selected_browser();
    let server = FixtureServer::start().await;
    let browser = discover_system_browser(preference).expect("system browser");
    let account_root = temporary_root("favorite-runner");
    let choice = resolve_browser(&account_root, preference, false).expect("browser choice");
    persist_validation_with_id(
        &account_root,
        &choice,
        DOUYIN_HISTORY_URL,
        Some(VALIDATION_ID),
    )
    .expect("persist validation");
    let selection = read_browser_selection(&account_root)
        .expect("read selection")
        .expect("selection");
    let profile_generation = selection.profile_generation.expect("profile generation");
    let task_root = account_root.join("task");
    std::fs::create_dir_all(&task_root).unwrap();
    let options = douyin_history_archive::collector::cli::CollectOptions {
        account_id: 11,
        sync_job_id: 701,
        source: SyncSourceV1::Favorites,
        source_instance_key: String::new(),
        task_payload_sha256: "a".repeat(64),
        frozen_at_ms: 1_786_420_800_000,
        cutoff_ms: None,
        phase: Some(SyncPhase::Collecting),
        phase_attempt_id: Some(PHASE_ATTEMPT_ID.to_string()),
        result_path: task_root.join("result.json"),
        progress_path: task_root.join("progress.json"),
        diagnostic_path: task_root.join("diagnostic.json"),
        cancel_file: task_root.join("cancel"),
        account_profile_root: account_root.clone(),
        browser_pid_file: task_root.join("browser.json"),
        collector_identity_path: Some(task_root.join("collector.json")),
        known_ids_path: None,
        browser: preference,
        profile_key: "favorite-runner".to_string(),
        profile_generation,
        browser_executable_sha256: browser_executable_path_sha256(
            &browser.executable_path.to_string_lossy(),
        ),
        page_url: server.url("/favorite-flow"),
        range: douyin_history_archive::collector::cli::CollectRange::Archive(
            ArchiveListRangeV1::RecentItems { count: 40 },
        ),
        run_mode: Some(ArchiveRunModeV1::Incremental),
        discover_remote_folders: true,
        scroll_speed: 6_000,
        headless: true,
        block_media: true,
        block_images: true,
        hard_timeout_ms: 60_000,
        max_records: 50_000,
    };
    run_command(CollectorCommand::Collect(options.clone()))
        .await
        .expect("favorite collection runner");
    let envelope: SyncExecutionEnvelopeV1 =
        serde_json::from_slice(&std::fs::read(&options.result_path).unwrap()).unwrap();
    let douyin_history_archive::sync::SourceExecutionResultV1::Favorites(result) = envelope.result
    else {
        panic!("favorites result");
    };
    assert_eq!(result.archive.records.len(), 40);
    let douyin_history_archive::sync::RemoteFolderDiscoveryResultV1::Success(discovery) =
        result.remote_folder_discovery
    else {
        panic!("folder discovery result");
    };
    assert_eq!(discovery.folders.len(), 2);
    assert!(discovery.complete_scan);
    assert!(discovery.bottom_confirmed);
    assert!(!options.diagnostic_path.exists());
    assert!(server
        .request_paths()
        .iter()
        .any(|path| path.contains("aweme/v1/web/collects/list")));
    server.stop().await;
    std::fs::remove_dir_all(account_root).expect("cleanup runner fixture");
}

async fn verify_history_uses_its_route_scroll_container(session: &CdpBrowserSession) {
    session
        .page
        .call_function::<Value>(
            r#"function () {
              document.body.innerHTML = `
                <nav data-e2e="douyin-navigation" style="height:120px;overflow-y:scroll">
                  <div style="height:5000px"></div>
                </nav>
                <main class="route-scroll-container" style="height:240px;overflow-y:auto">
                  <ul data-e2e="scroll-list"></ul>
                </main>`;
              const list = document.querySelector('ul[data-e2e="scroll-list"]');
              const infos = Array.from({ length: 20 }, (_, index) => ({
                awemeId: `865${String(index).padStart(16, "0")}`,
                awemeType: 0,
                historyTime: 1786420800000 - index * 1000,
                authorInfo: { nickname: `历史作者 ${index}` },
                itemTitle: `历史标题 ${index}`,
                video: { coverUrlList: [`https://example.com/history-${index}.webp`] },
                stats: { diggCount: 2000 + index }
              }));
              for (const info of infos) {
                const card = document.createElement("li");
                card.className = "record_card";
                card.style.height = "80px";
                const anchor = document.createElement("a");
                anchor.href = `https://www.douyin.com/video/${info.awemeId}`;
                card.appendChild(anchor);
                list.appendChild(card);
              }
              list.firstElementChild.__reactFiber$historyItems = {
                memoizedProps: infos.map((info) => ({ props: { itemInfo: info } })),
                pendingProps: null,
                alternate: null,
                return: null
              };
              return true;
            }"#,
            Value::Null,
        )
        .await
        .expect("install history snapshot fixture");

    let snapshot = read_raw_snapshot(&session.page)
        .await
        .expect("history snapshot");
    let route_scroll_height: f64 = session
        .page
        .evaluate("document.querySelector('.route-scroll-container').scrollHeight")
        .await
        .unwrap();
    assert_eq!(snapshot.items.len(), 20);
    assert_eq!(snapshot.scroll_height, route_scroll_height);

    let _: Value = session
        .page
        .call_function(
            include_str!("../collector-scripts/continuous-scroll.js"),
            json!({
                "startRecordCount": 20,
                "source": "history",
                "scrollSpeed": 900,
                "maxDurationMs": 350
            }),
        )
        .await
        .expect("history source-local scroll");
    let positions: Value = session
        .page
        .evaluate(
            r#"({
              route: document.querySelector('.route-scroll-container').scrollTop,
              navigation: document.querySelector('[data-e2e="douyin-navigation"]').scrollTop
            })"#,
        )
        .await
        .unwrap();
    assert!(positions["route"].as_f64().unwrap_or(0.0) > 0.0);
    assert_eq!(positions["navigation"], json!(0));
}

async fn verify_recovery_cancellation(preference: BrowserPreference, server: &FixtureServer) {
    let account_root = temporary_root("recover-cancel");
    let choice = resolve_browser(&account_root, preference, false).unwrap();
    persist_validation_with_id(
        &account_root,
        &choice,
        DOUYIN_HISTORY_URL,
        Some(VALIDATION_ID),
    )
    .unwrap();
    let task_root = account_root.join("task");
    std::fs::create_dir_all(&task_root).unwrap();
    let result_path = task_root.join("result.json");
    let progress_path = task_root.join("progress.json");
    let diagnostic_path = task_root.join("diagnostic.json");
    let cancel_file = task_root.join("cancel");
    let browser_pid_file = task_root.join("browser.json");
    let options = RecoverLoginOptions {
        account_id: 11,
        sync_job_id: 23,
        phase: SyncPhase::RecoveringLogin,
        phase_attempt_id: PHASE_ATTEMPT_ID.to_string(),
        result_path: result_path.clone(),
        progress_path,
        diagnostic_path: diagnostic_path.clone(),
        cancel_file: cancel_file.clone(),
        account_profile_root: account_root.clone(),
        browser_pid_file: browser_pid_file.clone(),
        collector_identity_path: task_root.join("collector.json"),
        browser: preference,
        home_url: server.url("/recovery"),
        deadline_at_ms: Utc::now().timestamp_millis() + 15_000,
        headless: true,
    };
    let pid_path_for_cancel = browser_pid_file.clone();
    let cancel_path_for_task = cancel_file.clone();
    let cancellation = tokio::spawn(async move {
        wait_for_path(&pid_path_for_cancel, Duration::from_secs(10)).await;
        let browser_record: SyncPhaseEnvelopeV1<BrowserProcessRecordV2> =
            serde_json::from_slice(&std::fs::read(&pid_path_for_cancel).unwrap()).unwrap();
        let pids = browser_process_tree(browser_record.payload.pid);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            visible_top_level_windows_for_processes(pids.clone()),
            Vec::<(u32, isize)>::new()
        );
        std::fs::write(&cancel_path_for_task, b"cancel").unwrap();
        (browser_record, pids)
    });
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        run_command(CollectorCommand::RecoverLogin(options)),
    )
    .await
    .expect("recovery cancellation timed out");
    let (browser_record, pids) = cancellation.await.unwrap();
    assert_eq!(outcome, Err("collector_cancelled".to_string()));
    assert!(
        cancel_file.exists(),
        "collector must preserve task cancellation"
    );
    assert!(!result_path.exists());
    let diagnostic: SyncPhaseEnvelopeV1<CollectorDiagnostic> =
        serde_json::from_slice(&std::fs::read(&diagnostic_path).unwrap()).unwrap();
    assert_eq!(diagnostic.payload.status, "cancelled");
    assert_eq!(diagnostic.payload.error_code, "collector_cancelled");
    assert_processes_stopped(&pids).await;
    assert_debug_port_closed(browser_record.payload.devtools_port).await;
    std::fs::remove_dir_all(&account_root).expect("cancelled profile must not remain locked");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires selected system Edge or Chrome"]
async fn selected_browser_visible_and_hidden_constraints() {
    let preference = selected_browser();
    let server = FixtureServer::start().await;
    run_window_constraints(preference, false, &server).await;
    run_window_constraints(preference, true, &server).await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires selected system Edge or Chrome and Douyin home access"]
async fn selected_browser_home_login_recovery_and_fetch_terminals() {
    let preference = selected_browser();
    let server = FixtureServer::start().await;
    let browser = discover_system_browser(preference).expect("system browser");
    let account_root = temporary_root("home-login");
    let session = launch_browser_session(
        &browser.executable_path,
        &account_root,
        browser.browser_id,
        false,
    )
    .await
    .expect("launch visible login browser");
    install_home_login_fixture(&session).await;
    verify_home_login_actions(&session).await;
    verify_identity_conflict(&session).await;
    close_session_and_assert_cleanup(session, &account_root).await;

    let resource_root = temporary_root("login-resources");
    let resource_session = launch_browser_session(
        &browser.executable_path,
        &resource_root,
        browser.browser_id,
        true,
    )
    .await
    .expect("launch hidden resource browser");
    resource_session
        .page
        .block_resources(false, true)
        .await
        .unwrap();
    verify_login_resource_pass_through(&resource_session, &server).await;
    close_session_and_assert_cleanup(resource_session, &resource_root).await;
    verify_recovery_cancellation(preference, &server).await;
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires selected system Edge or Chrome"]
async fn selected_browser_reads_archive_react_snapshots() {
    let preference = selected_browser();
    let server = FixtureServer::start().await;
    let browser = discover_system_browser(preference).expect("system browser");
    let account_root = temporary_root("archive-snapshot");
    let session = launch_browser_session(
        &browser.executable_path,
        &account_root,
        browser.browser_id,
        true,
    )
    .await
    .expect("launch hidden archive browser");
    session.page.navigate(&server.url("/")).await.unwrap();
    verify_delayed_favorite_mount_and_hot_region_guard(&session).await;
    verify_archive_snapshot_growth(&session).await;
    verify_modern_favorite_snapshot_and_selection(&session).await;
    verify_history_uses_its_route_scroll_container(&session).await;
    close_session_and_assert_cleanup(session, &account_root).await;
    server.stop().await;
}
