//! 数据库管理 API
//!
//! - `GET /api/database/stats` — 数据库统计信息
//! - `POST /api/database/cleanup` — 手动触发历史数据清理

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};
use crate::store;
use crate::AppState;
use super::{success, error};

/// 获取 metrics/alerts 记录数及时间范围。
pub async fn get_stats(State(state): State<AppState>) -> Json<Value> {
    match state.db.get_stats(store::retention_days()) {
        Ok(stats) => success(stats),
        Err(e) => error(&format!("获取统计信息失败: {}", e)),
    }
}

/// 手动清理超过保留期（默认 7 天）的历史数据。
pub async fn cleanup(State(state): State<AppState>) -> Json<Value> {
    let days = store::retention_days();
    match state.db.cleanup_old_data(days) {
        Ok((metrics_deleted, alerts_deleted)) => {
            success(json!({
                "metrics_deleted": metrics_deleted,
                "alerts_deleted": alerts_deleted,
                "retention_days": days,
                "message": format!("清理完成: 删除了 {} 条指标记录, {} 条告警记录", metrics_deleted, alerts_deleted)
            }))
        }
        Err(e) => error(&format!("数据清理失败: {}", e)),
    }
}
