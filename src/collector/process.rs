//! 进程信息采集
//!
//! 遍历 `/proc` 下数字目录，读取 status/cmdline 等文件。
//! CPU 使用率通过两次采样 `/proc/<pid>/stat` 的 utime+stime 差分计算。

use super::ProcessInfo;
use std::collections::HashMap;
use std::fs;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Instant;

/// 单个进程的上次 CPU tick 采样，用于差分计算 CPU 使用率。
struct PrevCpu {
    cpu_ticks: u64,
    instant: Instant,
}

/// 全局 PID → 上次采样 映射，跨调用持久化。
static PREV_CPU: LazyLock<Mutex<HashMap<i32, PrevCpu>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static NUM_CPUS: OnceLock<u64> = OnceLock::new();

/// 统计 `/proc` 下数字目录数量（即进程总数）。
pub fn count_processes() -> usize {
    fs::read_dir("/proc")
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()))
                .count()
        })
        .unwrap_or(0)
}

/// 获取逻辑 CPU 核心数。
fn num_cpus() -> u64 {
    *NUM_CPUS.get_or_init(|| {
        fs::read_to_string("/proc/cpuinfo")
            .ok()
            .map(|s| s.lines().filter(|l| l.starts_with("processor")).count() as u64)
            .unwrap_or(1)
            .max(1)
    })
}

/// 从 `/proc/<pid>/stat` 解析 utime + stime（单位：jiffies）。
fn parse_stat_cpu_ticks(pid: i32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    // comm 字段可能含空格/括号，需找到最后一个 ')'
    let after_comm = stat.rfind(')')? + 2; // skip ') '
    let rest = &stat[after_comm..];
    // comm 之后字段：state=0, ppid=1, ... utime=11, stime=12
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// 差分计算进程 CPU 使用率（%，相对单核，已除以核心数归一化）。
fn calc_cpu_usage(pid: i32) -> f32 {
    let ticks = match parse_stat_cpu_ticks(pid) {
        Some(t) => t,
        None => return 0.0,
    };
    let now = Instant::now();
    let mut map = PREV_CPU.lock().unwrap_or_else(|e| e.into_inner());
    let usage = if let Some(prev) = map.get(&pid) {
        let dt = now.duration_since(prev.instant);
        let dt_secs = dt.as_secs_f64();
        if dt_secs > 0.1 {
            let dticks = ticks.saturating_sub(prev.cpu_ticks);
            // sysconf(_SC_CLK_TCK) 在 Linux 上通常为 100
            let ticks_per_sec = 100.0_f64;
            let cpus = num_cpus() as f64;
            ((dticks as f64 / ticks_per_sec) / dt_secs / cpus * 100.0).min(100.0) as f32
        } else {
            0.0
        }
    } else {
        0.0
    };
    map.insert(pid, PrevCpu { cpu_ticks: ticks, instant: now });
    usage
}

/// 列出所有进程，按内存使用量降序排列。
pub fn list_processes() -> Vec<ProcessInfo> {
    let mut processes = Vec::new();

    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }

            let pid: i32 = match name.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let base = format!("/proc/{}", pid);

            let status_content = fs::read_to_string(format!("{}/status", base)).unwrap_or_default();
            let cmdline = fs::read_to_string(format!("{}/cmdline", base))
                .unwrap_or_default()
                .replace('\0', " ");

            let proc_name = get_field(&status_content, "Name").unwrap_or_else(|| {
                if cmdline.is_empty() {
                    "unknown".to_string()
                } else {
                    cmdline
                        .split_whitespace()
                        .next()
                        .unwrap_or("unknown")
                        .to_string()
                }
            });

            let status = get_field(&status_content, "State").unwrap_or_else(|| "unknown".to_string());
            let ppid = get_field(&status_content, "PPid")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let threads = get_field(&status_content, "Threads")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let vmrss = get_field(&status_content, "VmRSS")
                .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
                .unwrap_or(0);

            let cpu_usage = calc_cpu_usage(pid);

            processes.push(ProcessInfo {
                pid,
                name: proc_name,
                status,
                cpu_usage,
                memory_mb: vmrss / 1024,
                ppid,
                threads,
            });
        }
    }

    // 清理已退出进程的缓存条目
    {
        let mut map = PREV_CPU.lock().unwrap_or_else(|e| e.into_inner());
        let live: std::collections::HashSet<i32> = processes.iter().map(|p| p.pid).collect();
        map.retain(|pid, _| live.contains(pid));
    }

    processes.sort_by(|a, b| b.memory_mb.cmp(&a.memory_mb));
    processes
}

/// 获取单个进程的详细信息（status 原文、cmdline、环境变量、cwd、exe）。
pub fn get_process_detail(pid: i32) -> Option<serde_json::Value> {
    let base = format!("/proc/{}", pid);
    let status = fs::read_to_string(format!("{}/status", base)).ok()?;
    let cmdline = fs::read_to_string(format!("{}/cmdline", base))
        .unwrap_or_default()
        .replace('\0', " ");
    let environ = fs::read_to_string(format!("{}/environ", base))
        .unwrap_or_default()
        .replace('\0', "\n");
    let cwd = fs::read_link(format!("{}/cwd", base))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let exe = fs::read_link(format!("{}/exe", base))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    Some(serde_json::json!({
        "pid": pid,
        "status_raw": status,
        "cmdline": cmdline,
        "environ": environ,
        "cwd": cwd,
        "exe": exe,
    }))
}

/// 从 `/proc/<pid>/status` 中按 key 提取冒号后的值。
fn get_field(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        if line.starts_with(key) {
            return Some(line.split(':').nth(1)?.trim().to_string());
        }
    }
    None
}
