//! 终端 UI（TUI）模块
//!
//! 在物理 TTY 设备（默认 `/dev/tty1`）上渲染系统监控仪表盘。
//! 通过 `tokio::sync::watch` 订阅后台采集器推送的 `SystemOverview`，
//! 每次数据更新时清屏并重绘整屏内容。
//!
//! 启动方式：`device-monitor --tui [--tty /dev/ttyN]`

use std::io::Write;
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use std::collections::HashMap;
use crate::collector;

/// 网络接口的上一次采样快照，用于计算实时速率。
///
/// 以接口名称为键存入 `HashMap`，每次采集后更新。
/// 速率 = (当前累计字节 − 上次累计字节) / (当前时间戳 − 上次时间戳)。
struct NetPrev {
    /// 上次采样时的接收字节数（累计值，来自 `/sys/class/net/*/statistics/rx_bytes`）
    rx: u64,
    /// 上次采样时的发送字节数（累计值）
    tx: u64,
    /// 上次采样时的 Unix 时间戳（秒）
    ts: i64,
}

/// TUI 主循环入口。
///
/// 订阅 `watch::Receiver` 中的系统概览数据，每当采集器推送新快照时：
/// 1. 根据前后两次快照计算各网卡的实时上下行速率
/// 2. 将完整仪表盘写入指定 TTY 设备
///
/// 当 TTY 写入失败或 watch 通道关闭时退出循环。
///
/// # 参数
/// - `rx`: 系统概览数据的 watch 接收端（与后台 5 秒采集任务共享同一 channel）
/// - `tty_path`: 目标 TTY 设备路径，如 `/dev/tty1`
pub async fn run_tui(rx: &mut watch::Receiver<collector::SystemOverview>, tty_path: &str) -> std::io::Result<()> {
    // 跨帧持久化的网卡历史状态，用于差分计算速率
    let net_state: Arc<Mutex<HashMap<String, NetPrev>>> = Arc::new(Mutex::new(HashMap::new()));
    loop {
        // borrow_and_update: 读取当前值并标记为"已读"，避免重复触发 changed()
        let o = rx.borrow_and_update().clone();
        let speeds = calc_speeds(&net_state, &o);
        // 渲染失败（如 TTY 不存在）则退出
        if let Err(_) = render(tty_path, &o, &speeds) { break; }
        // 等待下一次采集推送；发送端 drop 时返回 Err，退出循环
        if rx.changed().await.is_err() { break; }
    }
    Ok(())
}

/// 根据当前与上一次快照，计算各网络接口的实时收发速率（字节/秒）。
///
/// 首次出现的接口没有历史数据，不产生速率条目（显示为 0）。
/// 每次调用后都会更新内部状态，供下一帧使用。
fn calc_speeds(state: &Arc<Mutex<HashMap<String, NetPrev>>>, o: &collector::SystemOverview) -> HashMap<String, (f64, f64)> {
    let mut map = state.lock().unwrap();
    let mut speeds = HashMap::new();
    for n in &o.network {
        let key = n.name.clone();
        let ts = o.timestamp;
        if let Some(prev) = map.get(&key) {
            let dt = (ts - prev.ts) as f64;
            if dt > 0.0 {
                // saturating_sub 防止计数器回绕导致负数
                let rx_s = (n.rx_bytes.saturating_sub(prev.rx)) as f64 / dt;
                let tx_s = (n.tx_bytes.saturating_sub(prev.tx)) as f64 / dt;
                speeds.insert(key.clone(), (rx_s, tx_s));
            }
        }
        // 无论是否计算出速率，都更新快照供下一帧差分
        map.insert(key.clone(), NetPrev { rx: n.rx_bytes, tx: n.tx_bytes, ts });
    }
    speeds
}

/// 将字节/秒格式化为人类可读字符串（B/s、KB/s、MB/s）。
fn fmt_speed(bps: f64) -> String {
    if bps >= 1048576.0 { format!("{:.1}MB/s", bps / 1048576.0) }
    else if bps >= 1024.0 { format!("{:.1}KB/s", bps / 1024.0) }
    else { format!("{:.0}B/s", bps) }
}

