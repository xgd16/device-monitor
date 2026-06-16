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

/// 采集电池容量、状态、电压、电流、温度及预估时间。
pub fn collect() -> BatteryInfo {
    let capacity: u8 = read_power_supply("capacity").parse().unwrap_or(0);
    let raw_status = read_power_supply("status");
    let voltage = read_power_supply_f64("voltage_now") / 1_000_000.0; // μV → V
    let current_ua = read_power_supply_f64("current_now");
    let current = current_ua / 1_000_000.0; // μA → A
    let temp = read_power_supply_f64("temp") / 10.0; // 0.1°C → °C

    let usb_online = is_external_power_online();
    let status = normalize_status(&raw_status, current_ua, usb_online);

    // charge_full_design: μAh, current_now: μA (= μAh/h)
    let charge_full_design = read_power_supply_f64("charge_full_design");
    let current_magnitude_ua = current_ua.abs();

    let time_left_min = if current_magnitude_ua < 1000.0 || charge_full_design <= 0.0 {
        // 电流太小 (< 1mA) 或无设计容量，无法估算
        0
    } else if status == "Discharging" {
        // 剩余 = (capacity% × 设计容量) / 电流 × 60 分钟
        let charge_now = charge_full_design * (capacity as f64 / 100.0);
        ((charge_now / current_magnitude_ua) * 60.0).max(0.0) as i64
    } else if status == "Charging" {
        // 充满 = ((100 - capacity%) × 设计容量) / 电流 × 60 分钟（负数表示距充满）
        let charge_needed = charge_full_design * ((100 - capacity) as f64 / 100.0);
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
}
