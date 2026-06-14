//! 系统概览 API
//!
//! `GET /api/system/overview` — 实时采集并返回完整 SystemOverview。

use axum::Json;
use serde_json::Value;
use crate::collector;
use super::success;

/// 采集并返回当前系统概览（每次请求触发一次实时采集）。
pub async fn overview() -> Json<Value> {
    let data = collector::collect_system_overview();
    success(data)
}
