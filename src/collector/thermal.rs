//! 温度传感器采集
//!
//! 遍历 `/sys/class/thermal/thermal_zone0..29`，读取 type 和 temp 节点。
//! temp 单位为 millidegree Celsius，需除以 1000。

use super::ThermalZone;
use std::fs;

/// 采集所有可用的 thermal zone。
pub fn collect() -> Vec<ThermalZone> {
    let mut zones = Vec::new();

    for i in 0..30 {
        let base = format!("/sys/class/thermal/thermal_zone{}", i);
        let type_path = format!("{}/type", base);
        let temp_path = format!("{}/temp", base);

        if let (Ok(name), Ok(temp_str)) = (fs::read_to_string(&type_path), fs::read_to_string(&temp_path)) {
            if let Ok(temp_raw) = temp_str.trim().parse::<f64>() {
                zones.push(ThermalZone {
                    id: i,
                    name: name.trim().to_string(),
                    temp_celsius: temp_raw / 1000.0,
                });
            }
        }
    }

    zones
}
