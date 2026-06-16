//! CPU 指标采集
//!
//! 通过两次 `/proc/stat` 采样差分计算 CPU 使用率。
//! idle 时间 = user+nice+system+idle+iowait+irq+softirq+steal 中的 idle+iowait。

use super::{CpuCore, CpuInfo};
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
    let mut lines = stat.lines();

    // 第一行 "cpu ..." 为所有核心的汇总
    let overall = parse_cpu_line(lines.next().unwrap_or(""));

    let freqs = read_frequencies();
    let mut cores = Vec::new();
    let mut core_samples = Vec::new();
    for line in lines {
        // cpu0, cpu1, ... 各核心行（排除汇总行 "cpu "）
        if line.starts_with("cpu") && !line.starts_with("cpu ") {
            let (idle, total) = parse_cpu_line(line);
            core_samples.push((idle, total));
        }
    }

    let mut state = STATE.lock().unwrap();
    let overall_usage = calc_usage(&mut state, overall.0, overall.1);
    if state.prev_core_idle.len() < core_samples.len() {
        state.prev_core_idle.resize(core_samples.len(), 0);
        state.prev_core_total.resize(core_samples.len(), 0);
    }

    for (idx, (idle, total)) in core_samples.into_iter().enumerate() {
        let usage = calc_core_usage(&mut state, idx, idle, total);
        let freq = freqs.get(idx).copied().unwrap_or(0);
        cores.push(CpuCore {
            id: idx,
            usage,
            frequency_mhz: freq,
        });
    }

    CpuInfo { overall_usage, cores }
}

/// 解析 `/proc/stat` 一行，返回 (idle+jiffies, total_jiffies)。
fn parse_cpu_line(line: &str) -> (u64, u64) {
    let mut idle = 0;
    let mut total = 0;
    for (idx, value) in line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse::<u64>().ok())
        .enumerate()
    {
        if idx == 3 || idx == 4 {
            idle += value;
        }
        total += value;
    }
    (idle, total)
}

/// 计算总体 CPU 使用率并更新全局缓存。
fn calc_usage(state: &mut CpuState, idle: u64, total: u64) -> f32 {
    let d_idle = idle.saturating_sub(state.prev_idle);
    let d_total = total.saturating_sub(state.prev_total);
    state.prev_idle = idle;
    state.prev_total = total;
    if d_total == 0 { 0.0 } else { ((d_total - d_idle) as f32 / d_total as f32) * 100.0 }
}

/// 计算单个核心的 CPU 使用率。
fn calc_core_usage(state: &mut CpuState, idx: usize, idle: u64, total: u64) -> f32 {
    let d_idle = idle.saturating_sub(state.prev_core_idle[idx]);
    let d_total = total.saturating_sub(state.prev_core_total[idx]);
    state.prev_core_idle[idx] = idle;
    state.prev_core_total[idx] = total;
    if d_total == 0 { 0.0 } else { ((d_total - d_idle) as f32 / d_total as f32) * 100.0 }
}

/// 读取各核心当前频率（kHz → MHz）。
fn read_frequencies() -> Vec<u64> {
    let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") else {
        return Vec::new();
    };

    let mut freqs: Vec<(usize, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let idx = name.strip_prefix("cpu")?.parse::<usize>().ok()?;
            let path = format!(
                "/sys/devices/system/cpu/{}/cpufreq/scaling_cur_freq",
                name
            );
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|khz| khz / 1000)
                .map(|freq| (idx, freq))
        })
        .collect();

    let Some(max_idx) = freqs.iter().map(|(idx, _)| *idx).max() else {
        return Vec::new();
    };

    let mut by_core = vec![0; max_idx + 1];
    for (idx, freq) in freqs.drain(..) {
        by_core[idx] = freq;
    }
    by_core
}

/// CPU Governor 信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CpuGovernor {
    /// 当前使用的 governor
    pub current: String,
    /// 所有可用的 governor
    pub available: Vec<String>,
    /// 各核心的 governor（通常所有核心相同）
    pub per_core: Vec<CoreGovernor>,
}

/// 单个核心的 governor 信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CoreGovernor {
    pub core_id: usize,
    pub governor: String,
}

