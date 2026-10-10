//! 温度传感器 API
//!
//! `GET /api/thermal` — 返回所有 thermal zone 的温度读数。

use super::success;
use crate::collector;
use axum::Json;
use serde_json::Value;

pub async fn thermal_info() -> Json<Value> {
    let data = collector::thermal::collect();
    success(data)
}
