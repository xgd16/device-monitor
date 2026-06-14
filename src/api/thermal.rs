//! 温度传感器 API
//!
//! `GET /api/thermal` — 返回所有 thermal zone 的温度读数。

use axum::Json;
use serde_json::Value;
use crate::collector;
use super::success;

pub async fn thermal_info() -> Json<Value> {
    let data = collector::thermal::collect();
    success(data)
}
