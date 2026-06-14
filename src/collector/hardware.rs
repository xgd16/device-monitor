//! 硬件控制模块
//!
//! 通过 sysfs 节点控制设备硬件：
//! - 手电筒 LED（white/yellow）
//! - 屏幕亮度与背光开关
//! - 振动马达（调用外部 `vibrate` 命令或 ioctl）
//!
//! 路径针对高通平台定制（backlight: ae94000.dsi.0）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;

/// 硬件当前状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareState {
    pub flashlight: FlashlightState,
    pub brightness: BrightnessState,
    pub screen_on: bool,
    pub vibrating: bool,
}

/// 手电筒双 LED 状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashlightState {
    pub white_on: bool,
    pub white_brightness: u32,
    pub yellow_on: bool,
    pub yellow_brightness: u32,
    pub max_brightness: u32,
}

/// 屏幕背光亮度。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrightnessState {
    pub current: u32,
    pub max: u32,
    pub percent: u32,
}

/// 振动模式中的一个时间段。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VibeSegment {
    pub duration_ms: u32,
    pub strong_pct: u32,
    pub weak_pct: u32,
}

/// 振动模式：多段振动 + 是否循环。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VibePattern {
    pub segments: Vec<VibeSegment>,
    pub repeat: bool,
}

/// 屏幕背光 sysfs 路径前缀
const BACKLIGHT: &str = "/sys/class/backlight/ae94000.dsi.0";

/// 振动是否正在进行（供状态查询）
static VIBRATING: AtomicBool = AtomicBool::new(false);
/// 振动模式的后台线程句柄
static VIBRATE_HANDLE: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

fn read_sysfs(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

fn write_sysfs(path: &str, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|e| format!("write {} failed: {}", path, e))
}

/// 调用外部 vibrate 命令触发一次振动。
fn run_vibrate(duration_ms: u32) -> Result<(), String> {
    std::process::Command::new("vibrate")
        .arg(duration_ms.to_string())
        .output()
        .map_err(|e| format!("vibrate failed: {}", e))
        .and_then(|o| {
            if o.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
            }
        })
}

/// 读取当前硬件状态（LED、亮度、屏幕、振动）。
pub fn get_state() -> HardwareState {
    let white_b: u32 = read_sysfs("/sys/class/leds/white:flash/brightness").parse().unwrap_or(0);
    let yellow_b: u32 = read_sysfs("/sys/class/leds/yellow:flash/brightness").parse().unwrap_or(0);
    let flash_max: u32 = read_sysfs("/sys/class/leds/white:flash/max_brightness").parse().unwrap_or(255);

    let brightness: u32 = read_sysfs(&format!("{}/brightness", BACKLIGHT)).parse().unwrap_or(0);
    let brightness_max: u32 = read_sysfs(&format!("{}/max_brightness", BACKLIGHT)).parse().unwrap_or(2047);
    let brightness_pct = if brightness_max > 0 { (brightness as f64 / brightness_max as f64 * 100.0) as u32 } else { 0 };

    // bl_power: 0=开, 4=关
    let bl_power: u32 = read_sysfs(&format!("{}/bl_power", BACKLIGHT)).parse().unwrap_or(0);
    let screen_on = bl_power == 0;

    HardwareState {
        flashlight: FlashlightState {
            white_on: white_b > 0,
            white_brightness: white_b,
            yellow_on: yellow_b > 0,
            yellow_brightness: yellow_b,
            max_brightness: flash_max,
        },
        brightness: BrightnessState {
            current: brightness,
            max: brightness_max,
            percent: brightness_pct,
        },
        screen_on,
        vibrating: VIBRATING.load(Ordering::Relaxed),
    }
}

/// 开关手电筒 LED（"white" 或 "yellow"）。
pub fn set_flashlight(led: &str, on: bool) -> Result<(), String> {
    let path = match led {
        "white" => "/sys/class/leds/white:flash/brightness",
        "yellow" => "/sys/class/leds/yellow:flash/brightness",
        _ => return Err("invalid LED".to_string()),
    };
    let max: u32 = read_sysfs(&format!("/sys/class/leds/{}:flash/max_brightness", led))
        .parse()
        .unwrap_or(255);
    let val = if on { max.to_string() } else { "0".to_string() };
    write_sysfs(path, &val)
}

/// 设置屏幕亮度（0-100%）。
pub fn set_brightness(percent: u32) -> Result<(), String> {
    let max: u32 = read_sysfs(&format!("{}/max_brightness", BACKLIGHT))
        .parse()
        .unwrap_or(2047);
    let val = ((percent.min(100) as f64 / 100.0) * max as f64) as u32;
    write_sysfs(&format!("{}/brightness", BACKLIGHT), &val.to_string())
}

/// 控制屏幕背光开关。
pub fn set_screen_power(on: bool) -> Result<(), String> {
    let val = if on { "0" } else { "4" };
    write_sysfs(&format!("{}/bl_power", BACKLIGHT), val)
}

/// 触发一次振动，时长限制在 50-3000ms。
pub fn vibrate_once(duration_ms: u32) -> Result<u32, String> {
    stop_vibrate();
    let ms = duration_ms.max(50).min(3000);
    run_vibrate(ms)?;
    Ok(ms)
}

/// 启动振动模式（后台线程循环执行各段振动）。
pub fn start_pattern(pattern: &VibePattern) -> Result<bool, String> {
    stop_vibrate();

    let segments: Vec<u32> = pattern.segments.iter()
        .map(|s| s.duration_ms.max(50).min(3000))
        .collect();
    let repeat = pattern.repeat;

    VIBRATING.store(true, Ordering::Relaxed);

    let handle = thread::spawn(move || {
        loop {
            for &dur in &segments {
                if !VIBRATING.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(e) = run_vibrate(dur) {
                    eprintln!("vibrate error: {}", e);
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
            if !repeat || !VIBRATING.load(Ordering::Relaxed) {
                break;
            }
        }
        VIBRATING.store(false, Ordering::Relaxed);
    });

    let mut h = VIBRATE_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    *h = Some(handle);
    Ok(repeat)
}

/// 停止振动模式并清理后台线程。
pub fn stop_vibrate() {
    VIBRATING.store(false, Ordering::Relaxed);
    let mut h = VIBRATE_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
    h.take();
}
