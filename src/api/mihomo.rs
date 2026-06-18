//! Mihomo 订阅更新 API

use axum::Json;
use serde_json::{json, Value};

use crate::api::{error, success};
use crate::collector;

pub async fn update_subscription() -> Json<Value> {
    let script = std::env::var("MIHOMO_FETCH_SUB_SCRIPT")
        .unwrap_or_else(|_| "/home/user/code/fetch_sub.py".to_string());

    let output = std::process::Command::new("python3").arg(&script).output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            if out.status.success() {
                success(json!({
                    "ok": true,
                    "output": stdout.trim(),
                    "last_update": collector::mihomo::subscription_last_update(),
                }))
            } else {
                let msg = if stderr.is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("{}\n{}", stdout.trim(), stderr.trim())
                };
                error(&format!("订阅更新失败: {}", msg))
            }
        }
        Err(e) => error(&format!("无法执行 {}: {}", script, e)),
    }
}
