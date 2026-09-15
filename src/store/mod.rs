//! 数据持久化模块
//!
//! 当前实现为 SQLite 后端，存储指标快照与告警记录。

pub mod sqlite;
pub use sqlite::Database;

/// 历史数据默认保留天数。
pub const DEFAULT_RETENTION_DAYS: i64 = 7;

/// 指标默认落库间隔（秒）。
///
/// 实时推送仍是 5 秒一次，落库单独降频：整份 `SystemOverview` JSON 实测约 3.3KB，
/// 5 秒一存时 7 天就有约 11 万行、370MB。报表曲线请求上限 2000 点（默认 500），
/// 降到 30 秒存后仍会被降采样到同样密度，界面看不出差别。
pub const DEFAULT_PERSIST_SECS: u64 = 30;

/// 生效的保留天数，可用 `RETENTION_DAYS` 环境变量覆盖；非法值回落默认。
pub fn retention_days() -> i64 {
    env_secs_or("RETENTION_DAYS", DEFAULT_RETENTION_DAYS as u64) as i64
}

/// 生效的落库间隔（秒），可用 `METRICS_PERSIST_SECS` 环境变量覆盖。
pub fn persist_interval_secs() -> u64 {
    env_secs_or("METRICS_PERSIST_SECS", DEFAULT_PERSIST_SECS)
}

/// 读取正整数环境变量，缺失或非法时回落默认值。
fn env_secs_or(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}
