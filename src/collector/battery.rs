//! 电池指标采集
//!
//! 读取高通平台 `/sys/class/power_supply/qcom-battery/` 下的电源管理节点。
//! 并根据设计容量与当前电流估算剩余/充满时间。

use super::BatteryInfo;
use std::fs;

/// 读取 power_supply 节点的字符串值。
fn read_power_supply(field: &str) -> String {
    let path = format!("/sys/class/power_supply/qcom-battery/{}", field);
    fs::read_to_string(&path).unwrap_or_default().trim().to_string()
}

/// 读取 power_supply 节点的浮点数值。
fn read_power_supply_f64(field: &str) -> f64 {
    read_power_supply(field).parse::<f64>().unwrap_or(0.0)
}

/// 采集电池容量、状态、电压、电流、温度及预估时间。
pub fn collect() -> BatteryInfo {
    let capacity: u8 = read_power_supply("capacity").parse().unwrap_or(0);
    let status = read_power_supply("status");
    let voltage = read_power_supply_f64("voltage_now") / 1_000_000.0;  // μV → V
    let current = read_power_supply_f64("current_now") / 1_000_000.0; // μA → A
    let temp = read_power_supply_f64("temp") / 10.0;                  // 0.1°C → °C

    // charge_full_design: μAh, current_now: μA (= μAh/h)
    let charge_full_design = read_power_supply_f64("charge_full_design");
    let current_ua = read_power_supply_f64("current_now").abs();

    let time_left_min = if current_ua < 1000.0 || charge_full_design <= 0.0 {
        // 电流太小 (< 1mA) 或无设计容量，无法估算
        0
    } else if status == "Discharging" {
        // 剩余 = (capacity% × 设计容量) / 电流 × 60 分钟
        let charge_now = charge_full_design * (capacity as f64 / 100.0);
        ((charge_now / current_ua) * 60.0).max(0.0) as i64
    } else if status == "Charging" {
        // 充满 = ((100 - capacity%) × 设计容量) / 电流 × 60 分钟（负数表示距充满）
        let charge_needed = charge_full_design * ((100 - capacity) as f64 / 100.0);
        -((charge_needed / current_ua) * 60.0).max(0.0) as i64
    } else {
        0
    };

    BatteryInfo {
        capacity,
        status,
        voltage_v: voltage,
        current_ma: current * 1000.0,
        temp_celsius: temp,
        time_left_min,
    }
}
