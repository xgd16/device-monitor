//! 硬件控制 API
//!
//! 提供手电筒、屏幕亮度、背光、振动、内存清理等控制接口。
//! 底层通过 collector::hardware 读写 sysfs 节点。

use axum::Json;
use serde::Deserialize;
use serde_json::Value;
use crate::collector;
use super::{success, error};

/// `GET /api/hardware` — 读取当前硬件状态。
pub async fn hardware_state() -> Json<Value> {
    success(collector::hardware::get_state())
}

#[derive(Deserialize)]
pub struct FlashlightParams {
    pub led: String,
    pub on: bool,
}

/// `POST /api/hardware/flashlight` — 开关手电筒 LED。
pub async fn flashlight_control(Json(params): Json<FlashlightParams>) -> Json<Value> {
    match collector::hardware::set_flashlight(&params.led, params.on) {
        Ok(()) => success(serde_json::json!({ "led": params.led, "on": params.on })),
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct BrightnessParams {
    pub percent: u32,
}

/// `POST /api/hardware/brightness` — 设置屏幕亮度（0-100%）。
pub async fn brightness_control(Json(params): Json<BrightnessParams>) -> Json<Value> {
    match collector::hardware::set_brightness(params.percent) {
        Ok(()) => success(serde_json::json!({ "percent": params.percent })),
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct ScreenPowerParams {
    pub on: bool,
}

/// `POST /api/hardware/screen` — 控制屏幕背光开关。
pub async fn screen_power_control(Json(params): Json<ScreenPowerParams>) -> Json<Value> {
    match collector::hardware::set_screen_power(params.on) {
        Ok(()) => success(serde_json::json!({ "screen_on": params.on })),
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct VibrateParams {
    pub duration_ms: u32,
}

/// `POST /api/hardware/vibrate` — 触发一次振动。
pub async fn vibrate_control(Json(params): Json<VibrateParams>) -> Json<Value> {
    match collector::hardware::vibrate_once(params.duration_ms) {
        Ok(ms) => success(serde_json::json!({ "vibrated_ms": ms })),
        Err(e) => error(&e),
    }
}

/// `POST /api/hardware/vibrate/pattern` — 启动振动模式。
pub async fn vibrate_pattern(Json(params): Json<collector::hardware::VibePattern>) -> Json<Value> {
    match collector::hardware::start_pattern(&params) {
        Ok(repeat) => success(serde_json::json!({ "started": true, "repeat": repeat })),
        Err(e) => error(&e),
    }
}

/// `POST /api/hardware/vibrate/stop` — 停止振动。
pub async fn vibrate_stop() -> Json<Value> {
    collector::hardware::stop_vibrate();
    success(serde_json::json!({ "stopped": true }))
}

/// `POST /api/hardware/clear-memory` — 释放页缓存（需 root 权限）。
///
/// 执行 sync + echo 3 > /proc/sys/vm/drop_caches，返回清理前后内存对比。
pub async fn clear_memory() -> Json<Value> {
    let before = read_mem_info();
    
    let sync_ok = std::process::Command::new("sync").status().map(|s| s.success()).unwrap_or(false);
    let drop_ok = std::fs::write("/proc/sys/vm/drop_caches", "3").is_ok();
    
    let after = read_mem_info();
    
    if sync_ok && drop_ok {
        success(serde_json::json!({
            "freed_mb": after.0.saturating_sub(before.0),
            "before": { "free_mb": before.0, "available_mb": before.1 },
            "after": { "free_mb": after.0, "available_mb": after.1 },
        }))
    } else {
        error("清理内存失败: 需要 root 权限")
    }
}

/// 读取 MemFree 和 MemAvailable（MB）。
fn read_mem_info() -> (u64, u64) {
    let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut free_kb = 0u64;
    let mut avail_kb = 0u64;
    for line in content.lines() {
        if line.starts_with("MemFree:") {
            free_kb = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = line.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    (free_kb / 1024, avail_kb / 1024)
}
