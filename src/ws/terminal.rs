//! Web 终端 WebSocket 模块
//!
//! 客户端连接 `/ws/terminal` 后获得独立 PTY shell 会话。
//! 协议：客户端 Binary → PTY 输入；Text JSON resize → 调整终端尺寸；
//! 服务端 Binary ← PTY 输出；Text JSON exit ← 进程退出通知。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::ConnectInfo;
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock, Mutex};

const MAX_SESSIONS_PER_IP: usize = 3;

static SESSION_COUNTS: LazyLock<Arc<Mutex<HashMap<String, usize>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(HashMap::new())));

#[derive(Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    msg_type: String,
    cols: Option<u16>,
    rows: Option<u16>,
}

struct SessionGuard {
    ip: String,
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        let mut counts = SESSION_COUNTS.lock().unwrap();
        if let Some(n) = counts.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                counts.remove(&self.ip);
            }
        }
    }
}

fn try_acquire_session(ip: &str) -> Result<SessionGuard, Response> {
    let mut counts = SESSION_COUNTS.lock().unwrap();
    let n = counts.entry(ip.to_string()).or_insert(0);
    if *n >= MAX_SESSIONS_PER_IP {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many terminal sessions for this client",
        )
            .into_response());
    }
    *n += 1;
    Ok(SessionGuard { ip: ip.to_string() })
}

/// WebSocket 握手入口。
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let ip = addr.ip().to_string();
    match try_acquire_session(&ip) {
        Ok(guard) => ws.on_upgrade(move |socket| handle_terminal(socket, guard)),
        Err(resp) => resp,
    }
}

async fn handle_terminal(socket: WebSocket, _guard: SessionGuard) {
    tracing::info!("Terminal WebSocket connected");

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to open PTY: {}", e);
            return;
        }
    };

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");

    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to spawn shell: {}", e);
            return;
        }
    };

    let master = pair.master;
    let mut reader = match master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to clone PTY reader: {}", e);
            let _ = child.kill();
            return;
        }
    };
    let mut writer = match master.take_writer() {
        Ok(w) => w,
        Err(e) => {
            tracing::error!("Failed to take PTY writer: {}", e);
            let _ = child.kill();
            return;
        }
    };

    let (mut ws_tx, mut ws_rx) = socket.split();
    let (pty_tx, mut pty_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

    // PTY 读取线程
    let read_handle = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if pty_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!("PTY read error: {}", e);
                    break;
                }
            }
        }
    });

    loop {
        tokio::select! {
            // PTY → WebSocket
            Some(data) = pty_rx.recv() => {
                if ws_tx.send(Message::Binary(data.into())).await.is_err() {
                    break;
                }
            }
            // WebSocket → PTY
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        if writer.write_all(&data).is_err() {
                            break;
                        }
                        let _ = writer.flush();
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(parsed) = serde_json::from_str::<ClientMessage>(&text) {
                            match parsed.msg_type.as_str() {
                                "resize" => {
                                    if let (Some(cols), Some(rows)) = (parsed.cols, parsed.rows) {
                                        let _ = master.resize(PtySize {
                                            rows,
                                            cols,
                                            pixel_width: 0,
                                            pixel_height: 0,
                                        });
                                    }
                                }
                                "ping" => {
                                    let _ = ws_tx.send(Message::Text(r#"{"type":"pong"}"#.into())).await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
            // 检测 shell 退出
            _ = tokio::task::yield_now() => {
                if let Ok(Some(status)) = child.try_wait() {
                    let code = status.exit_code();
                    let exit_msg = serde_json::json!({"type":"exit","code":code}).to_string();
                    let _ = ws_tx.send(Message::Text(exit_msg.into())).await;
                    break;
                }
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = read_handle.await;
    tracing::info!("Terminal WebSocket disconnected");
}
