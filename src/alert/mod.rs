//! 告警引擎模块
//!
//! 在每次系统采集后检查 CPU 温度、内存使用率、电池电量是否超过阈值，
//! 触发告警写入 SQLite。同类告警有 300 秒冷却期，避免重复刷屏。

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::collector::SystemOverview;
use crate::store::Database;

/// 告警阈值配置（可通过 API 读取/更新，当前更新尚未持久化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    /// CPU 最高温度阈值（°C）
    pub cpu_temp_threshold: f64,
    /// 内存使用率阈值（%）
    pub memory_threshold: f64,
    /// 磁盘使用率阈值（%，当前 check 中未使用）
    pub disk_threshold: f64,
    /// 电池低电量阈值（%）
    pub battery_low_threshold: u8,
    /// 电池温度阈值（°C）。锂电充电时超过 45°C 会明显加速老化，
    /// 内核 pmi8998 自己的 TEMP_ALERT_MAX 就设的 450（45.0°C），照它取齐。
    pub battery_temp_threshold: f64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            cpu_temp_threshold: 70.0,
            memory_threshold: 90.0,
            disk_threshold: 90.0,
            battery_low_threshold: 15,
            battery_temp_threshold: 45.0,
        }
    }
}

/// 告警检测引擎，持有配置与各类型上次告警时间戳。
pub struct AlertEngine {
    db: Arc<Database>,
    config: AlertConfig,
    /// 上次 CPU 温度告警时间（Unix 秒）
    last_cpu_alert: i64,
    /// 上次内存告警时间
    last_mem_alert: i64,
    /// 上次电池告警时间
    last_bat_alert: i64,
    /// 上次触发低电量告警时的电量（用于**边沿触发**；0 = 尚未触发）
    last_bat_level: u8,
    /// 上次电池温度告警时间
    last_bat_temp_alert: i64,
    /// 上次 XTokenHub 告警时间
    last_xth_alert: i64,
}

impl AlertEngine {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            config: AlertConfig::default(),
            last_cpu_alert: 0,
            last_mem_alert: 0,
            last_bat_alert: 0,
            last_bat_level: 0,
            last_bat_temp_alert: 0,
            last_xth_alert: 0,
        }
    }

    /// 对一次采集快照执行告警规则检查。
    pub fn check(&mut self, data: &SystemOverview) {
        let now = data.timestamp;

        // CPU 温度：取所有 thermal zone 的最高温
        if let Some(max_temp) = data.thermal.iter().map(|t| t.temp_celsius).reduce(f64::max) {
            if max_temp >= self.config.cpu_temp_threshold && now - self.last_cpu_alert > 300 {
                let _ = self.db.store_alert(
                    "warning",
                    "CPU 温度过高",
                    &format!("当前最高温度: {:.1}°C (阈值: {:.1}°C)", max_temp, self.config.cpu_temp_threshold),
                );
                self.last_cpu_alert = now;
            }
        }

        // 内存使用率
        if data.memory.usage_percent >= self.config.memory_threshold && now - self.last_mem_alert > 300 {
            let _ = self.db.store_alert(
                "warning",
                "内存使用率过高",
                &format!("当前使用: {:.1}% (阈值: {:.1}%)", data.memory.usage_percent, self.config.memory_threshold),
            );
            self.last_mem_alert = now;
        }

        // 电池低电量：**边沿触发**（跨越阈值、或又掉了 ≥5% 才报）。
        //
        // 旧写法是「低于阈值就每 300s 报一次」，实测在电量卡在 1% 的那一小时里
        // 连报了 13 条一字不差的信息（数据库里同类告警累计 91 条），纯噪音；
        // 而用户恰好是在手机已经撑不住的时候最不需要被刷屏。
        let low = data.battery.capacity <= self.config.battery_low_threshold
            && data.battery.status != "Charging";
        if low {
            let first = self.last_bat_level == 0;
            let dropped = !first && data.battery.capacity + 5 <= self.last_bat_level;
            // 60s 内不重复：电量在阈值附近台阶式下降时避免连报
            let quiet = now - self.last_bat_alert > 60;
            if (first || dropped) && quiet {
                let _ = self.db.store_alert(
                    "warning",
                    "电池电量低",
                    &format!(
                        "当前电量: {}% (阈值: {}%)",
                        data.battery.capacity, self.config.battery_low_threshold
                    ),
                );
                self.last_bat_alert = now;
                self.last_bat_level = data.battery.capacity;
            }
        } else {
            // 充上电 / 回到阈值以上 → 复位，下次再低电还能报
            self.last_bat_level = 0;
        }

        // 电池温度过高（充电时发热最凶，与 CPU 温度分开看）
        if data.battery.temp_celsius >= self.config.battery_temp_threshold
            && now - self.last_bat_temp_alert > 300
        {
            let _ = self.db.store_alert(
                "warning",
                "电池温度过高",
                &format!(
                    "当前 {:.1}°C (阈值 {:.0}°C)，{}",
                    data.battery.temp_celsius,
                    self.config.battery_temp_threshold,
                    if data.battery.status == "Charging" { "充电中" } else { "未充电" },
                ),
            );
            self.last_bat_temp_alert = now;
        }

        // XTokenHub 不可用
        if !data.xtokenhub.available && now - self.last_xth_alert > 300 {
            let _ = self.db.store_alert(
                "warning",
                "XTokenHub 不可用",
                &format!("错误: {}", data.xtokenhub.error),
            );
            self.last_xth_alert = now;
        }
    }
}
