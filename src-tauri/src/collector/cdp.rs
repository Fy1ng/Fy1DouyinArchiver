use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{collections::{HashMap, HashSet}, sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Arc}, time::Duration};
use tokio::{sync::{mpsc, oneshot, watch, Mutex}, time::timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use super::deadline::Deadline;

#[derive(Debug, Clone, PartialEq)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkResponse {
    pub request_id: String,
    pub resource_type: String,
    pub url: String,
    pub status: u16,
    pub mime_type: String,
}

impl NetworkResponse {
    pub fn from_event_params(params: &Value) -> Option<Self> {
        let response = params.get("response")?;
        Some(Self {
            request_id: params.get("requestId")?.as_str()?.to_string(),
            resource_type: params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            url: response.get("url")?.as_str()?.to_string(),
            status: response
                .get("status")?
                .as_f64()
                .filter(|value| *value >= 0.0 && *value <= u16::MAX as f64)?
                as u16,
            mime_type: response
                .get("mimeType")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }

    pub fn is_json_detail_for(&self, aweme_id: &str) -> bool {
        if self.status != 200
            || !(self.mime_type.eq_ignore_ascii_case("application/json")
                || self.mime_type.ends_with("+json"))
        {
            return false;
        }
        let Ok(url) = url::Url::parse(&self.url) else {
            return false;
        };
        url.path() == "/aweme/v1/web/aweme/detail/"
            && url
                .query_pairs()
                .any(|(key, value)| key == "aweme_id" && value == aweme_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdpCloseReason {
    PageSocket,
    BrowserSocket,
    TargetDestroyed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchStats {
    pub paused: u64,
    pub decided: u64,
    pub duplicates: u64,
    pub pending: u64,
}

#[derive(Debug, Default)]
struct FetchCounters {
    paused: AtomicU64,
    decided: AtomicU64,
    duplicates: AtomicU64,
    pending: AtomicU64,
}

impl FetchCounters {
    fn snapshot(&self) -> FetchStats {
        FetchStats {
            paused: self.paused.load(Ordering::Acquire),
            decided: self.decided.load(Ordering::Acquire),
            duplicates: self.duplicates.load(Ordering::Acquire),
            pending: self.pending.load(Ordering::Acquire),
        }
    }
}

#[derive(Debug)]
pub struct CdpConnection {
    sender: mpsc::UnboundedSender<Message>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    next_id: AtomicU64,
    events: Arc<Mutex<mpsc::UnboundedReceiver<CdpEvent>>>,
    network_responses: Arc<Mutex<mpsc::UnboundedReceiver<NetworkResponse>>>,
    block_resources: Arc<AtomicBool>,
    fetch_counters: Arc<FetchCounters>,
    closed: watch::Receiver<Option<CdpCloseReason>>,
    target_watchers: Arc<Mutex<HashMap<String, watch::Sender<Option<CdpCloseReason>>>>>,
}

impl CdpConnection {
    pub async fn connect(url: &str) -> Result<Self, String> {
        Self::connect_with_deadline(url, CdpCloseReason::BrowserSocket, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn connect_with_close_reason(
        url: &str,
        close_reason: CdpCloseReason,
    ) -> Result<Self, String> {
        Self::connect_with_deadline(url, close_reason, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn connect_with_deadline(url: &str, close_reason: CdpCloseReason, deadline: Deadline) -> Result<Self, String> {
        let (socket, _) = deadline.run(async {
            connect_async(url).await.map_err(|_| "cdp_connection_failed".to_string())
        }).await?;
        let (mut writer, mut reader) = socket.split();
        let (sender, mut outgoing) = mpsc::unbounded_channel::<Message>();
        let (event_sender, event_receiver) = mpsc::unbounded_channel::<CdpEvent>();
        let (network_sender, network_receiver) = mpsc::unbounded_channel::<NetworkResponse>();
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> = Arc::new(Mutex::new(HashMap::new()));
        let block_resources = Arc::new(AtomicBool::new(false));
        let fetch_counters = Arc::new(FetchCounters::default());
        let (closed_sender, closed) = watch::channel(None);
        let target_watchers = Arc::new(Mutex::new(HashMap::<String, watch::Sender<Option<CdpCloseReason>>>::new()));
        let pending_for_reader = Arc::clone(&pending);
        let event_sender_for_reader = event_sender.clone();
        let sender_for_reader = sender.clone();
        let blocking_for_reader = Arc::clone(&block_resources);
        let fetch_counters_for_reader = Arc::clone(&fetch_counters);
        let target_watchers_for_reader = Arc::clone(&target_watchers);
        tokio::spawn(async move {
            let mut decided_fetch_requests = HashSet::new();
            while let Some(Ok(message)) = reader.next().await {
                let Message::Text(text) = message else { continue };
                let Ok(value) = serde_json::from_str::<Value>(text.as_ref()) else { continue };
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    let result = if let Some(error) = value.get("error") { Err(error.to_string()) } else { Ok(value.get("result").cloned().unwrap_or(Value::Null)) };
                    if let Some(waiter) = pending_for_reader.lock().await.remove(&id) { let _ = waiter.send(result); }
                    continue;
                }
                let Some(method) = value.get("method").and_then(Value::as_str) else { continue };
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                if method == "Network.responseReceived" {
                    if let Some(response) = NetworkResponse::from_event_params(&params) {
                        let _ = network_sender.send(response);
                    }
                }
                if method == "Target.targetDestroyed" {
                    if let Some(target_id) = params.get("targetId").and_then(Value::as_str) {
                        if let Some(watcher) = target_watchers_for_reader.lock().await.get(target_id) {
                            let _ = watcher.send(Some(CdpCloseReason::TargetDestroyed));
                        }
                    }
                }
                if blocking_for_reader.load(Ordering::Acquire) && method == "Fetch.requestPaused" {
                    if let Some(request_id) = params.get("requestId").and_then(Value::as_str) {
                        fetch_counters_for_reader.paused.fetch_add(1, Ordering::AcqRel);
                        if decided_fetch_requests.insert(request_id.to_string()) {
                            fetch_counters_for_reader.pending.fetch_add(1, Ordering::AcqRel);
                            let message = json!({"id": 0, "method": "Fetch.failRequest", "params": {"requestId": request_id, "errorReason": "Aborted"}});
                            if sender_for_reader.send(Message::Text(message.to_string().into())).is_ok() {
                                fetch_counters_for_reader.decided.fetch_add(1, Ordering::AcqRel);
                            }
                            fetch_counters_for_reader.pending.fetch_sub(1, Ordering::AcqRel);
                        } else {
                            fetch_counters_for_reader.duplicates.fetch_add(1, Ordering::AcqRel);
                        }
                    }
                }
                let _ = event_sender_for_reader.send(CdpEvent { method: method.to_string(), params });
            }
            let _ = closed_sender.send(Some(close_reason));
            let mut pending = pending_for_reader.lock().await;
            for (_, waiter) in pending.drain() { let _ = waiter.send(Err("cdp_target_closed".to_string())); }
        });
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if writer.send(message).await.is_err() { break; }
            }
        });
        Ok(Self { sender, pending, next_id: AtomicU64::new(1), events: Arc::new(Mutex::new(event_receiver)), network_responses: Arc::new(Mutex::new(network_receiver)), block_resources, fetch_counters, closed, target_watchers })
    }

    pub async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, String> {
        self.call_with_deadline(method, params, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn call_with_deadline<T: DeserializeOwned>(&self, method: &str, params: Value, deadline: Deadline) -> Result<T, String> {
        let remaining = deadline.remaining()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        let request = json!({"id": id, "method": method, "params": params});
        if self.sender.send(Message::Text(request.to_string().into())).is_err() {
            self.pending.lock().await.remove(&id);
            return Err("cdp_target_closed".to_string());
        }
        let result = match timeout(remaining, receiver).await {
            Ok(result) => result.map_err(|_| "cdp_target_closed".to_string())??,
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err("collector_deadline_exceeded".to_string());
            }
        };
        serde_json::from_value(result).map_err(|_| "cdp_protocol_error".to_string())
    }

    pub async fn next_event(&self) -> Result<CdpEvent, String> {
        self.events.lock().await.recv().await.ok_or_else(|| "cdp_target_closed".to_string())
    }

    async fn next_network_response(&self) -> Result<NetworkResponse, String> {
        self.network_responses
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| "cdp_target_closed".to_string())
    }

    pub async fn dispatch_left_click(&self, x: f64, y: f64) -> Result<(), String> {
        self.dispatch_left_click_with_deadline(x, y, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn dispatch_left_click_with_deadline(&self, x: f64, y: f64, deadline: Deadline) -> Result<(), String> {
        if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
            return Err("invalid_click_target".to_string());
        }
        for event_type in ["mousePressed", "mouseReleased"] {
            self.call_with_deadline::<Value>(
                "Input.dispatchMouseEvent",
                json!({"type": event_type, "x": x, "y": y, "button": "left", "clickCount": 1}),
                deadline,
            ).await?;
        }
        Ok(())
    }

    pub async fn dispatch_mouse_wheel(&self, x: f64, y: f64, delta_y: f64) -> Result<(), String> {
        self.dispatch_mouse_wheel_with_deadline(
            x,
            y,
            delta_y,
            Deadline::after(Duration::from_secs(60)),
        )
        .await
    }

    pub async fn dispatch_mouse_wheel_with_deadline(
        &self,
        x: f64,
        y: f64,
        delta_y: f64,
        deadline: Deadline,
    ) -> Result<(), String> {
        if !x.is_finite()
            || !y.is_finite()
            || !delta_y.is_finite()
            || x < 0.0
            || y < 0.0
            || delta_y == 0.0
        {
            return Err("invalid_scroll_target".to_string());
        }
        self.call_with_deadline::<Value>(
            "Input.dispatchMouseEvent",
            json!({"type": "mouseMoved", "x": x, "y": y}),
            deadline,
        )
        .await?;
        self.call_with_deadline::<Value>(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": x,
                "y": y,
                "deltaX": 0.0,
                "deltaY": delta_y
            }),
            deadline,
        )
        .await?;
        Ok(())
    }

    pub async fn call_function<T: DeserializeOwned>(&self, function_declaration: &str, argument: Value) -> Result<T, String> {
        self.call_function_with_deadline(function_declaration, argument, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn call_function_with_deadline<T: DeserializeOwned>(&self, function_declaration: &str, argument: Value, deadline: Deadline) -> Result<T, String> {
        let root: Value = self.call_with_deadline("Runtime.evaluate", json!({"expression":"globalThis", "returnByValue":false}), deadline).await?;
        let object_id = root.get("result").and_then(|result| result.get("objectId")).and_then(Value::as_str).ok_or_else(|| "cdp_protocol_error".to_string())?.to_string();
        let called = self.call_with_deadline::<Value>("Runtime.callFunctionOn", json!({
            "objectId": object_id,
            "functionDeclaration": function_declaration,
            "arguments": [{"value": argument}],
            "awaitPromise": true,
            "returnByValue": true
        }), deadline).await;
        let _ = self.call_with_deadline::<Value>("Runtime.releaseObject", json!({"objectId":object_id}), deadline).await;
        let called = called?;
        if called.get("exceptionDetails").is_some() { return Err("cdp_evaluate_failed".to_string()); }
        serde_json::from_value(called.get("result").and_then(|result| result.get("value")).cloned().unwrap_or(Value::Null)).map_err(|_| "cdp_evaluate_failed".to_string())
    }

    pub fn subscribe_closed(&self) -> watch::Receiver<Option<CdpCloseReason>> {
        self.closed.clone()
    }

    pub async fn subscribe_target_closed(
        &self,
        target_id: &str,
    ) -> watch::Receiver<Option<CdpCloseReason>> {
        let (sender, receiver) = watch::channel(None);
        self.target_watchers
            .lock()
            .await
            .insert(target_id.to_string(), sender);
        receiver
    }

    pub fn enable_resource_blocking(&self) { self.block_resources.store(true, Ordering::Release); }

    pub fn fetch_stats(&self) -> FetchStats { self.fetch_counters.snapshot() }

    pub async fn shutdown_resource_blocking(&self, deadline: Deadline) -> Result<FetchStats, String> {
        if self.block_resources.load(Ordering::Acquire) {
            self.call_with_deadline::<Value>("Fetch.disable", json!({}), deadline).await?;
            self.block_resources.store(false, Ordering::Release);
        }
        Ok(self.fetch_stats())
    }

    pub async fn close(&self) { let _ = self.sender.send(Message::Close(None)); }
}

fn create_target_params(hidden: bool) -> Value {
    if hidden {
        json!({"url":"about:blank", "background":true, "focus":false})
    } else {
        json!({"url":"about:blank"})
    }
}

fn visible_page_target_id(targets: &Value) -> Result<String, String> {
    let target_infos = targets.get("targetInfos").and_then(Value::as_array).ok_or_else(|| "cdp_protocol_error".to_string())?;
    let pages = target_infos.iter().filter(|target| target.get("type").and_then(Value::as_str) == Some("page")).collect::<Vec<_>>();
    let selected = if pages.len() == 1 {
        pages[0]
    } else {
        let blank_pages = pages.iter().copied().filter(|target| target.get("url").and_then(Value::as_str) == Some("about:blank")).collect::<Vec<_>>();
        if blank_pages.len() != 1 {
            return Err(if pages.is_empty() { "cdp_visible_target_unavailable" } else { "cdp_visible_target_ambiguous" }.to_string());
        }
        blank_pages[0]
    };
    selected.get("targetId").and_then(Value::as_str).filter(|target_id| !target_id.is_empty()).map(str::to_string).ok_or_else(|| "cdp_protocol_error".to_string())
}

pub async fn create_page(browser_url: &str, port: u16, hidden: bool, deadline: Deadline) -> Result<(CdpConnection, String), String> {
    let browser = CdpConnection::connect_with_deadline(browser_url, CdpCloseReason::BrowserSocket, deadline).await?;
    browser.call_with_deadline::<Value>("Target.setDiscoverTargets", json!({"discover": true}), deadline).await?;
    let target_id = if hidden {
        let target: Value = browser.call_with_deadline("Target.createTarget", create_target_params(true), deadline).await?;
        target.get("targetId").and_then(Value::as_str).ok_or_else(|| "cdp_protocol_error".to_string())?.to_string()
    } else {
        let launch_target_id = loop {
            let targets: Value = browser.call_with_deadline("Target.getTargets", json!({}), deadline).await?;
            match visible_page_target_id(&targets) {
                Ok(target_id) => break target_id,
                Err(error) if error == "cdp_visible_target_unavailable" => deadline.run(async { tokio::time::sleep(Duration::from_millis(50)).await; Ok(()) }).await?,
                Err(error) => return Err(error),
            }
        };
        let target: Value = browser.call_with_deadline("Target.createTarget", create_target_params(false), deadline).await?;
        let target_id = target.get("targetId").and_then(Value::as_str).ok_or_else(|| "cdp_protocol_error".to_string())?.to_string();
        browser.call_with_deadline::<Value>("Target.activateTarget", json!({"targetId": target_id}), deadline).await?;
        let closed: Value = browser.call_with_deadline("Target.closeTarget", json!({"targetId": launch_target_id}), deadline).await?;
        if closed.get("success").and_then(Value::as_bool) != Some(true) { return Err("cdp_launch_target_close_failed".to_string()); }
        target_id
    };
    let page_url = format!("ws://127.0.0.1:{port}/devtools/page/{target_id}");
    Ok((browser, page_url))
}

pub struct CdpPage {
    browser: CdpConnection,
    page: CdpConnection,
    target_id: String,
    closed: watch::Receiver<Option<CdpCloseReason>>,
}

impl CdpPage {
    pub async fn connect(browser_url: &str, port: u16, hidden: bool) -> Result<Self, String> {
        Self::connect_with_deadline(browser_url, port, hidden, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn connect_with_deadline(browser_url: &str, port: u16, hidden: bool, deadline: Deadline) -> Result<Self, String> {
        let (browser, page_url) = create_page(browser_url, port, hidden, deadline).await?;
        let target_id = page_url.rsplit('/').next().ok_or_else(|| "cdp_protocol_error".to_string())?.to_string();
        let mut browser_closed = browser.subscribe_closed();
        let mut target_closed = browser.subscribe_target_closed(&target_id).await;
        let page = CdpConnection::connect_with_deadline(&page_url, CdpCloseReason::PageSocket, deadline).await?;
        let mut page_closed = page.subscribe_closed();
        let (closed_sender, closed) = watch::channel(None);
        for receiver in [&mut browser_closed, &mut target_closed, &mut page_closed] {
            let mut receiver = receiver.clone();
            let closed_sender = closed_sender.clone();
            tokio::spawn(async move {
                if receiver.changed().await.is_ok() {
                    let reason = *receiver.borrow();
                    if reason.is_some() {
                        let _ = closed_sender.send(reason);
                    }
                }
            });
        }
        page.call_with_deadline::<Value>("Page.enable", json!({}), deadline).await?;
        page.call_with_deadline::<Value>("Runtime.enable", json!({}), deadline).await?;
        Ok(Self { browser, page, target_id, closed })
    }

    pub async fn navigate(&self, url: &str) -> Result<(), String> {
        self.navigate_with_deadline(url, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn navigate_with_deadline(&self, url: &str, deadline: Deadline) -> Result<(), String> {
        let mut events = self.page.events.lock().await;
        let navigation: Value = self.page.call_with_deadline("Page.navigate", json!({"url": url}), deadline).await?;
        if navigation.get("errorText").is_some() { return Err("navigation_failed".to_string()); }
        let wait = async {
            while let Some(event) = events.recv().await {
                if event.method == "Page.domContentEventFired" { return Ok::<(), String>(()); }
            }
            Err("cdp_target_closed".to_string())
        };
        deadline.run(wait).await?;
        Ok(())
    }

    pub async fn evaluate<T: DeserializeOwned>(&self, expression: &str) -> Result<T, String> {
        let result: Value = self.page.call("Runtime.evaluate", json!({"expression": expression, "awaitPromise": true, "returnByValue": true})).await?;
        if result.get("exceptionDetails").is_some() { return Err("cdp_evaluate_failed".to_string()); }
        serde_json::from_value(result.get("result").and_then(|value| value.get("value")).cloned().unwrap_or(Value::Null)).map_err(|_| "cdp_evaluate_failed".to_string())
    }

    pub async fn call_function<T: DeserializeOwned>(&self, function_declaration: &str, argument: Value) -> Result<T, String> {
        self.page.call_function(function_declaration, argument).await
    }

    pub async fn call_function_with_deadline<T: DeserializeOwned>(&self, function_declaration: &str, argument: Value, deadline: Deadline) -> Result<T, String> {
        self.page.call_function_with_deadline(function_declaration, argument, deadline).await
    }

    pub async fn dispatch_left_click(&self, x: f64, y: f64) -> Result<(), String> {
        self.page.dispatch_left_click(x, y).await
    }

    pub async fn dispatch_left_click_with_deadline(&self, x: f64, y: f64, deadline: Deadline) -> Result<(), String> {
        self.page.dispatch_left_click_with_deadline(x, y, deadline).await
    }

    pub async fn dispatch_mouse_wheel(
        &self,
        x: f64,
        y: f64,
        delta_y: f64,
    ) -> Result<(), String> {
        self.page.dispatch_mouse_wheel(x, y, delta_y).await
    }

    pub async fn dispatch_mouse_wheel_with_deadline(
        &self,
        x: f64,
        y: f64,
        delta_y: f64,
        deadline: Deadline,
    ) -> Result<(), String> {
        self.page
            .dispatch_mouse_wheel_with_deadline(x, y, delta_y, deadline)
            .await
    }

    pub async fn block_resources(&self, images: bool, media: bool) -> Result<(), String> {
        self.block_resources_with_deadline(images, media, Deadline::after(Duration::from_secs(60))).await
    }

    pub async fn block_resources_with_deadline(&self, images: bool, media: bool, deadline: Deadline) -> Result<(), String> {
        let mut patterns = Vec::new();
        if images { patterns.push(json!({"resourceType":"Image", "requestStage":"Request"})); }
        if media { patterns.push(json!({"resourceType":"Media", "requestStage":"Request"})); }
        if patterns.is_empty() { return Ok(()); }
        self.page.call_with_deadline::<Value>("Fetch.enable", json!({"patterns": patterns}), deadline).await?;
        self.page.enable_resource_blocking();
        Ok(())
    }

    pub async fn enable_network(&self) -> Result<(), String> {
        self.page
            .call::<Value>("Network.enable", json!({}))
            .await
            .map(|_| ())
    }

    pub async fn disable_network(&self) -> Result<(), String> {
        self.page
            .call::<Value>("Network.disable", json!({}))
            .await
            .map(|_| ())
    }

    pub async fn next_network_response(&self) -> Result<NetworkResponse, String> {
        self.page.next_network_response().await
    }

    pub async fn response_body(&self, request_id: &str) -> Result<Vec<u8>, String> {
        let result: Value = self
            .page
            .call(
                "Network.getResponseBody",
                json!({"requestId": request_id}),
            )
            .await?;
        let body = result
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| "cdp_protocol_error".to_string())?;
        if result
            .get("base64Encoded")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            STANDARD
                .decode(body)
                .map_err(|_| "cdp_protocol_error".to_string())
        } else {
            Ok(body.as_bytes().to_vec())
        }
    }

    pub fn fetch_stats(&self) -> FetchStats { self.page.fetch_stats() }

    pub async fn shutdown_resource_blocking_with_deadline(&self, deadline: Deadline) -> Result<FetchStats, String> {
        self.page.shutdown_resource_blocking(deadline).await
    }

    pub async fn wait_for_timeout(&self, milliseconds: u64) { tokio::time::sleep(Duration::from_millis(milliseconds)).await; }

    pub fn subscribe_closed(&self) -> watch::Receiver<Option<CdpCloseReason>> {
        self.closed.clone()
    }

    pub async fn close(&self) {
        let _ = self.close_with_deadline(Deadline::after(Duration::from_secs(60))).await;
    }

    pub async fn close_with_deadline(&self, deadline: Deadline) -> Result<(), String> {
        let resource_shutdown = self.shutdown_resource_blocking_with_deadline(deadline).await;
        self.browser.call_with_deadline::<Value>("Target.closeTarget", json!({"targetId": self.target_id}), deadline).await?;
        match self.browser.call_with_deadline::<Value>("Browser.close", json!({}), deadline).await {
            Ok(_) => {}
            Err(error) if error == "cdp_target_closed" => {}
            Err(error) => return Err(error),
        }
        self.page.close().await;
        self.browser.close().await;
        resource_shutdown.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{create_target_params, visible_page_target_id, NetworkResponse};
    use serde_json::json;

    #[test]
    fn hidden_sync_creates_a_background_target_without_focus() {
        assert_eq!(
            create_target_params(true),
            json!({"url":"about:blank", "background":true, "focus":false})
        );
        assert_eq!(create_target_params(false), json!({"url":"about:blank"}));
    }

    #[test]
    fn visible_launch_target_selection_accepts_the_only_page() {
        assert_eq!(
            visible_page_target_id(&json!({
                "targetInfos": [
                    {"targetId": "page-1", "type": "page", "url": "edge://newtab/"},
                    {"targetId": "worker-1", "type": "worker", "url": ""}
                ]
            })),
            Ok("page-1".to_string())
        );
    }

    #[test]
    fn visible_launch_target_selection_prefers_the_unique_blank_page() {
        assert_eq!(
            visible_page_target_id(&json!({
                "targetInfos": [
                    {"targetId": "page-1", "type": "page", "url": "edge://welcome/"},
                    {"targetId": "page-2", "type": "page", "url": "about:blank"}
                ]
            })),
            Ok("page-2".to_string())
        );
    }

    #[test]
    fn visible_launch_target_selection_rejects_ambiguous_pages() {
        assert_eq!(
            visible_page_target_id(&json!({
                "targetInfos": [
                    {"targetId": "page-1", "type": "page", "url": "edge://welcome/"},
                    {"targetId": "page-2", "type": "page", "url": "edge://newtab/"}
                ]
            })),
            Err("cdp_visible_target_ambiguous".to_string())
        );
    }

    #[test]
    fn network_detail_candidate_requires_the_exact_detail_endpoint_and_aweme_query() {
        let response = NetworkResponse {
            request_id: "1".into(),
            resource_type: "XHR".into(),
            url: "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=7600&aid=6383"
                .into(),
            status: 200,
            mime_type: "application/json".into(),
        };
        assert!(response.is_json_detail_for("7600"));

        let mut unrelated = response.clone();
        unrelated.url = "https://www.douyin.com/aweme/v1/web/aweme/post/?aweme_id=7600".into();
        assert!(!unrelated.is_json_detail_for("7600"));

        let mut wrong_aweme = response;
        wrong_aweme.url = "https://www.douyin.com/aweme/v1/web/aweme/detail/?aweme_id=7601".into();
        assert!(!wrong_aweme.is_json_detail_for("7600"));
    }
}
