//! CPU 信息 API
//!
//! - `GET /api/cpu` — 返回 CPU 总体使用率、各核心使用率与频率。
//! - `GET /api/cpu/governor` — 返回 CPU governor 信息（当前策略、可用策略）。
//! - `POST /api/cpu/governor` — 设置 CPU governor。
//! - `GET /api/cpu/frequency` — 返回 CPU 频率信息。
//! - `POST /api/cpu/low-power` — 设置低频省电模式。
//! - `POST /api/cpu/normal` — 恢复正常模式。

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

/// GET /api/cpu/frequency
pub async fn get_frequency() -> Json<Value> {
    let freq = collector::cpu::get_frequency();
    success(freq)
}

/// POST /api/cpu/low-power 的请求体
#[derive(Deserialize)]
pub struct SetFrequencyRequest {
    /// 最大频率限制 (kHz)，可选。不提供则使用最低可用频率。
    pub max_freq: Option<u64>,
}

/// POST /api/cpu/low-power
/// 设置低频省电模式
pub async fn set_low_power_mode(Json(req): Json<SetFrequencyRequest>) -> Json<Value> {
    let result = if let Some(freq) = req.max_freq {
        collector::cpu::set_max_frequency_limit(freq)
    } else {
        collector::cpu::set_low_power_mode()
    };
    
    match result {
        Ok(governor) => success(serde_json::json!({
            "message": "已切换到低频省电模式",
            "governor": governor
        })),
        Err(e) => error(&e),
    }
}

/// POST /api/cpu/normal
/// 恢复正常模式
pub async fn set_normal_mode() -> Json<Value> {
    match collector::cpu::set_normal_mode() {
        Ok(governor) => success(serde_json::json!({
            "message": "已恢复正常模式",
            "governor": governor
        })),
        Err(e) => error(&e),
    }
}