/// 读取 CPU governor 信息
pub fn get_governor() -> CpuGovernor {
    // 读取 CPU0 的 governor 作为当前值
    let current = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor")
        .unwrap_or_default()
        .trim()
        .to_string();

    // 读取可用的 governor
    let available_str = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")
        .unwrap_or_default();
    let available: Vec<String> = available_str
        .trim()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    // 读取各核心的 governor
    let mut per_core = Vec::new();
    for i in 0..8 {
        let path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor", i);
        if let Ok(gov) = fs::read_to_string(&path) {
            per_core.push(CoreGovernor {
                core_id: i,
                governor: gov.trim().to_string(),
            });
        }
    }

    CpuGovernor {
        current,
        available,
        per_core,
    }
}

/// 设置 CPU governor
/// 对所有核心设置相同的 governor
pub fn set_governor(governor: &str) -> Result<(), String> {
    // 先验证 governor 是否可用
    let available_str = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_governors")
        .unwrap_or_default();
    let available: Vec<&str> = available_str.trim().split_whitespace().collect();

    if !available.contains(&governor) {
        return Err(format!("Governor '{}' 不可用。可用选项: {:?}", governor, available));
    }

    // 设置所有核心的 governor
    let mut errors = Vec::new();
    for i in 0..8 {
        let path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_governor", i);
        if std::path::Path::new(&path).exists() {
            if let Err(e) = fs::write(&path, governor) {
                errors.push(format!("CPU{}: {}", i, e));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("部分核心设置失败: {}", errors.join(", ")))
    }
}

/// CPU 频率信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CpuFrequency {
    /// 当前最小频率 (kHz)
    pub min_freq: u64,
    /// 当前最大频率 (kHz)
    pub max_freq: u64,
    /// 硬件支持的最小频率 (kHz)
    pub hw_min_freq: u64,
    /// 硬件支持的最大频率 (kHz)
    pub hw_max_freq: u64,
    /// 可用频率列表 (kHz)
    pub available_freqs: Vec<u64>,
}

/// 读取 CPU 频率信息
pub fn get_frequency() -> CpuFrequency {
    let min_freq = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_min_freq")
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    
    let max_freq = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_max_freq")
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    
    let hw_min_freq = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_min_freq")
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    
    let hw_max_freq = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    
    let freqs_str = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_frequencies")
        .unwrap_or_default();
    let available_freqs: Vec<u64> = freqs_str
        .trim()
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();

    CpuFrequency {
        min_freq,
        max_freq,
        hw_min_freq,
        hw_max_freq,
        available_freqs,
    }
}

/// 设置 CPU 最大频率限制
/// 通过 userspace governor + 设置频率实现低频模式
pub fn set_max_frequency_limit(max_freq_khz: u64) -> Result<CpuGovernor, String> {
    // 验证频率是否可用
    let freqs_str = fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_available_frequencies")
        .unwrap_or_default();
    let available_freqs: Vec<u64> = freqs_str
        .trim()
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    
    if !available_freqs.contains(&max_freq_khz) {
        return Err(format!("频率 {} kHz 不可用。可用频率: {:?}", max_freq_khz, available_freqs));
    }

    // 切换到 userspace governor
    set_governor("userspace")?;

    // 设置所有核心的频率
    let mut errors = Vec::new();
    for i in 0..8 {
        let setspeed_path = format!("/sys/devices/system/cpu/cpu{}/cpufreq/scaling_setspeed", i);
        if std::path::Path::new(&setspeed_path).exists() {
            if let Err(e) = fs::write(&setspeed_path, max_freq_khz.to_string()) {
                errors.push(format!("CPU{}: {}", i, e));
            }
        }
    }

    if !errors.is_empty() {
        return Err(format!("部分核心设置频率失败: {}", errors.join(", ")));
    }

    Ok(get_governor())
}

/// 设置低频模式（限制最大频率为最低可用频率）
pub fn set_low_power_mode() -> Result<CpuGovernor, String> {
    let freq_info = get_frequency();
    if let Some(&min_freq) = freq_info.available_freqs.first() {
        set_max_frequency_limit(min_freq)
    } else {
        Err("无法获取可用频率列表".to_string())
    }
}

/// 恢复正常模式（使用 schedutil governor）
pub fn set_normal_mode() -> Result<CpuGovernor, String> {
    set_governor("schedutil")?;
    Ok(get_governor())
}
