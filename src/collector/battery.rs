//! 电池指标采集
//!
//! 读取高通平台 `/sys/class/power_supply/qcom-battery/` 下的电源管理节点。
//! 并根据设计容量与当前电流估算剩余/充满时间。

use super::BatteryInfo;
use std::fs;
use std::path::Path;

const BATTERY_SUPPLY: &str = "qcom-battery";
/// 判定为充电的最小电流（μA）
const CHARGE_CURRENT_UA: f64 = 100_000.0;
/// 判定为放电的最小电流（μA，负值）
const DISCHARGE_CURRENT_UA: f64 = -30_000.0;
/// 充电截止判定电流（μA）：低于此值视为已到达实际上限
const CHARGE_TAIL_CURRENT_UA: f64 = 50_000.0;
/// 开始学习实际上限 SOC 的最低电量
const LEARN_MIN_CAPACITY: u8 = 85;
/// 判定电池健康衰减：实际上限低于此值
const DEGRADED_MAX_THRESHOLD: u8 = 98;
const EFFECTIVE_MAX_FILE: &str = "battery_effective_max.txt";

/// 读取指定 power_supply 节点的字符串值。
fn read_supply(supply: &str, field: &str) -> String {
    let path = format!("/sys/class/power_supply/{supply}/{field}");
    fs::read_to_string(&path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 读取 qcom-battery 节点的字符串值。
fn read_power_supply(field: &str) -> String {
    read_supply(BATTERY_SUPPLY, field)
}

/// 读取 qcom-battery 节点的浮点数值。
fn read_power_supply_f64(field: &str) -> f64 {
    read_power_supply(field).parse::<f64>().unwrap_or(0.0)
}

/// 检测 USB / AC / 无线充电器是否在线。
fn is_external_power_online() -> bool {
    let dir = Path::new("/sys/class/power_supply");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == BATTERY_SUPPLY || name == "battery" {
            continue;
        }

        let supply_type = read_supply(&name, "type");
        if !matches!(supply_type.as_str(), "USB" | "Mains" | "Wireless") {
            continue;
        }

        if read_supply(&name, "online") == "1" {
            return true;
        }
    }

    false
}

/// 结合 sysfs status、电流方向与外部电源，修正充电状态。
fn normalize_status(raw_status: &str, current_ua: f64, usb_online: bool) -> String {
    if !usb_online {
        return if raw_status == "Full" {
            "Full".into()
        } else {
            "Discharging".into()
        };
    }

    match raw_status {
        "Full" => "Full".into(),
        "Not charging" => "Not charging".into(),
        "Charging" => {
            if current_ua < CHARGE_CURRENT_UA {
                "Not charging".into()
            } else {
                "Charging".into()
            }
        }
        "Discharging" => "Discharging".into(),
        _ => {
            if current_ua > CHARGE_CURRENT_UA {
                "Charging".into()
            } else if current_ua < DISCHARGE_CURRENT_UA {
                "Discharging".into()
            } else {
                "Not charging".into()
            }
        }
    }
}

fn load_effective_max_pct() -> u8 {
    if let Ok(env) = std::env::var("BATTERY_EFFECTIVE_MAX_PCT") {
        if let Ok(v) = env.parse::<u8>() {
            return v.clamp(50, 100);
        }
    }
    fs::read_to_string(EFFECTIVE_MAX_FILE)
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok())
        .map(|v| v.clamp(50, 100))
        .unwrap_or(100)
}

fn save_effective_max_pct(value: u8) {
    let _ = fs::write(EFFECTIVE_MAX_FILE, format!("{}\n", value.clamp(50, 100)));
}

/// 根据充电截止行为学习实际上限 SOC（老化电池可能到不了 100%）。
fn learn_effective_max_pct(
    capacity: u8,
    usb_online: bool,
    current_ua: f64,
    status: &str,
) -> u8 {
    let mut max_pct = load_effective_max_pct();

    if capacity >= 100 {
        max_pct = 100;
        save_effective_max_pct(100);
        return max_pct;
    }

    let charge_stalled = usb_online
        && capacity >= LEARN_MIN_CAPACITY
        && current_ua.abs() < CHARGE_TAIL_CURRENT_UA;
    let full_like = status == "Full" || status == "Not charging";

    if charge_stalled && full_like && capacity < max_pct {
        max_pct = capacity;
        save_effective_max_pct(max_pct);
    }

    max_pct
}

