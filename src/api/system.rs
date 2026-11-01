//! 系统概览 API
//!
//! - `GET /api/system/overview` — 实时采集并返回完整 SystemOverview
//! - `GET/POST /api/system/refresh` — 界面刷新间隔（秒），即采集心跳，
//!   WebSocket 推送与 DRM 物理屏重绘随之变化
//! - `GET/POST /api/system/rotation` — DRM 屏横向朝向（面板竖装，横放有两个朝向），
//!   真机上也可以双击音量上加翻转
//! - `POST /api/system/power` — 重启 / 关机（面板设置页二次确认后调用）

use super::{error, success};
use crate::AppState;
use crate::store;
use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

/// 电源操作互斥：请求已下发后拒绝重复调用，避免 systemd 同时收到 reboot/poweroff。
static POWER_PENDING: AtomicBool = AtomicBool::new(false);

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

/// 物理屏当前朝向（供 Web 端展示；实际渲染由启动器拉起面板时读取 rotation.txt 生效）。
pub async fn get_rotation() -> Json<Value> {
    success(json!({
        "rot270": crate::collector::hotkeys::rot270(),
        "rotation": if crate::collector::hotkeys::rot270() { "rot270" } else { "rot90" },
        "choices": store::settings::ROTATION_CHOICES,
    }))
}

/// 设置物理屏朝向。与双击音量上走同一条状态与落盘路径（rotation.txt），
/// 由启动器在拉起面板时生效（面板重启后按新朝向渲染）。
pub async fn set_rotation(Json(body): Json<Value>) -> Json<Value> {
    let Some(rot) = body.get("rotation").and_then(|v| v.as_str()) else {
        return error("缺少 rotation（rot90 / rot270）");
    };
    if !store::settings::ROTATION_CHOICES.contains(&rot) {
        return error(&format!("朝向仅支持 {:?}", store::settings::ROTATION_CHOICES));
    }
    let rot270 = rot == "rot270";
    crate::collector::hotkeys::set_rot270(rot270);
    if let Err(e) = store::settings::save_rotation(rot) {
        tracing::warn!("保存屏幕朝向失败: {e}");
    }
    tracing::info!("屏幕朝向改为 {rot}（来源：API）");
    success(json!({ "rotation": rot }))
}

/// `POST /api/system/power` — 重启 / 关机。
///
/// 只接受 `reboot` / `poweroff`；延迟 1.5 秒再交给 systemd，让 HTTP 响应和
/// 面板的「正在重启…」提示先到达，再用 `--no-block` 立即返回，不阻塞关机流程。
pub async fn power_control(Json(body): Json<Value>) -> Json<Value> {
    let Some(action) = body.get("action").and_then(|v| v.as_str()) else {
        return error("缺少 action（reboot / poweroff）");
    };
    if action != "reboot" && action != "poweroff" {
        return error("action 仅支持 reboot / poweroff");
    }
    if POWER_PENDING.swap(true, Ordering::SeqCst) {
        return error("已有电源操作在执行");
    }

    tracing::warn!("收到电源操作：{action}（1.5s 后执行 systemctl --no-block {action}）");
    let action_for_thread = action.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let status = std::process::Command::new("systemctl").args(["--no-block", action_for_thread.as_str()]).status();
        match status {
            Ok(s) if s.success() => {
                tracing::warn!("systemctl --no-block {action_for_thread} 已下发");
            }
            Ok(s) => {
                tracing::error!("systemctl --no-block {action_for_thread} 退出码 {s}");
                POWER_PENDING.store(false, Ordering::SeqCst);
            }
            Err(e) => {
                tracing::error!("systemctl --no-block {action_for_thread} 执行失败: {e}");
                POWER_PENDING.store(false, Ordering::SeqCst);
            }
        }
    });

    success(json!({ "action": action, "scheduled_in_ms": 1500 }))
}