/// 生成 ASCII 进度条，如 `[====....]`。
///
/// # 参数
/// - `pct`: 百分比（0–100）
/// - `w`: 条内字符总宽度
///
/// 填充字符随负载变化：`>`80% 用 `=`，>50% 用 `~`，否则用 `-`。
fn bar(pct: f64, w: usize) -> String {
    let filled = (pct / 100.0 * w as f64).round() as usize;
    let empty = w.saturating_sub(filled);
    let ch = if pct > 80.0 { "=" } else if pct > 50.0 { "~" } else { "-" };
    format!("[{}{}]", ch.repeat(filled), ".".repeat(empty))
}

/// 根据百分比返回 ANSI 前景色代码：红(31) / 黄(33) / 绿(32)。
fn clr(p: f64) -> u8 { if p > 80.0 { 31 } else if p > 60.0 { 33 } else { 32 } }

/// 将完整仪表盘渲染到 TTY 设备。
///
/// 使用 ANSI 转义序列清屏（`\x1b[2J`）并光标归位（`\x1b[H`），
/// 然后逐段拼接 CPU、内存、各核、电池、温度、网络、磁盘等信息。
fn render(tty_path: &str, o: &collector::SystemOverview, speeds: &HashMap<String, (f64, f64)>) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).open(tty_path)?;
    let mut out = String::with_capacity(8192);

    // 清屏 + 光标移到左上角；额外换行避免覆盖 TTY 登录提示行
    out.push_str("\x1b[2J\x1b[H");
    out.push_str("\r\n\r\n\r\n");

    // ── 标题栏 ──
    let ts = chrono::Local::now().format("%H:%M:%S");
    out.push_str(&format!("\x1b[36;1m  Device Monitor v0.2 | {}\x1b[0m\r\n\r\n", ts));

    // ── CPU 总使用率 + 负载均值 ──
    let cpu = o.cpu.overall_usage as f64;
    out.push_str(&format!("\x1b[{}m  CPU {} {:.1}%\x1b[0m\r\n", clr(cpu), bar(cpu, 80), cpu));
    // load_avg: 1/5/15 分钟平均负载；cores.len() 为逻辑核心数
    out.push_str(&format!("  Load {:.2} / {:.2} / {:.2} | {} cores\r\n\r\n", o.load_avg[0], o.load_avg[1], o.load_avg[2], o.cpu.cores.len()));

    // ── 内存 + Swap ──
    let mem = o.memory.usage_percent;
    out.push_str(&format!("\x1b[{}m  MEM {} {:.1}%\x1b[0m\r\n", clr(mem), bar(mem, 80), mem));
    out.push_str(&format!("  {}/{} MB  Swap {}/{} MB\r\n\r\n", o.memory.used_mb, o.memory.total_mb, o.memory.swap_used_mb, o.memory.swap_total_mb));

    // ── 各 CPU 核心详情 ──
    // 从 thermal 列表中提取 cpuN-thermal 区域，映射为核心 ID → 温度
    let tm: HashMap<usize, f64> = o.thermal.iter().filter_map(|t| {
        let n = &t.name;
        if n.starts_with("cpu") && n.ends_with("-thermal") {
            // "cpu0-thermal" → 核心 ID 0
            n[3..n.len()-8].parse::<usize>().ok().map(|id| (id, t.temp_celsius))
        } else { None }
    }).collect();

    out.push_str(&format!("\x1b[1m  Per-Core ({} cores):\x1b[0m\r\n", o.cpu.cores.len()));
    for c in &o.cpu.cores {
        let u = c.usage as f64;
        let temp = tm.get(&c.id).map(|t| format!("{}C", *t as u32)).unwrap_or("---".into());
        out.push_str(&format!("  \x1b[{}mC{} {} {:>5.1}% {:>4}MHz {:>4}\x1b[0m\r\n",
            clr(u), c.id, bar(u, 60), u, c.frequency_mhz, temp));
    }
    out.push_str("\r\n");

    // ── 电池状态 ──
    let b = &o.battery;
    // 电量低于 20% 红色，低于 50% 黄色，否则绿色
    let bc = if b.capacity < 20 { 31 } else if b.capacity < 50 { 33 } else { 32 };
    let bs = match b.status.as_str() { "Charging" => "CHG", "Discharging" => "BAT", "Full" => "FULL", _ => &b.status };
    let bf = (b.capacity as f64 / 100.0 * 80.0).round() as usize;
    let be = 80 - bf;
    // time_left_min: 正数=剩余可用分钟，负数=距充满分钟，0=未知
    let time = if b.time_left_min > 0 { format!("Left {}h{}m", b.time_left_min/60, b.time_left_min%60) }
        else if b.time_left_min < 0 { let a = b.time_left_min.unsigned_abs(); format!("Full {}h{}m", a/60, a%60) }
        else { String::new() };
    out.push_str(&format!("\x1b[{}m  BAT [{}{}] {}% {} {:.1}V {:.0}mA {}C {}\x1b[0m\r\n\r\n",
        bc, "#".repeat(bf), ".".repeat(be), b.capacity, bs, b.voltage_v, b.current_ma, b.temp_celsius as u32, time));

    // ── 温度传感器（按温度降序，显示前 8 个）──
    let mut thermals: Vec<_> = o.thermal.iter().collect();
    thermals.sort_by(|a, b| b.temp_celsius.partial_cmp(&a.temp_celsius).unwrap());
    out.push_str("\x1b[1m  Thermal (top 8):\x1b[0m\r\n  ");
    for (i, t) in thermals.iter().take(8).enumerate() {
        if i > 0 { out.push_str(" | "); }
        out.push_str(&format!("\x1b[{}m{}:{:.0}C\x1b[0m", clr(t.temp_celsius), t.name.replace("-thermal", ""), t.temp_celsius));
    }
    out.push_str("\r\n\r\n");

    // ── 网络接口（仅显示 is_up 的接口）──
    out.push_str("\x1b[1m  Network:\x1b[0m\r\n");
    for n in o.network.iter().filter(|n| n.is_up) {
        let ip = n.ip_addresses.first().map(|s| s.as_str()).unwrap_or("?");
        let rx_mb = n.rx_bytes as f64 / 1048576.0;
        let tx_mb = n.tx_bytes as f64 / 1048576.0;
        let (rx_s, tx_s) = speeds.get(&n.name).copied().unwrap_or((0.0, 0.0));
        out.push_str(&format!("\x1b[32m    {}: {}\x1b[0m\r\n", n.name, ip));
        // UL=上传(TX)，DL=下载(RX)；括号内为累计流量
        out.push_str(&format!("      \x1b[36mUL: {} ({:.1}MB)\x1b[0m  \x1b[33mDL: {} ({:.1}MB)\x1b[0m\r\n",
            fmt_speed(tx_s), tx_mb, fmt_speed(rx_s), rx_mb));
    }
    out.push_str("\r\n");

    // ── 磁盘分区（调用 df -h）──
    out.push_str("\x1b[1m  Disk:\x1b[0m\r\n");
    if let Ok(output) = std::process::Command::new("df").args(["-h"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            // df 输出格式: Filesystem Size Used Avail Use% MountedOn
            if p.len() >= 6 && p[0].starts_with("/dev/") {
                out.push_str(&format!("\x1b[33m    {} {} {}/{} ({}) free:{}\x1b[0m\r\n",
                    p[5], p[0], p[2], p[1], p[4], p[3]));
            }
        }
    }

    // ── 磁盘 I/O 统计（读取 /proc/diskstats）──
    // 仅展示 loop0 和 sda，字段含义见 Linux 文档 Documentation/admin-guide/iostats.rst
    if let Ok(content) = std::fs::read_to_string("/proc/diskstats") {
        for line in content.lines() {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() >= 14 && (p[2] == "loop0" || p[2] == "sda") {
                let reads: u64 = p[5].parse().unwrap_or(0);   // 读完成次数
                let writes: u64 = p[9].parse().unwrap_or(0);  // 写完成次数
                let io_ms: u64 = p[12].parse().unwrap_or(0);  // 累计 I/O 耗时(ms)
                // 每次读写默认 512 字节扇区
                out.push_str(&format!("\x1b[36m    {} R {:.1}MB W {:.1}MB IO:{}ms\x1b[0m\r\n",
                    p[2], reads as f64 * 512.0 / 1048576.0, writes as f64 * 512.0 / 1048576.0, io_ms));
            }
        }
    }
    out.push_str("\r\n");

    // ── 系统运行时间与进程数 ──
    let s = o.uptime as u64;
    let (d,h,m) = (s/86400,(s%86400)/3600,(s%3600)/60);
    let up = if d>0 {format!("{}d{}h{}m",d,h,m)} else if h>0 {format!("{}h{}m",h,m)} else {format!("{}m",m)};
    out.push_str(&format!("  Uptime: {} | Processes: {}\r\n", up, o.process_count));

    write!(f, "{}", out)?;
    f.flush()?;
    Ok(())
}
