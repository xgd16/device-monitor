//! 内存指标采集
//!
//! 解析 `/proc/meminfo` 获取物理内存与 Swap 使用情况。

use super::MemoryInfo;
use std::fs;

/// 采集内存与 Swap 使用信息。
pub fn collect() -> MemoryInfo {
    let info = fs::read_to_string("/proc/meminfo").unwrap_or_default();

    // 从 meminfo 中按 key 提取 kB 数值
    let get = |key: &str| -> u64 {
        info.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };

    let total = get("MemTotal:");
    let free = get("MemFree:");
    let available = get("MemAvailable:");
    let buffers = get("Buffers:");
    let cached = get("Cached:");
    let swap_total = get("SwapTotal:");
    let swap_free = get("SwapFree:");

    // used = total - free - buffers - cached（不含缓存的可回收部分）
    let used = total - free - buffers - cached;
    let usage_percent = if total > 0 { (used as f64 / total as f64) * 100.0 } else { 0.0 };

    MemoryInfo {
        total_mb: total / 1024,
        used_mb: used / 1024,
        free_mb: free / 1024,
        available_mb: available / 1024,
        swap_total_mb: swap_total / 1024,
        swap_used_mb: (swap_total - swap_free) / 1024,
        usage_percent,
    }
}
