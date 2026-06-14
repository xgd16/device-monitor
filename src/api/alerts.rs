//! 告警 API
//!
//! - `GET /api/alerts` — 最近 50 条告警
//! - `GET /api/alerts/config` — 告警阈值配置
//! - `PUT /api/alerts/config` — 更新配置（尚未持久化到数据库）

use axum::Json;
use serde_json::Value;
use crate::AppState;
use axum::extract::State;
use super::success;

pub async fn get_alerts(State(state): State<AppState>) -> Json<Value> {
    let alerts = state.db.get_alerts(50).unwrap_or_default();
    success(alerts)
}

pub async fn get_config() -> Json<Value> {
    let config = crate::alert::AlertConfig::default();
    success(config)
}

pub async fn update_config(Json(config): Json<crate::alert::AlertConfig>) -> Json<Value> {
    // TODO: 持久化到数据库并同步到 AlertEngine
    success(config)
}
