//! 进程管理 API
//!
//! - `GET /api/process` — 进程列表（按内存降序）
//! - `GET /api/process/{pid}` — 进程详情
//! - `POST /api/process/{pid}/kill` — 发送信号终止进程

use axum::Json;
use axum::extract::Path;
use serde::Deserialize;
use serde_json::Value;
use crate::collector;
use super::{success, error};

pub async fn process_list() -> Json<Value> {
    let data = collector::process::list_processes();
    success(data)
}

pub async fn process_detail(Path(pid): Path<i32>) -> Json<Value> {
    match collector::process::get_process_detail(pid) {
        Some(data) => success(data),
        None => error("Process not found"),
    }
}

/// kill 请求体，signal 默认为 TERM。
#[derive(Deserialize)]
pub struct KillParams {
    pub signal: Option<String>,
}

pub async fn process_kill(
    Path(pid): Path<i32>,
    Json(params): Json<KillParams>,
) -> Json<Value> {
    let sig = params.signal.as_deref().unwrap_or("TERM");
    let signal_arg = match sig {
        "TERM" => "-TERM",
        "KILL" => "-KILL",
        "STOP" => "-STOP",
        "CONT" => "-CONT",
        "HUP" => "-HUP",
        "USR1" => "-USR1",
        "USR2" => "-USR2",
        _ => return error("Unsupported signal"),
    };

    match std::process::Command::new("kill")
        .arg(signal_arg)
        .arg(pid.to_string())
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                success(serde_json::json!({
                    "pid": pid,
                    "signal": sig,
                    "ok": true
                }))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                error(&format!("kill failed: {}", stderr.trim()))
            }
        }
        Err(e) => error(&format!("kill error: {}", e)),
    }
}
