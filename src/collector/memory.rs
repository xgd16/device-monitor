//! 内存指标采集
//!
//! 解析 `/proc/meminfo` 获取物理内存与 Swap 使用情况。

use super::MemoryInfo;
use std::fs;

/// 采集内存与 Swap 使用信息。
pub fn collect() -> MemoryInfo {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();

    let mut total = 0;
    let mut free = 0;
    let mut available = 0;
    let mut buffers = 0;
    let mut cached = 0;
    let mut swap_total = 0;
    let mut swap_free = 0;

    // 单次扫描 meminfo，避免为每个字段重复遍历整份文件。
    for line in info.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let value = parts
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        match key {
            "MemTotal:" => total = value,
            "MemFree:" => free = value,
            "MemAvailable:" => available = value,
            "Buffers:" => buffers = value,
            "Cached:" => cached = value,
            "SwapTotal:" => swap_total = value,
            "SwapFree:" => swap_free = value,
            _ => {}
        }
    }

    // used = total - free - buffers - cached（不含缓存的可回收部分）
    let used = total.saturating_sub(free + buffers + cached);
    let usage_percent = if total > 0 { (used as f64 / total as f64) * 100.0 } else { 0.0 };

    MemoryInfo {
        total_mb: total / 1024,
        used_mb: used / 1024,
        free_mb: free / 1024,
        available_mb: available / 1024,
        swap_total_mb: swap_total / 1024,
        swap_used_mb: swap_total.saturating_sub(swap_free) / 1024,
        usage_percent,
    }
}
