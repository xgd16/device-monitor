//! 系统概览 API
//!
//! - `GET /api/system/overview` — 实时采集并返回完整 SystemOverview
//! - `GET/POST /api/system/refresh` — 界面刷新间隔（秒），即采集心跳，
//!   WebSocket 推送与 DRM 物理屏重绘随之变化

use axum::Json;
use axum::extract::State;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use crate::store;
use crate::AppState;
use super::{success, error};

/// 返回后台采集任务维护的最新系统概览，避免每次 HTTP 请求重复采集。
pub async fn overview(State(state): State<AppState>) -> Json<Value> {
    let data = state.latest.borrow().clone();
    success(data)
}

/// 当前界面刷新间隔与可选项。
pub async fn get_refresh(State(state): State<AppState>) -> Json<Value> {
    success(json!({
        "refresh_secs": state.refresh_secs.load(Ordering::Relaxed),
        "choices": store::settings::REFRESH_CHOICES,
    }))
}

/// 设置界面刷新间隔（秒）。立即生效于采集心跳，进而驱动 WS 推送与 DRM 屏重绘；
/// 持久化到 refresh_secs.txt，重启保留。
pub async fn set_refresh(State(state): State<AppState>, Json(body): Json<Value>) -> Json<Value> {
    let Some(secs) = body.get("secs").and_then(|v| v.as_u64()) else {
        return error("缺少 secs（整数秒）");
    };
    if !store::settings::REFRESH_CHOICES.contains(&secs) {
        return error(&format!("刷新间隔仅支持 {:?} 秒", store::settings::REFRESH_CHOICES));
    }

    state.refresh_secs.store(secs, Ordering::Relaxed);
    if let Err(e) = store::settings::save_refresh_secs(secs) {
        // 设置已即时生效，落盘失败只影响重启后的恢复
        tracing::warn!("保存刷新间隔失败: {e}");
    }
    tracing::info!("界面刷新间隔改为 {secs} 秒");
    success(json!({ "refresh_secs": secs }))
}
