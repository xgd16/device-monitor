//! WebSocket 模块
//!
//! - `/ws/realtime` — 系统指标实时推送
//! - `/ws/terminal` — PTY 终端会话

pub mod terminal;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::AppState;

static WS_CLIENTS: AtomicUsize = AtomicUsize::new(0);

/// 当前活跃的 realtime WS 客户端数。
/// 熄屏降频的前提条件之一：有看板连着就不降（手机可能只当采集器用）。
pub fn client_count() -> usize {
    WS_CLIENTS.load(Ordering::Relaxed)
}

/// 连接计数守卫：handle_ws 的所有退出路径（含 panic 之外的 break/返回）都可靠 -1。
struct ClientGuard;
impl Drop for ClientGuard {
    fn drop(&mut self) {
        WS_CLIENTS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// WebSocket 握手入口，升级 HTTP 连接为 WebSocket 并启动消息循环。
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.latest.clone();
    ws.on_upgrade(move |socket| handle_ws(socket, rx))
}

/// 双向监听：采集数据更新 → 推送 JSON；客户端 Close → 断开。
async fn handle_ws(mut socket: WebSocket, mut rx: tokio::sync::watch::Receiver<crate::collector::SystemOverview>) {
    WS_CLIENTS.fetch_add(1, Ordering::Relaxed);
    let _guard = ClientGuard;
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
