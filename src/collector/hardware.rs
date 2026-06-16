//! 硬件控制模块
//!
//! 通过 sysfs 节点控制设备硬件：
//! - 手电筒 LED（white/yellow）、状态灯（white:status）
//! - 屏幕亮度与背光开关
//! - 振动马达（调用外部 `vibrate` 命令或 ioctl）
//! - 充电电流上限、GPU 频率上限、WiFi 省电模式
//!
//! 路径针对高通平台定制（backlight: ae94000.dsi.0）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;

/// 硬件当前状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareState {
    pub flashlight: FlashlightState,
    pub status_led: StatusLedState,
    pub cpu_status_led_link: CpuStatusLedLinkState,
    pub brightness: BrightnessState,
    pub screen_on: bool,
    pub vibrating: bool,
    pub charging: ChargingState,
    pub gpu: GpuState,
    pub wifi_power_save: WifiPowerSaveState,
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

/// 通知/状态 LED（white:status）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusLedState {
    pub on: bool,
    pub brightness: u32,
    pub max_brightness: u32,
    pub percent: u32,
}

/// CPU 使用率与状态 LED 亮度联动。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuStatusLedLinkState {
    pub enabled: bool,
}

/// 充电电流限制（pmi8998-charger current_max，单位 µA）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargingState {
    pub current_max_ua: u32,
    pub charger_online: bool,
}

/// Adreno GPU devfreq 状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuState {
    pub cur_freq_mhz: u32,
    pub min_freq_mhz: u32,
    pub max_freq_mhz: u32,
    pub governor: String,
    pub available_freqs_mhz: Vec<u32>,
}

/// WiFi 省电模式（iw dev wlan0 get/set power_save）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiPowerSaveState {
    pub enabled: bool,
    pub iface: String,
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
const STATUS_LED: &str = "/sys/class/leds/white:status/brightness";
const STATUS_LED_MAX: &str = "/sys/class/leds/white:status/max_brightness";
const CHARGER_CURRENT_MAX: &str = "/sys/class/power_supply/pmi8998-charger/current_max";
const CHARGER_ONLINE: &str = "/sys/class/power_supply/pmi8998-charger/online";
const GPU_DEVFREQ: &str = "/sys/class/devfreq/5000000.gpu";
const WIFI_IFACE: &str = "wlan0";

/// 振动是否正在进行（供状态查询）
static VIBRATING: AtomicBool = AtomicBool::new(false);
/// CPU 使用率是否联动状态 LED 亮度。
static CPU_STATUS_LED_LINK: AtomicBool = AtomicBool::new(false);
/// 振动模式的后台线程句柄
static VIBRATE_HANDLE: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

fn read_sysfs(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

fn write_sysfs(path: &str, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|e| format!("write {} failed: {}", path, e))
}

fn command_path(candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|path| path.contains('/') && Path::new(path).exists())
        .or_else(|| candidates.last())
        .unwrap_or(&"")
        .to_string()
}

fn iw_cmd() -> String {
    command_path(&["/usr/sbin/iw", "/sbin/iw", "/usr/bin/iw", "/bin/iw", "iw"])
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

    let status_b: u32 = read_sysfs(STATUS_LED).parse().unwrap_or(0);
    let status_max: u32 = read_sysfs(STATUS_LED_MAX).parse().unwrap_or(511);
    let status_pct = if status_max > 0 {
        (status_b as f64 / status_max as f64 * 100.0) as u32
    } else {
        0
    };

    HardwareState {
        flashlight: FlashlightState {
            white_on: white_b > 0,
            white_brightness: white_b,
            yellow_on: yellow_b > 0,
            yellow_brightness: yellow_b,
            max_brightness: flash_max,
        },
        status_led: StatusLedState {
            on: status_b > 0,
            brightness: status_b,
            max_brightness: status_max,
            percent: status_pct,
        },
        cpu_status_led_link: CpuStatusLedLinkState {
            enabled: CPU_STATUS_LED_LINK.load(Ordering::Relaxed),
        },
        brightness: BrightnessState {
            current: brightness,
            max: brightness_max,
            percent: brightness_pct,
        },
        screen_on,
        vibrating: VIBRATING.load(Ordering::Relaxed),
        charging: read_charging_state(),
        gpu: read_gpu_state(),
        wifi_power_save: read_wifi_power_save(),
    }
}

fn read_charging_state() -> ChargingState {
    let current_max_ua: u32 = read_sysfs(CHARGER_CURRENT_MAX).parse().unwrap_or(0);
    let charger_online = read_sysfs(CHARGER_ONLINE) == "1";
    ChargingState {
        current_max_ua,
        charger_online,
    }
}

fn hz_to_mhz(hz: u64) -> u32 {
    (hz / 1_000_000) as u32
}

