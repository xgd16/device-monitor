//! 电池信息 API
//!
//! `GET /api/battery` — 返回电池容量、状态、电压、电流、温度等。

use super::success;
use crate::collector;
use axum::Json;
use serde_json::Value;

pub async fn battery_info() -> Json<Value> {
    let data = collector::battery::collect();
    success(data)
}
