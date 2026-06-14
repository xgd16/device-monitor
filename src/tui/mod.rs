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
struct NetPrev {
    rx: u64,
    tx: u64,
    ts: i64,
}

/// 块设备的上一次 I/O 采样快照，用于计算实时读写速率。
struct DiskPrev {
    read_sectors: u64,
    write_sectors: u64,
    ts: i64,
}

/// TUI 主循环入口。
pub async fn run_tui(rx: &mut watch::Receiver<collector::SystemOverview>, tty_path: &str) -> std::io::Result<()> {
    let net_state: Arc<Mutex<HashMap<String, NetPrev>>> = Arc::new(Mutex::new(HashMap::new()));
    let disk_state: Arc<Mutex<HashMap<String, DiskPrev>>> = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let o = rx.borrow_and_update().clone();
        let speeds = calc_speeds(&net_state, &o);
        let disk_speeds = calc_disk_speeds(&disk_state, &o);
        if let Err(_) = render(tty_path, &o, &speeds, &disk_speeds) { break; }
        if rx.changed().await.is_err() { break; }
    }
    Ok(())
}

fn calc_speeds(state: &Arc<Mutex<HashMap<String, NetPrev>>>, o: &collector::SystemOverview) -> HashMap<String, (f64, f64)> {
    let mut map = state.lock().unwrap();
    let mut speeds = HashMap::new();
    for n in &o.network {
        let key = n.name.clone();
        let ts = o.timestamp;
        if let Some(prev) = map.get(&key) {
            let dt = (ts - prev.ts) as f64;
            if dt > 0.0 {
                let rx_s = (n.rx_bytes.saturating_sub(prev.rx)) as f64 / dt;
                let tx_s = (n.tx_bytes.saturating_sub(prev.tx)) as f64 / dt;
                speeds.insert(key.clone(), (rx_s, tx_s));
            }
        }
        map.insert(key.clone(), NetPrev { rx: n.rx_bytes, tx: n.tx_bytes, ts });
    }
    speeds
}

fn calc_disk_speeds(
    state: &Arc<Mutex<HashMap<String, DiskPrev>>>,
    o: &collector::SystemOverview,
) -> HashMap<String, (f64, f64)> {
    let mut map = state.lock().unwrap();
    let mut speeds = HashMap::new();
    let ts = o.timestamp;
    for block_dev in collector::disk::list_block_devices(3) {
        let (read_sectors, write_sectors, _) = collector::disk::get_io_stats(&block_dev);
        if let Some(prev) = map.get(&block_dev) {
            let dt = (ts - prev.ts) as f64;
            if dt > 0.0 {
                let read_bps = (read_sectors.saturating_sub(prev.read_sectors)) as f64 * 512.0 / dt;
                let write_bps = (write_sectors.saturating_sub(prev.write_sectors)) as f64 * 512.0 / dt;
                speeds.insert(block_dev.clone(), (read_bps, write_bps));
            }
        }
        map.insert(block_dev.clone(), DiskPrev { read_sectors, write_sectors, ts });
    }
    speeds
}

fn fmt_speed(bps: f64) -> String {
    if bps >= 1048576.0 { format!("{:.1}MB/s", bps / 1048576.0) }
    else if bps >= 1024.0 { format!("{:.1}KB/s", bps / 1024.0) }
    else { format!("{:.0}B/s", bps) }
}

fn fmt_bytes(bytes: f64) -> String {
    if bytes >= 1073741824.0 { format!("{:.1}G", bytes / 1073741824.0) }
    else if bytes >= 1048576.0 { format!("{:.1}M", bytes / 1048576.0) }
    else if bytes >= 1024.0 { format!("{:.1}K", bytes / 1024.0) }
    else { format!("{:.0}B", bytes) }
}

fn bar(pct: f64, w: usize) -> String {
    let filled = (pct / 100.0 * w as f64).round() as usize;
    let empty = w.saturating_sub(filled);
    let ch = if pct > 80.0 { "=" } else if pct > 50.0 { "~" } else { "-" };
    format!("[{}{}]", ch.repeat(filled), ".".repeat(empty))
}

fn clr(p: f64) -> u8 { if p > 80.0 { 31 } else if p > 60.0 { 33 } else { 32 } }