fn display_capacity_pct(capacity: u8, effective_max_pct: u8) -> u8 {
    if effective_max_pct >= 100 || effective_max_pct == 0 {
        return capacity;
    }
    ((capacity as f64 / effective_max_pct as f64) * 100.0)
        .clamp(0.0, 100.0)
        .round() as u8
}

fn at_charge_limit(capacity: u8, effective_max_pct: u8, usb_online: bool, current_ua: f64) -> bool {
    usb_online
        && capacity >= effective_max_pct
        && current_ua.abs() < CHARGE_CURRENT_UA
}

/// 采集电池容量、状态、电压、电流、温度及预估时间。
pub fn collect() -> BatteryInfo {
    let capacity: u8 = read_power_supply("capacity").parse().unwrap_or(0);
    let raw_status = read_power_supply("status");
    let voltage = read_power_supply_f64("voltage_now") / 1_000_000.0; // μV → V
    let current_ua = read_power_supply_f64("current_now");
    let current = current_ua / 1_000_000.0; // μA → A
    let temp = read_power_supply_f64("temp") / 10.0; // 0.1°C → °C

    let usb_online = is_external_power_online();
    let mut status = normalize_status(&raw_status, current_ua, usb_online);

    let effective_max_pct = learn_effective_max_pct(capacity, usb_online, current_ua, &status);
    let is_degraded = effective_max_pct < DEGRADED_MAX_THRESHOLD;
    let at_limit = at_charge_limit(capacity, effective_max_pct, usb_online, current_ua);
    let display_capacity_pct = display_capacity_pct(capacity, effective_max_pct);

    if at_limit && usb_online {
        status = "Full".into();
    }

    // charge_full_design: μAh, current_now: μA (= μAh/h)
    let charge_full_design = read_power_supply_f64("charge_full_design");
    let current_magnitude_ua = current_ua.abs();

    let time_left_min = if at_limit {
        0
    } else if current_magnitude_ua < 1000.0 || charge_full_design <= 0.0 {
        // 电流太小 (< 1mA) 或无设计容量，无法估算
        0
    } else if status == "Discharging" {
        // 剩余 = (capacity% × 设计容量) / 电流 × 60 分钟
        let charge_now = charge_full_design * (capacity as f64 / 100.0);
        ((charge_now / current_magnitude_ua) * 60.0).max(0.0) as i64
    } else if status == "Charging" {
        // 充满 = ((effective_max - capacity)% × 设计容量) / 电流 × 60 分钟
        let headroom = effective_max_pct.saturating_sub(capacity) as f64;
        let charge_needed = charge_full_design * (headroom / 100.0);
        -((charge_needed / current_magnitude_ua) * 60.0).max(0.0) as i64
    } else {
        0
    };

    BatteryInfo {
        capacity,
        status,
        voltage_v: voltage,
        current_ma: current * 1000.0,
        power_w: voltage * current.abs(),
        temp_celsius: temp,
        time_left_min,
        effective_max_pct,
        display_capacity_pct,
        is_degraded,
        at_charge_limit: at_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unplugged_charging_status_becomes_discharging() {
        assert_eq!(
            normalize_status("Charging", 245_000.0, false),
            "Discharging"
        );
    }

    #[test]
    fn plugged_low_current_is_not_charging() {
        assert_eq!(
            normalize_status("Charging", 50_000.0, true),
            "Not charging"
        );
    }

    #[test]
    fn plugged_high_current_is_charging() {
        assert_eq!(
            normalize_status("Charging", 500_000.0, true),
            "Charging"
        );
    }

    #[test]
    fn unplugged_full_stays_full() {
        assert_eq!(normalize_status("Full", 0.0, false), "Full");
    }

    #[test]
    fn display_capacity_scales_to_effective_max() {
        assert_eq!(display_capacity_pct(97, 97), 100);
        assert_eq!(display_capacity_pct(85, 97), 88);
    }

    #[test]
    fn at_charge_limit_when_plateau_reached() {
        assert!(at_charge_limit(97, 97, true, 20_000.0));
        assert!(!at_charge_limit(96, 97, true, 20_000.0));
    }
}
