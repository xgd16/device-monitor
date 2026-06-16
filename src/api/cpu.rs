//! CPU 信息 API
//!
//! - `GET /api/cpu` — 返回 CPU 总体使用率、各核心使用率与频率。
//! - `GET /api/cpu/governor` — 返回 CPU governor 信息（当前策略、可用策略）。
//! - `POST /api/cpu/governor` — 设置 CPU governor。

use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use crate::collector;
use super::{success, error};

/// GET /api/cpu
pub async fn cpu_info() -> Json<Value> {
    let data = collector::cpu::collect();
    success(data)
}

/// GET /api/cpu/governor
pub async fn get_governor() -> Json<Value> {
    let governor = collector::cpu::get_governor();
    success(governor)
}

/// POST /api/cpu/governor 的请求体
#[derive(Deserialize)]
pub struct SetGovernorRequest {
    pub governor: String,
}

/// POST /api/cpu/governor
pub async fn set_governor(Json(req): Json<SetGovernorRequest>) -> Json<Value> {
    match collector::cpu::set_governor(&req.governor) {
        Ok(()) => {
            // 返回更新后的 governor 信息
            let governor = collector::cpu::get_governor();
            success(serde_json::json!({
                "message": format!("已切换到 {} 模式", req.governor),
                "governor": governor
            }))
        }
        Err(e) => error(&e),
    }
}
