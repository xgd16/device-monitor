//! CPU 指标采集
//!
//! 通过两次 `/proc/stat` 采样差分计算 CPU 使用率。
//! idle 时间 = user+nice+system+idle+iowait+irq+softirq+steal 中的 idle+iowait。

use super::{CpuInfo, CpuCore};
use std::fs;
use std::sync::Mutex;

/// 跨采样周期的 CPU 计数器缓存，用于差分计算使用率。
struct CpuState {
    prev_idle: u64,
    prev_total: u64,
    prev_core_idle: Vec<u64>,
    prev_core_total: Vec<u64>,
}

static STATE: Mutex<CpuState> = Mutex::new(CpuState {
    prev_idle: 0,
    prev_total: 0,
    prev_core_idle: Vec::new(),
    prev_core_total: Vec::new(),
});

/// 采集 CPU 总体使用率、各核心使用率及当前频率。
pub fn collect() -> CpuInfo {
    let stat = fs::read_to_string("/proc/stat").unwrap_or_default();
    let lines: Vec<&str> = stat.lines().collect();

    // 第一行 "cpu ..." 为所有核心的汇总
    let overall = parse_cpu_line(lines.first().copied().unwrap_or(""));
    let overall_usage = calc_usage(overall.0, overall.1);

    let freqs = read_frequencies();
    let mut cores = Vec::new();
    let mut idx = 0;
    for line in &lines {
        // cpu0, cpu1, ... 各核心行（排除汇总行 "cpu "）
        if line.starts_with("cpu") && !line.starts_with("cpu ") {
            let (idle, total) = parse_cpu_line(line);
            let usage = calc_core_usage(idx, idle, total);
            let freq = freqs.get(idx).copied().unwrap_or(0);
            cores.push(CpuCore {
                id: idx,
                usage,
                frequency_mhz: freq,
            });
            idx += 1;
        }
    }

    CpuInfo { overall_usage, cores }
}

/// 解析 `/proc/stat` 一行，返回 (idle+jiffies, total_jiffies)。
fn parse_cpu_line(line: &str) -> (u64, u64) {
    let parts: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let idle = parts.get(3).copied().unwrap_or(0) + parts.get(4).copied().unwrap_or(0);
    let total: u64 = parts.iter().sum();
    (idle, total)
}

/// 计算总体 CPU 使用率并更新全局缓存。
fn calc_usage(idle: u64, total: u64) -> f32 {
    let mut state = STATE.lock().unwrap();
    let d_idle = idle.saturating_sub(state.prev_idle);
    let d_total = total.saturating_sub(state.prev_total);
    state.prev_idle = idle;
    state.prev_total = total;
    if d_total == 0 { 0.0 } else { ((d_total - d_idle) as f32 / d_total as f32) * 100.0 }
}

/// 计算单个核心的 CPU 使用率。
fn calc_core_usage(idx: usize, idle: u64, total: u64) -> f32 {
    let mut state = STATE.lock().unwrap();
    if state.prev_core_idle.len() <= idx {
        state.prev_core_idle.resize(idx + 1, 0);
        state.prev_core_total.resize(idx + 1, 0);
    }
    let d_idle = idle.saturating_sub(state.prev_core_idle[idx]);
    let d_total = total.saturating_sub(state.prev_core_total[idx]);
    state.prev_core_idle[idx] = idle;
    state.prev_core_total[idx] = total;
    if d_total == 0 { 0.0 } else { ((d_total - d_idle) as f32 / d_total as f32) * 100.0 }
}

/// 读取各核心当前频率（kHz → MHz），最多探测 8 核。
fn read_frequencies() -> Vec<u64> {
    (0..8)
        .map(|i| {
            let path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_cur_freq", i);
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|khz| khz / 1000)
                .unwrap_or(0)
        })
        .collect()
}