fn read_gpu_state() -> GpuState {
    let cur = read_sysfs(&format!("{}/cur_freq", GPU_DEVFREQ))
        .parse::<u64>()
        .unwrap_or(0);
    let min = read_sysfs(&format!("{}/min_freq", GPU_DEVFREQ))
        .parse::<u64>()
        .unwrap_or(0);
    let max = read_sysfs(&format!("{}/max_freq", GPU_DEVFREQ))
        .parse::<u64>()
        .unwrap_or(0);
    let governor = read_sysfs(&format!("{}/governor", GPU_DEVFREQ));
    let available_freqs_mhz = read_sysfs(&format!("{}/available_frequencies", GPU_DEVFREQ))
        .split_whitespace()
        .filter_map(|v| v.parse::<u64>().ok())
        .map(hz_to_mhz)
        .collect();

    GpuState {
        cur_freq_mhz: hz_to_mhz(cur),
        min_freq_mhz: hz_to_mhz(min),
        max_freq_mhz: hz_to_mhz(max),
        governor,
        available_freqs_mhz,
    }
}

fn read_wifi_power_save() -> WifiPowerSaveState {
    let enabled = match std::process::Command::new(iw_cmd())
        .args(["dev", WIFI_IFACE, "get", "power_save"])
        .output()
    {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).to_lowercase().contains("on")
        }
        _ => false,
    };
    WifiPowerSaveState {
        enabled,
        iface: WIFI_IFACE.to_string(),
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

/// 开关状态 LED（white:status）。
pub fn set_status_led(on: bool) -> Result<(), String> {
    CPU_STATUS_LED_LINK.store(false, Ordering::Relaxed);
    let max: u32 = read_sysfs(STATUS_LED_MAX).parse().unwrap_or(511);
    let val = if on { max.to_string() } else { "0".to_string() };
    write_sysfs(STATUS_LED, &val)
}

/// 设置状态 LED 亮度（0-100%）。
pub fn set_status_led_brightness(percent: u32) -> Result<(), String> {
    CPU_STATUS_LED_LINK.store(false, Ordering::Relaxed);
    set_status_led_brightness_raw(percent)
}

fn set_status_led_brightness_raw(percent: u32) -> Result<(), String> {
    let max: u32 = read_sysfs(STATUS_LED_MAX).parse().unwrap_or(511);
    let val = ((percent.min(100) as f64 / 100.0) * max as f64) as u32;
    write_sysfs(STATUS_LED, &val.to_string())
}

/// 开关 CPU 使用率联动状态 LED。
pub fn set_cpu_status_led_link(enabled: bool) -> Result<bool, String> {
    CPU_STATUS_LED_LINK.store(enabled, Ordering::Relaxed);
    if !enabled {
        set_status_led_brightness_raw(0)?;
    }
    Ok(enabled)
}

/// 后台采集时调用：开启联动后按 CPU 使用率线性映射到 LED 亮度。
pub fn apply_cpu_status_led_link(cpu_usage: f64) -> Result<Option<u32>, String> {
    if !CPU_STATUS_LED_LINK.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let percent = cpu_usage.clamp(0.0, 100.0).round() as u32;
    set_status_led_brightness_raw(percent)?;
    Ok(Some(percent))
}

/// 设置充电电流上限（µA）。0 表示不限流（由驱动默认处理）。
pub fn set_charge_current_max(microamps: u32) -> Result<u32, String> {
    write_sysfs(CHARGER_CURRENT_MAX, &microamps.to_string())?;
    Ok(microamps)
}

/// 设置 GPU 最大频率（MHz），会写入 devfreq max_freq。
pub fn set_gpu_max_freq_mhz(max_mhz: u32) -> Result<u32, String> {
    let available: Vec<u64> = read_sysfs(&format!("{}/available_frequencies", GPU_DEVFREQ))
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    if available.is_empty() {
        return Err("GPU frequencies unavailable".to_string());
    }

    let target_hz = (max_mhz as u64) * 1_000_000;
    let chosen = available
        .iter()
        .copied()
        .filter(|&hz| hz <= target_hz)
        .max()
        .or_else(|| available.iter().copied().min())
        .ok_or_else(|| "no GPU frequency".to_string())?;

    write_sysfs(
        &format!("{}/max_freq", GPU_DEVFREQ),
        &chosen.to_string(),
    )?;
    Ok(hz_to_mhz(chosen))
}

/// 设置 WiFi 省电模式。
pub fn set_wifi_power_save(enabled: bool) -> Result<bool, String> {
    let arg = if enabled { "on" } else { "off" };
    let output = std::process::Command::new(iw_cmd())
        .args(["dev", WIFI_IFACE, "set", "power_save", arg])
        .output()
        .map_err(|e| format!("iw failed: {}", e))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(enabled)
}
