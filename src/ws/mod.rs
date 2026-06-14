//! WebSocket 实时推送模块
//!
//! 客户端连接 `/ws/realtime` 后，每当后台采集任务推送新的 `SystemOverview`，
//! 服务端即将其序列化为 JSON 文本帧发送给客户端。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use crate::AppState;

/// WebSocket 握手入口，升级 HTTP 连接为 WebSocket 并启动消息循环。
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.latest.clone();
    ws.on_upgrade(move |socket| handle_ws(socket, rx))
}

/// 双向监听：采集数据更新 → 推送 JSON；客户端 Close → 断开。
async fn handle_ws(mut socket: WebSocket, mut rx: tokio::sync::watch::Receiver<crate::collector::SystemOverview>) {
    tracing::info!("WebSocket client connected");

    loop {
        tokio::select! {
            // watch 通道有新数据时推送
            res = rx.changed() => {
                if res.is_err() { break; }
                let overview = rx.borrow_and_update().clone();
                let msg = serde_json::to_string(&overview).unwrap_or_default();
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            // 监听客户端消息（主要处理 Close）
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    tracing::info!("WebSocket client disconnected");
}
