//! 系统概览 API
//!
//! `GET /api/system/overview` — 实时采集并返回完整 SystemOverview。

use axum::Json;
use axum::extract::State;
use serde_json::Value;
use crate::AppState;
use super::success;

/// 返回后台采集任务维护的最新系统概览，避免每次 HTTP 请求重复采集。
pub async fn overview(State(state): State<AppState>) -> Json<Value> {
    let data = state.latest.borrow().clone();
    success(data)
}
