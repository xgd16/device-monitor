//! CPU 信息 API
//!
//! `GET /api/cpu` — 返回 CPU 总体使用率、各核心使用率与频率。

use axum::Json;
use serde_json::Value;
use crate::collector;
use super::success;

pub async fn cpu_info() -> Json<Value> {
    let data = collector::cpu::collect();
    success(data)
}
