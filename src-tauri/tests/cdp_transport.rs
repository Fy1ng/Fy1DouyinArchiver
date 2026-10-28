use douyin_history_archive::collector::cdp::{CdpCloseReason, CdpConnection, FetchStats};
use douyin_history_archive::collector::deadline::Deadline;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[tokio::test]
async fn deadline_uses_one_absolute_budget_and_expired_calls_send_nothing() {
    let deadline = Deadline::after(Duration::from_millis(40));
    let first = deadline.remaining().expect("initial budget");
    tokio::time::sleep(Duration::from_millis(15)).await;
    let second = deadline.remaining().expect("remaining budget");
    assert!(second < first);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(80), socket.next())
            .await
            .is_err());
    });
    let connection = CdpConnection::connect(&format!("ws://{address}"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        connection
            .call_with_deadline::<Value>("Runtime.evaluate", json!({}), deadline)
            .await,
        Err("collector_deadline_exceeded".to_string())
    );
    server.await.unwrap();
}

#[tokio::test]
async fn deadline_is_propagated_to_function_calls_and_mouse_input() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(80), socket.next())
            .await
            .is_err());
    });
    let connection = CdpConnection::connect(&format!("ws://{address}"))
        .await
        .unwrap();
    let expired = Deadline::after(Duration::from_millis(1));
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(
        connection
            .call_function_with_deadline::<Value>(
                "function () { return true; }",
                json!(null),
                expired,
            )
            .await,
        Err("collector_deadline_exceeded".to_string())
    );
    assert_eq!(
        connection
            .dispatch_left_click_with_deadline(10.0, 20.0, expired)
            .await,
        Err("collector_deadline_exceeded".to_string())
    );
    server.await.unwrap();
}

#[tokio::test]
async fn websocket_close_aborts_pending_calls_and_broadcasts_the_reason() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let _ = socket.next().await;
        socket.close(None).await.unwrap();
    });

    let connection = CdpConnection::connect_with_close_reason(
        &format!("ws://{address}"),
        CdpCloseReason::PageSocket,
    )
    .await
    .unwrap();
    let mut closed = connection.subscribe_closed();
    let started = Instant::now();
    assert_eq!(
        connection
            .call::<Value>("Runtime.evaluate", json!({}))
            .await,
        Err("cdp_target_closed".to_string())
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    closed.changed().await.unwrap();
    assert_eq!(*closed.borrow(), Some(CdpCloseReason::PageSocket));
}