fn read_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn read_kernel() -> String {
    std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn trunc_name(name: &str, max: usize) -> String {
    if name.len() > max {
        format!("{}...", &name[..max.saturating_sub(3)])
    } else {
        name.to_string()
    }
}

fn render_process_row(out: &mut String, p: &collector::ProcessInfo, color_metric: f64) {
    out.push_str(&format!(
        "    \x1b[{}m{:>6} {:>6.1}% {:>7} {:>4}  {}\x1b[0m\r\n",
        clr(color_metric),
        p.pid,
        p.cpu_usage,
        p.memory_mb,
        p.threads,
        trunc_name(&p.name, 20),
    ));
}

fn render(
    tty_path: &str,
    o: &collector::SystemOverview,
    speeds: &HashMap<String, (f64, f64)>,
    disk_speeds: &HashMap<String, (f64, f64)>,
) -> std::io::Result<()> {
    let mut f = OpenOptions::new().write(true).open(tty_path)?;
    let mut out = String::with_capacity(16384);

    out.push_str("\x1b[2J\x1b[H");
    out.push_str("\r\n\r\n\r\n");

    // ── 标题栏 + 系统标识 ──
    let ts = chrono::Local::now().format("%H:%M:%S");
    out.push_str(&format!("\x1b[36;1m  Device Monitor v0.2 | {}\x1b[0m\r\n", ts));
    out.push_str(&format!(
        "  {} | kernel {} | avail {}/{} MB\r\n\r\n",
        read_hostname(),
        read_kernel(),
        o.memory.available_mb,
        o.memory.total_mb,
    ));

    // ── CPU 总使用率 + 负载均值 ──
    let cpu = o.cpu.overall_usage as f64;
    out.push_str(&format!("\x1b[{}m  CPU {} {:.1}%\x1b[0m\r\n", clr(cpu), bar(cpu, 80), cpu));
    out.push_str(&format!("  Load {:.2} / {:.2} / {:.2} | {} cores\r\n\r\n", o.load_avg[0], o.load_avg[1], o.load_avg[2], o.cpu.cores.len()));

    // ── 内存 + Swap ──
    let mem = o.memory.usage_percent;
    out.push_str(&format!("\x1b[{}m  MEM {} {:.1}%\x1b[0m\r\n", clr(mem), bar(mem, 80), mem));
    out.push_str(&format!("  {}/{} MB  Swap {}/{} MB\r\n\r\n", o.memory.used_mb, o.memory.total_mb, o.memory.swap_used_mb, o.memory.swap_total_mb));

    // ── 各 CPU 核心详情 ──
    let tm: HashMap<usize, f64> = o.thermal.iter().filter_map(|t| {
        let n = &t.name;
        if n.starts_with("cpu") && n.ends_with("-thermal") {
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
    let bc = if b.capacity < 20 { 31 } else if b.capacity < 50 { 33 } else { 32 };
    let bs = match b.status.as_str() { "Charging" => "CHG", "Discharging" => "BAT", "Full" => "FULL", _ => &b.status };
    let bf = (b.capacity as f64 / 100.0 * 80.0).round() as usize;
    let be = 80 - bf;
    let time = if b.time_left_min > 0 { format!("Left {}h{}m", b.time_left_min/60, b.time_left_min%60) }
        else if b.time_left_min < 0 { let a = b.time_left_min.unsigned_abs(); format!("Full {}h{}m", a/60, a%60) }
        else { String::new() };
    let power_label = match b.status.as_str() {
        "Charging" => format!("+{:.1}W", b.power_w),
        "Discharging" => format!("-{:.1}W", b.power_w),
        _ => format!("{:.1}W", b.power_w),
    };
    out.push_str(&format!("\x1b[{}m  BAT [{}{}] {}% {} {} {:.1}V {:.0}mA {}C {}\x1b[0m\r\n\r\n",
        bc, "#".repeat(bf), ".".repeat(be), b.capacity, bs, power_label, b.voltage_v, b.current_ma, b.temp_celsius as u32, time));

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
        out.push_str(&format!("      \x1b[36mUL: {} ({:.1}MB)\x1b[0m  \x1b[33mDL: {} ({:.1}MB)\x1b[0m\r\n",
            fmt_speed(tx_s), tx_mb, fmt_speed(rx_s), rx_mb));
    }
    out.push_str("\r\n");

    // ── 无线连接 ──
    let wifi = collector::network::get_wifi_info();
    let bt = collector::network::get_bluetooth_info();
    out.push_str("\x1b[1m  Wireless:\x1b[0m\r\n");
    if wifi.connected {
        let bitrate = if wifi.bitrate.is_empty() { String::new() } else { format!(" {}", wifi.bitrate) };
        let band_ch = if wifi.band.is_empty() {
            String::new()
        } else {
            format!(" {} Ch{}", wifi.band, wifi.channel)
        };
        out.push_str(&format!(
            "    \x1b[32mWiFi: {} {}dBm{}{}\x1b[0m\r\n",
            trunc_name(&wifi.ssid, 20),
            wifi.signal_dbm,
            band_ch,
            bitrate,
        ));
    } else {
        out.push_str("    WiFi: disconnected\r\n");
    }
    if bt.powered {
        let addr = if bt.address.is_empty() { "active" } else { &bt.address };
        out.push_str(&format!("    \x1b[34mBT:   ON  {}\x1b[0m\r\n", addr));
    } else {
        out.push_str("    BT:   OFF\r\n");
    }
    out.push_str("\r\n");

    // ── 磁盘分区（df -h -T，含 inode 与 fstype）──
    out.push_str("\x1b[1m  Disk:\x1b[0m\r\n");
    if let Ok(output) = std::process::Command::new("df").args(["-h", "-T"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines().skip(1) {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() >= 7 && p[0].starts_with("/dev/") {
                let (_, _, _, inode_pct) = collector::disk::get_inode_info(p[6]);
                out.push_str(&format!(
                    "\x1b[33m    {} {} {}/{} ({}) inode:{:.0}% {}\x1b[0m\r\n",
                    p[6], p[0], p[3], p[2], p[5], inode_pct, p[1],
                ));
            }
        }
    }

    // ── 磁盘 I/O 速率（动态块设备，最多 3 个）──
    for block_dev in collector::disk::list_block_devices(3) {
        let (read_sectors, write_sectors, _) = collector::disk::get_io_stats(&block_dev);
        let disk_type = collector::disk::get_disk_type(&block_dev);
        let (read_bps, write_bps) = disk_speeds.get(&block_dev).copied().unwrap_or((0.0, 0.0));
        let cum_read = read_sectors as f64 * 512.0;
        let cum_write = write_sectors as f64 * 512.0;
        out.push_str(&format!(
            "\x1b[36m    {} R {} W {}  {}  cum:R {} W {}\x1b[0m\r\n",
            block_dev,
            fmt_speed(read_bps),
            fmt_speed(write_bps),
            disk_type,
            fmt_bytes(cum_read),
            fmt_bytes(cum_write),
        ));
    }
    out.push_str("\r\n");

    // ── 进程 Top 10 CPU + Top 5 MEM ──
    let processes = collector::process::list_processes();
    let mut by_cpu: Vec<_> = processes.iter().collect();
    by_cpu.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut by_mem: Vec<_> = processes.iter().collect();
    by_mem.sort_by(|a, b| b.memory_mb.cmp(&a.memory_mb));

    out.push_str(&format!(
        "\x1b[1m  Processes ({} total):\x1b[0m\r\n",
        o.process_count
    ));
    out.push_str("  Top 10 by CPU:\r\n");
    out.push_str("    PID      CPU%    MEM MB  THR  NAME\r\n");
    for p in by_cpu.iter().take(10) {
        render_process_row(&mut out, p, p.cpu_usage as f64);
    }
    out.push_str("  Top 5 by MEM:\r\n");
    out.push_str("    PID      CPU%    MEM MB  THR  NAME\r\n");
    for p in by_mem.iter().take(5) {
        render_process_row(&mut out, p, p.memory_mb as f64);
    }
    out.push_str("\r\n");

    // ── 系统运行时间 ──
    let s = o.uptime as u64;
    let (d,h,m) = (s/86400,(s%86400)/3600,(s%3600)/60);
    let up = if d>0 {format!("{}d{}h{}m",d,h,m)} else if h>0 {format!("{}h{}m",h,m)} else {format!("{}m",m)};
    out.push_str(&format!("  Uptime: {}\r\n", up));

    write!(f, "{}", out)?;
    f.flush()?;
    Ok(())
}