#[tokio::test]
async fn matching_target_destroyed_uses_a_dedicated_close_channel() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let request = socket.next().await.unwrap().unwrap();
        let Message::Text(request) = request else { panic!("request") };
        let request: Value = serde_json::from_str(request.as_ref()).unwrap();
        let id = request["id"].as_u64().unwrap();
        socket
            .send(Message::Text(
                json!({"method":"Target.targetDestroyed","params":{"targetId":"other"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({"id":id,"result":{}}).to_string().into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({"method":"Target.targetDestroyed","params":{"targetId":"expected"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    let connection = CdpConnection::connect(&format!("ws://{address}"))
        .await
        .unwrap();
    let mut closed = connection.subscribe_target_closed("expected").await;
    connection
        .call::<Value>("Target.setDiscoverTargets", json!({"discover": true}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), closed.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*closed.borrow(), Some(CdpCloseReason::TargetDestroyed));
}

#[tokio::test]
async fn dispatch_left_click_sends_pressed_then_released_at_the_same_coordinates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        for expected_type in ["mousePressed", "mouseReleased"] {
            let Message::Text(request) = socket.next().await.unwrap().unwrap() else { panic!("request") };
            let request: Value = serde_json::from_str(request.as_ref()).unwrap();
            assert_eq!(request["method"], "Input.dispatchMouseEvent");
            assert_eq!(request["params"]["type"], expected_type);
            assert_eq!(request["params"]["x"], 80.0);
            assert_eq!(request["params"]["y"], 50.0);
            assert_eq!(request["params"]["button"], "left");
            socket.send(Message::Text(json!({"id":request["id"],"result":{}}).to_string().into())).await.unwrap();
        }
    });
    let connection = CdpConnection::connect(&format!("ws://{address}")).await.unwrap();
    connection.dispatch_left_click(80.0, 50.0).await.unwrap();
}

#[tokio::test]
async fn dispatch_mouse_wheel_moves_to_content_before_sending_delta() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        for expected_type in ["mouseMoved", "mouseWheel"] {
            let Message::Text(request) = socket.next().await.unwrap().unwrap() else {
                panic!("request")
            };
            let request: Value = serde_json::from_str(request.as_ref()).unwrap();
            assert_eq!(request["method"], "Input.dispatchMouseEvent");
            assert_eq!(request["params"]["type"], expected_type);
            assert_eq!(request["params"]["x"], 640.0);
            assert_eq!(request["params"]["y"], 450.0);
            if expected_type == "mouseWheel" {
                assert_eq!(request["params"]["deltaX"], 0.0);
                assert_eq!(request["params"]["deltaY"], 1200.0);
            }
            socket
                .send(Message::Text(
                    json!({"id": request["id"], "result": {}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        }
    });
    let connection = CdpConnection::connect(&format!("ws://{address}"))
        .await
        .unwrap();
    connection
        .dispatch_mouse_wheel(640.0, 450.0, 1200.0)
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn call_function_passes_json_as_a_cdp_argument_without_source_concatenation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let Message::Text(root) = socket.next().await.unwrap().unwrap() else { panic!("root") };
        let root: Value = serde_json::from_str(root.as_ref()).unwrap();
        assert_eq!(root["method"], "Runtime.evaluate");
        socket.send(Message::Text(json!({"id":root["id"],"result":{"result":{"objectId":"root-1"}}}).to_string().into())).await.unwrap();
        let Message::Text(call) = socket.next().await.unwrap().unwrap() else { panic!("call") };
        let call: Value = serde_json::from_str(call.as_ref()).unwrap();
        assert_eq!(call["method"], "Runtime.callFunctionOn");
        assert_eq!(call["params"]["arguments"][0]["value"], json!({"action":"save_login"}));
        socket.send(Message::Text(json!({"id":call["id"],"result":{"result":{"value":{"kind":"found"}}}}).to_string().into())).await.unwrap();
        let Message::Text(release) = socket.next().await.unwrap().unwrap() else { panic!("release") };
        let release: Value = serde_json::from_str(release.as_ref()).unwrap();
        socket.send(Message::Text(json!({"id":release["id"],"result":{}}).to_string().into())).await.unwrap();
    });
    let connection = CdpConnection::connect(&format!("ws://{address}")).await.unwrap();
    let result: Value = connection.call_function("function (input) { return input; }", json!({"action":"save_login"})).await.unwrap();
    assert_eq!(result, json!({"kind":"found"}));
}

#[tokio::test]
async fn duplicate_fetch_pause_receives_exactly_one_decision() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let Message::Text(enable) = socket.next().await.unwrap().unwrap() else { panic!("enable") };
        let enable: Value = serde_json::from_str(enable.as_ref()).unwrap();
        socket.send(Message::Text(json!({"id":enable["id"],"result":{}}).to_string().into())).await.unwrap();
        let paused = json!({"method":"Fetch.requestPaused","params":{"requestId":"request-1","resourceType":"Media","request":{"url":"https://www.douyin.com/media"}}});
        socket.send(Message::Text(paused.to_string().into())).await.unwrap();
        socket.send(Message::Text(paused.to_string().into())).await.unwrap();
        let Message::Text(decision) = socket.next().await.unwrap().unwrap() else { panic!("decision") };
        let decision: Value = serde_json::from_str(decision.as_ref()).unwrap();
        assert_eq!(decision["method"], "Fetch.failRequest");
        assert_eq!(decision["params"]["requestId"], "request-1");
        assert!(tokio::time::timeout(Duration::from_millis(100), socket.next()).await.is_err());
    });
    let connection = CdpConnection::connect(&format!("ws://{address}")).await.unwrap();
    connection.enable_resource_blocking();
    connection.call::<Value>("Fetch.enable", json!({})).await.unwrap();
    server.await.unwrap();
    assert_eq!(connection.fetch_stats(), FetchStats { paused: 2, decided: 1, duplicates: 1, pending: 0 });
}

#[tokio::test]
async fn fetch_shutdown_disables_the_domain_after_all_requests_are_decided() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let Message::Text(enable) = socket.next().await.unwrap().unwrap() else { panic!("enable") };
        let enable: Value = serde_json::from_str(enable.as_ref()).unwrap();
        socket.send(Message::Text(json!({"id":enable["id"],"result":{}}).to_string().into())).await.unwrap();
        socket.send(Message::Text(json!({"method":"Fetch.requestPaused","params":{"requestId":"request-1","resourceType":"Media","request":{"url":"https://www.douyin.com/media"}}}).to_string().into())).await.unwrap();
        let Message::Text(decision) = socket.next().await.unwrap().unwrap() else { panic!("decision") };
        let decision: Value = serde_json::from_str(decision.as_ref()).unwrap();
        assert_eq!(decision["method"], "Fetch.failRequest");
        let Message::Text(disable) = socket.next().await.unwrap().unwrap() else { panic!("disable") };
        let disable: Value = serde_json::from_str(disable.as_ref()).unwrap();
        assert_eq!(disable["method"], "Fetch.disable");
        socket.send(Message::Text(json!({"id":disable["id"],"result":{}}).to_string().into())).await.unwrap();
    });
    let connection = CdpConnection::connect(&format!("ws://{address}")).await.unwrap();
    connection.enable_resource_blocking();
    connection.call::<Value>("Fetch.enable", json!({})).await.unwrap();
    let stats = connection.shutdown_resource_blocking(Deadline::after(Duration::from_secs(1))).await.unwrap();
    assert_eq!(stats, FetchStats { paused: 1, decided: 1, duplicates: 0, pending: 0 });
    server.await.unwrap();
}
