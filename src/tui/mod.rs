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
use crate::store::Database;

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
pub async fn run_tui(
    rx: &mut watch::Receiver<collector::SystemOverview>,
    tty_path: &str,
    db: Option<Arc<Database>>,
) -> std::io::Result<()> {
    let net_state: Arc<Mutex<HashMap<String, NetPrev>>> = Arc::new(Mutex::new(HashMap::new()));
    let disk_state: Arc<Mutex<HashMap<String, DiskPrev>>> = Arc::new(Mutex::new(HashMap::new()));
    let output = parse_tty_output(tty_path);
    loop {
        let o = rx.borrow_and_update().clone();
        let speeds = calc_speeds(&net_state, &o);
        let disk_speeds = calc_disk_speeds(&disk_state, &o);
        if let Err(_) = render(&output, &o, &speeds, &disk_speeds, db.as_deref()) { break; }
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

fn utf8_bar(pct: f64, w: usize) -> String {
    let width = w.max(1);
    let total_eighths = ((pct.clamp(0.0, 100.0) / 100.0) * width as f64 * 8.0).round() as usize;
    let full = (total_eighths / 8).min(width);
    let part = if full >= width { 0 } else { total_eighths % 8 };
    let empty = width.saturating_sub(full + usize::from(part > 0));
    let partial = match part {
        1 => "▏",
        2 => "▎",
        3 => "▍",
        4 => "▌",
        5 => "▋",
        6 => "▊",
        7 => "▉",
        _ => "",
    };
    format!("{}{}{}", "█".repeat(full), partial, "░".repeat(empty))
}

fn progress_bar(pct: f64, w: usize, utf8: bool) -> String {
    if utf8 {
        utf8_bar(pct, w)
    } else {
        bar(pct, w)
    }
}

fn term_layout(output: &TuiOutput) -> (usize, usize) {
    match output {
        TuiOutput::Stdout => crossterm::terminal::size()
            .map(|(c, r)| (c as usize, r as usize))
            .unwrap_or((80, 24)),
        TuiOutput::File(_) => (80, 24),
    }
}

fn bar_width(cols: usize, reserved: usize) -> usize {
    cols.saturating_sub(reserved).clamp(16, 72)
}

fn section_gap(rows: usize) -> &'static str {
    if rows >= 55 { "\r\n\r\n" } else { "\r\n" }
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
    let char_count = name.chars().count();
    if char_count <= max {
        return name.to_string();
    }
    let take = max.saturating_sub(3);
    format!("{}...", name.chars().take(take).collect::<String>())
}

fn clean_proxy_name(name: &str) -> String {
    let without_url = name
        .split_whitespace()
        .filter(|part| !part.starts_with("网址:") && !part.starts_with("网址："))
        .collect::<Vec<_>>()
        .join(" ");
    if without_url.is_empty() {
        "unknown".to_string()
    } else {
        without_url
    }
}

fn ascii_proxy_name(name: &str) -> String {
    let ascii = clean_proxy_name(name)
        .chars()
        .map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if ascii.is_empty() { "selected".to_string() } else { ascii }
}

fn char_width(c: char) -> usize {
    if c.is_ascii() { 1 } else { 2 }
}

fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn trunc_display(s: &str, max_width: usize) -> String {
    if display_width(s) <= max_width {
        return s.to_string();
    }
    let ellipsis = "..";
    let limit = max_width.saturating_sub(ellipsis.len());
    let mut width = 0;
    let mut out = String::new();
    for c in s.chars() {
        let w = char_width(c);
        if width + w > limit {
            break;
        }
        out.push(c);
        width += w;
    }
    out.push_str(ellipsis);
    out
}

fn fmt_uptime(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("{}d{}h{}m", d, h, m)
    } else if h > 0 {
        format!("{}h{}m", h, m)
    } else {
        format!("{}m", m)
    }
}

fn fmt_charge_ua(ua: u32) -> String {
    if ua == 0 {
        "unlim".to_string()
    } else if ua >= 1_000_000 {
        format!("{:.1}A", ua as f64 / 1_000_000.0)
    } else {
        format!("{}mA", ua / 1000)
    }
}

fn fmt_cpu_led_link(link: &collector::hardware::CpuStatusLedLinkState, utf8: bool) -> String {
    if !link.enabled {
        return if utf8 { "手动".into() } else { "manual".into() };
    }
    format!(
        "{}%+ {}%",
        link.threshold_pct,
        link.link_brightness_pct
    )
}

fn fmt_charge_source(source: &str, utf8: bool) -> &'static str {
    match (source, utf8) {
        ("wired", true) => "有线",
        ("wireless", true) => "无线",
        ("wired", false) => "wired",
        ("wireless", false) => "wls",
        (_, true) => "未接",
        (_, false) => "unplug",
    }
}

fn fmt_freq_khz(khz: u64) -> String {
    if khz >= 1_000_000 {
        format!("{:.2}GHz", khz as f64 / 1_000_000.0)
    } else if khz >= 1_000 {
        format!("{}MHz", khz / 1_000)
    } else {
        format!("{}kHz", khz)
    }
}

fn fmt_temp(celsius: f64, utf8: bool) -> String {
    if utf8 {
        format!("{:.0}°C", celsius)
    } else {
        format!("{:.0}C", celsius)
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "ON" } else { "off" }
}

fn on_off_label(on: bool, utf8: bool) -> &'static str {
    if utf8 {
        if on { "开" } else { "关" }
    } else {
        on_off(on)
    }
}

fn to_ascii_lossy(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii() && !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_floats(s: &str) -> Vec<f64> {
    let mut floats = Vec::new();
    let mut current = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            current.push(c);
        } else if !current.is_empty() {
            if let Ok(v) = current.parse() {
                floats.push(v);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(v) = current.parse() {
            floats.push(v);
        }
    }
    floats
}

/// TTY 帧缓冲通常仅支持 ASCII，将数据库中的中文告警转为英文摘要。
fn alert_to_ascii(title: &str, message: &str) -> (String, String) {
    let nums = extract_floats(message);
    let ascii_title = match title {
        "CPU 温度过高" => "CPU TEMP HIGH".to_string(),
        "内存使用率过高" => "MEM USAGE HIGH".to_string(),
        "电池电量低" => "BATTERY LOW".to_string(),
        _ => {
            let t = to_ascii_lossy(title);
            if t.is_empty() { "ALERT".to_string() } else { t }
        }
    };
    let ascii_msg = match title {
        "CPU 温度过高" if nums.len() >= 2 => {
            format!("max {:.1}C  limit {:.1}C", nums[0], nums[1])
        }
        "内存使用率过高" if nums.len() >= 2 => {
            format!("use {:.1}%  limit {:.1}%", nums[0], nums[1])
        }
        "电池电量低" if nums.len() >= 2 => {
            format!("level {:.0}%  limit {:.0}%", nums[0], nums[1])
        }
        _ => {
            let m = to_ascii_lossy(message);
            if m.is_empty() { "-".to_string() } else { m }
        }
    };
    (ascii_title, ascii_msg)
}

fn render_recent_alerts(out: &mut String, db: Option<&Database>, msg_limit: usize) {
    let Some(db) = db else { return };
    let Ok(alerts) = db.get_alerts(3) else { return };
    if alerts.is_empty() {
        return;
    }
    let utf8 = tui_utf8_enabled();
    out.push_str(if utf8 {
        "\x1b[1m  【告警】最近告警:\x1b[0m\r\n"
    } else {
        "\x1b[1m  ALERTS (recent):\x1b[0m\r\n"
    });
    for a in alerts {
        let title_raw = a["title"].as_str().unwrap_or("?");
        let msg_raw = a["message"].as_str().unwrap_or("");
        let level = a["level"].as_str().unwrap_or("info");
        let (title, msg) = if utf8 {
            (title_raw.to_string(), msg_raw.to_string())
        } else {
            alert_to_ascii(title_raw, msg_raw)
        };
        let tag = if utf8 {
            match level {
                "error" => "错误",
                "warning" => "警告",
                _ => "信息",
            }
        } else {
            match level {
                "error" => "ERR",
                "warning" => "WRN",
                _ => "INF",
            }
        };
        let color = match level {
            "error" => 31,
            "warning" => 33,
            _ => 36,
        };
        let limit = if utf8 { msg_limit.min(34) } else { msg_limit.min(42).max(28) };
        let title = if utf8 {
            trunc_display(&title, 16)
        } else {
            trunc_name(&title, 16)
        };
        let msg = if utf8 {
            trunc_display(&msg, limit)
        } else {
            trunc_name(&msg, limit)
        };
        out.push_str(&format!(
            "    \x1b[{}m[{}] {}: {}\x1b[0m\r\n",
            color,
            tag,
            title,
            msg,
        ));
    }
    out.push_str("\r\n");
}

/// TUI 输出目标：直接写 TTY 设备，或 stdout（配合 kmscon 等 UTF-8 终端）。
enum TuiOutput {
    File(String),
    Stdout,
}

fn parse_tty_output(path: &str) -> TuiOutput {
    match path {
        "-" | "stdout" | "/dev/stdout" => TuiOutput::Stdout,
        other => TuiOutput::File(other.to_string()),
    }
}

/// 是否输出 UTF-8 中文。裸 VT 帧缓冲无 CJK 字库时需配合 kmscon + Noto 字体。
fn tui_utf8_enabled() -> bool {
    match std::env::var("TUI_UTF8")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => return true,
        "0" | "false" | "no" | "off" => return false,
        _ => {}
    }
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_lowercase()
        .contains("utf-8")
}

fn write_frame(output: &TuiOutput, content: &str) -> std::io::Result<()> {
    match output {
        TuiOutput::Stdout => {
            use std::io::{stdout, Write};
            let mut out = stdout().lock();
            write!(out, "{}", content)?;
            out.flush()
        }
        TuiOutput::File(path) => {
            let mut f = OpenOptions::new().write(true).open(path)?;
            write!(f, "{}", content)?;
            f.flush()
        }
    }
}

fn render_process_row(out: &mut String, p: &collector::ProcessInfo, color_metric: f64, name_max: usize) {
    out.push_str(&format!(
        "    \x1b[{}m{:>6} {:>6.1}% {:>7} {:>4}  {}\x1b[0m\r\n",
        clr(color_metric),
        p.pid,
        p.cpu_usage,
        p.memory_mb,
        p.threads,
        trunc_name(&p.name, name_max),
    ));
}

fn render(
    output: &TuiOutput,
    o: &collector::SystemOverview,
    speeds: &HashMap<String, (f64, f64)>,
    disk_speeds: &HashMap<String, (f64, f64)>,
    db: Option<&Database>,
) -> std::io::Result<()> {
    let mut out = String::with_capacity(16384);
    let (cols, rows) = term_layout(output);
    let utf8 = tui_utf8_enabled();
    let gap = section_gap(rows);
    let w_cpu = bar_width(cols, 34);
    let w_core = bar_width(cols, 44).min(60);
    let w_gpu = bar_width(cols, 52).min(44);
    let w_bat = bar_width(cols, 44).min(56);
    let name_max = bar_width(cols, 40).clamp(10, 18);
    let msg_limit = bar_width(cols, 38);
    let thermal_n = if rows >= 50 { 6 } else { 5 };
    let proc_cpu_n = if rows >= 55 { 8 } else if rows >= 40 { 6 } else { 5 };
    let proc_mem_n = if rows >= 55 { 4 } else { 3 };

    let hw = collector::hardware::get_state();
    let gov = collector::cpu::get_governor();
    let freq = collector::cpu::get_frequency();

    // 隐藏光标并关闭自动换行：避免底部出现输入光标，超宽内容裁切在当前行。
    out.push_str("\x1b[?25l\x1b[?7l\x1b[2J\x1b[H");
    out.push_str("\r\n");

    // ── 标题栏 + 系统标识 ──
    let ts = chrono::Local::now().format("%H:%M:%S");
    let up = fmt_uptime(o.uptime as u64);
    if utf8 {
        out.push_str(&format!(
            "\x1b[36;1m  【设备】设备监控 v0.2 | {} | 运行 {}\x1b[0m\r\n",
            ts, up
        ));
        out.push_str(&format!(
            "  {} | 内核 {} | 进程 {} | 可用 {}/{} MB\r\n",
            read_hostname(),
            read_kernel(),
            o.process_count,
            o.memory.available_mb,
            o.memory.total_mb,
        ));
    } else {
        out.push_str(&format!(
            "\x1b[36;1m  Device Monitor v0.2 | {} | up {}\x1b[0m\r\n",
            ts, up
        ));
        out.push_str(&format!(
            "  {} | kernel {} | {} procs | avail {}/{} MB\r\n",
            read_hostname(),
            read_kernel(),
            o.process_count,
            o.memory.available_mb,
            o.memory.total_mb,
        ));
    }
    out.push_str(gap);

    // ── CPU 总使用率 + 负载 + governor ──
    let cpu = o.cpu.overall_usage as f64;
    out.push_str(&format!(
        "\x1b[{}m  【CPU】{} {:.1}%\x1b[0m\r\n",
        clr(cpu),
        progress_bar(cpu, w_cpu, utf8),
        cpu,
    ));
    if utf8 {
        out.push_str(&format!(
            "  负载 {:.2}/{:.2}/{:.2} | {} 核 | 策略 {} | 频率 {}-{}",
            o.load_avg[0],
            o.load_avg[1],
            o.load_avg[2],
            o.cpu.cores.len(),
            gov.current,
            fmt_freq_khz(freq.min_freq),
            fmt_freq_khz(freq.max_freq),
        ));
    } else {
        out.push_str(&format!(
            "  Load {:.2}/{:.2}/{:.2} | {} cores | gov: {} | cap {}-{}",
            o.load_avg[0],
            o.load_avg[1],
            o.load_avg[2],
            o.cpu.cores.len(),
            gov.current,
            fmt_freq_khz(freq.min_freq),
            fmt_freq_khz(freq.max_freq),
        ));
    }
    out.push_str(gap);

    // ── GPU ──
    let gpu_pct = if hw.gpu.max_freq_mhz > 0 {
        (hw.gpu.cur_freq_mhz as f64 / hw.gpu.max_freq_mhz as f64 * 100.0).min(100.0)
    } else {
        0.0
    };
    let gpu_gov = trunc_name(&hw.gpu.governor, if utf8 { 10 } else { 12 });
    out.push_str(&format!(
        "\x1b[35m  【GPU】{} {:>3}MHz\x1b[0m {}{} {}:{}\r\n",
        progress_bar(gpu_pct, w_gpu, utf8),
        hw.gpu.cur_freq_mhz,
        if utf8 { "上限" } else { "max" },
        hw.gpu.max_freq_mhz,
        if utf8 { "策略" } else { "gov" },
        gpu_gov,
    ));
    out.push_str(gap);

    // ── 硬件状态 ──
    out.push_str(if utf8 { "\x1b[1m  【硬件】硬件:\x1b[0m\r\n" } else { "\x1b[1m  Hardware:\x1b[0m\r\n" });
    if utf8 {
        out.push_str(&format!(
            "    屏幕:{} {}% | 手电 白:{} 黄:{} | 状态灯:{}\r\n",
            if hw.screen_on { "开" } else { "关" },
            hw.brightness.percent,
            on_off_label(hw.flashlight.white_on, true),
            on_off_label(hw.flashlight.yellow_on, true),
            on_off_label(hw.status_led.on, true),
        ));
        out.push_str(&format!(
            "    充电:{} {} {} | WiFi省电:{} | CPU灯:{} | 震动:{}\r\n",
            fmt_charge_source(&hw.charging.charge_source, true),
            if hw.charging.charge_mode == "power_only" { "仅供电" } else { "充电" },
            fmt_charge_ua(hw.charging.current_max_ua),
            if hw.wifi_power_save.enabled { "开" } else { "关" },
            fmt_cpu_led_link(&hw.cpu_status_led_link, true),
            if hw.vibrating { "运行" } else { "空闲" },
        ));
        if hw.charging.charger_online && hw.charging.power_w > 0.0 {
            out.push_str(&format!(
                "    充电功率:{:.1}W {:.0}mA\r\n",
                hw.charging.power_w,
                hw.charging.current_now_ua.max(0) as f64 / 1000.0,
            ));
        }
    } else {
        out.push_str(&format!(
            "    Screen: {} {}% | Flash W:{} Y:{} | Status LED: {}\r\n",
            if hw.screen_on { "ON" } else { "OFF" },
            hw.brightness.percent,
            on_off(hw.flashlight.white_on),
            on_off(hw.flashlight.yellow_on),
            on_off(hw.status_led.on),
        ));
        out.push_str(&format!(
            "    Charge: {} {} {} | WiFi PS: {} | CPU LED: {} | Vibrate: {}\r\n",
            fmt_charge_source(&hw.charging.charge_source, false),
            if hw.charging.charge_mode == "power_only" { "pwr" } else { "chg" },
            fmt_charge_ua(hw.charging.current_max_ua),
            if hw.wifi_power_save.enabled { "on" } else { "off" },
            fmt_cpu_led_link(&hw.cpu_status_led_link, false),
            if hw.vibrating { "active" } else { "idle" },
        ));
        if hw.charging.charger_online && hw.charging.power_w > 0.0 {
            out.push_str(&format!(
                "    Charge pwr: {:.1}W {:.0}mA\r\n",
                hw.charging.power_w,
                hw.charging.current_now_ua.max(0) as f64 / 1000.0,
            ));
        }
    }
    out.push_str(gap);

    // ── 内存 + Swap ──
    let mem = o.memory.usage_percent;
    out.push_str(&format!(
        "\x1b[{}m  {} {} {:.1}%\x1b[0m\r\n",
        clr(mem),
        if utf8 { "【内存】" } else { "MEM" },
        progress_bar(mem, w_cpu, utf8),
        mem,
    ));
    out.push_str(&format!(
        "  {}/{} MB  {} {}/{} MB\r\n",
        o.memory.used_mb,
        o.memory.total_mb,
        if utf8 { "交换" } else { "Swap" },
        o.memory.swap_used_mb,
        o.memory.swap_total_mb,
    ));
    out.push_str(gap);

    // ── 各 CPU 核心详情 ──
    let tm: HashMap<usize, f64> = o.thermal.iter().filter_map(|t| {
        let n = &t.name;
        if n.starts_with("cpu") && n.ends_with("-thermal") {
            n[3..n.len()-8].parse::<usize>().ok().map(|id| (id, t.temp_celsius))
        } else { None }
    }).collect();

    out.push_str(&format!(
        "\x1b[1m  {} ({}{}):\x1b[0m\r\n",
        if utf8 { "【核心】" } else { "Per-Core" },
        o.cpu.cores.len(),
        if utf8 { "核" } else { " cores" },
    ));
    for c in &o.cpu.cores {
        let u = c.usage as f64;
        let temp = tm.get(&c.id).map(|t| fmt_temp(*t, utf8)).unwrap_or("---".into());
        out.push_str(&format!("  \x1b[{}mC{} {} {:>5.1}% {:>4}MHz {:>4}\x1b[0m\r\n",
            clr(u), c.id, progress_bar(u, w_core, utf8), u, c.frequency_mhz, temp));
    }
    out.push_str(gap);

    // ── 电池状态 ──
    let b = &o.battery;
    let bc = if b.capacity < 20 { 31 } else if b.capacity < 50 { 33 } else { 32 };
    let bs = if utf8 {
        match b.status.as_str() { "Charging" => "充电", "Discharging" => "电池", "Full" => "已满", _ => &b.status }
    } else {
        match b.status.as_str() { "Charging" => "CHG", "Discharging" => "BAT", "Full" => "FULL", _ => &b.status }
    };
    let time = if b.time_left_min > 0 {
        if utf8 { format!("余{}h{}m", b.time_left_min/60, b.time_left_min%60) }
        else { format!("Left {}h{}m", b.time_left_min/60, b.time_left_min%60) }
    }
        else if b.time_left_min < 0 { let a = b.time_left_min.unsigned_abs(); if utf8 { format!("满{}h{}m", a/60, a%60) } else { format!("Full {}h{}m", a/60, a%60) } }
        else { String::new() };
    let power_label = match b.status.as_str() {
        "Charging" => format!("+{:.1}W", b.power_w),
        "Discharging" => format!("-{:.1}W", b.power_w),
        _ => format!("{:.1}W", b.power_w),
    };
    out.push_str(&format!("\x1b[{}m  {} {} {}% {} {} {:.1}V {:.0}mA {} {}\x1b[0m\r\n",
        bc, if utf8 { "【电池】" } else { "BAT" }, progress_bar(b.capacity as f64, w_bat, utf8), b.capacity, bs, power_label, b.voltage_v, b.current_ma, fmt_temp(b.temp_celsius, utf8), time));
    out.push_str(gap);

    // ── 温度传感器（按温度降序，显示前 N 个）──
    let mut thermals: Vec<_> = o.thermal.iter().collect();
    thermals.sort_by(|a, b| b.temp_celsius.partial_cmp(&a.temp_celsius).unwrap());
    if utf8 {
        out.push_str(&format!("\x1b[1m  【温度】温度 (前 {}):\x1b[0m\r\n  ", thermal_n));
    } else {
        out.push_str(&format!("\x1b[1m  Thermal (top {}):\x1b[0m\r\n  ", thermal_n));
    }
    for (i, t) in thermals.iter().take(thermal_n).enumerate() {
        if i > 0 { out.push_str(" | "); }
        let name = t.name.replace("-thermal", "");
        out.push_str(&format!(
            "\x1b[{}m{}:{}\x1b[0m",
            clr(t.temp_celsius),
            trunc_display(&name, if utf8 { 10 } else { 14 }),
            fmt_temp(t.temp_celsius, utf8),
        ));
    }
    out.push_str("\r\n");
    out.push_str(gap);

    // ── 网络接口（仅显示 is_up 的接口）──
    out.push_str(if utf8 { "\x1b[1m  【网络】网络:\x1b[0m\r\n" } else { "\x1b[1m  Network:\x1b[0m\r\n" });
    for n in o.network.iter().filter(|n| n.is_up) {
        let ip = n.ip_addresses.first().map(|s| s.as_str()).unwrap_or("?");
        let rx_mb = n.rx_bytes as f64 / 1048576.0;
        let tx_mb = n.tx_bytes as f64 / 1048576.0;
        let (rx_s, tx_s) = speeds.get(&n.name).copied().unwrap_or((0.0, 0.0));
        out.push_str(&format!("\x1b[32m    {}: {}\x1b[0m\r\n", n.name, ip));
        out.push_str(&format!(
            "      \x1b[36m{}:{} ({:.1}MB)\x1b[0m  \x1b[33m{}:{} ({:.1}MB)\x1b[0m\r\n",
            if utf8 { "上" } else { "UL" },
            fmt_speed(tx_s),
            tx_mb,
            if utf8 { "下" } else { "DL" },
            fmt_speed(rx_s),
            rx_mb,
        ));
    }
    if o.mihomo.available {
        let vpn_kind = if o.mihomo.tun_enabled {
            if utf8 { "TUN" } else { "tun" }
        } else if utf8 {
            "代理"
        } else {
            "proxy"
        };
        let proxy_name = if utf8 {
            clean_proxy_name(&o.mihomo.active_proxy)
        } else {
            ascii_proxy_name(&o.mihomo.active_proxy)
        };
        let proxy = trunc_display(&proxy_name, if utf8 { 30 } else { 36 });
        if utf8 {
            out.push_str(&format!(
                "    \x1b[35mVPN: {} {} | 节点:{} | 连接:{} | ↓{} ↑{}\x1b[0m\r\n",
                vpn_kind,
                o.mihomo.mode,
                proxy,
                o.mihomo.connection_count,
                fmt_bytes(o.mihomo.download_total as f64),
                fmt_bytes(o.mihomo.upload_total as f64),
            ));
        } else {
            out.push_str(&format!(
                "    \x1b[35mVPN: {} {} | node:{} | conn:{} | D:{} U:{}\x1b[0m\r\n",
                vpn_kind,
                o.mihomo.mode,
                proxy,
                o.mihomo.connection_count,
                fmt_bytes(o.mihomo.download_total as f64),
                fmt_bytes(o.mihomo.upload_total as f64),
            ));
        }
    } else {
        out.push_str(if utf8 { "    VPN: 未连接\r\n" } else { "    VPN: disconnected\r\n" });
    }
    out.push_str(gap);

    // ── 无线连接 ──
    let wifi = collector::network::get_wifi_info();
    let bt = collector::network::get_bluetooth_info();
    out.push_str(if utf8 { "\x1b[1m  【无线】无线:\x1b[0m\r\n" } else { "\x1b[1m  Wireless:\x1b[0m\r\n" });
    if wifi.connected {
        let bitrate = if wifi.bitrate.is_empty() { String::new() } else { format!(" {}", wifi.bitrate) };
        let band_ch = if wifi.band.is_empty() {
            String::new()
        } else {
            format!(" {} Ch{}", wifi.band, wifi.channel)
        };
        out.push_str(&format!(
            "    \x1b[32mWiFi: {} {}dBm{}{}\x1b[0m\r\n",
            trunc_name(&wifi.ssid, name_max.min(20)),
            wifi.signal_dbm,
            band_ch,
            bitrate,
        ));
    } else {
        out.push_str(if utf8 { "    WiFi: 未连接\r\n" } else { "    WiFi: disconnected\r\n" });
    }
    if bt.powered {
        let addr = if bt.address.is_empty() { "active" } else { &bt.address };
        let devs = bt.devices.len();
        let conn = bt.devices.iter().filter(|d| d.connected).count();
        if utf8 {
            out.push_str(&format!(
                "    \x1b[34mBT:   开  {}  设备:{} 已连:{}\x1b[0m\r\n",
                trunc_display(addr, 18),
                devs,
                conn,
            ));
        } else {
            out.push_str(&format!(
                "    \x1b[34mBT:   ON  {}  devs:{} conn:{}\x1b[0m\r\n",
                addr, devs, conn
            ));
        }
    } else {
        out.push_str(if utf8 { "    BT:   关\r\n" } else { "    BT:   OFF\r\n" });
    }
    out.push_str(gap);

    render_recent_alerts(&mut out, db, msg_limit);

    // ── 磁盘分区（df -h -T，含 inode 与 fstype）──
    out.push_str(if utf8 { "\x1b[1m  【磁盘】磁盘:\x1b[0m\r\n" } else { "\x1b[1m  Disk:\x1b[0m\r\n" });
    if let Ok(output) = std::process::Command::new("df").args(["-h", "-T"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines().skip(1) {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() >= 7 && p[0].starts_with("/dev/") {
                let (_, _, _, inode_pct) = collector::disk::get_inode_info(p[6]);
                out.push_str(&format!(
                    "\x1b[33m    {} {}/{} {} {}{:.0}% {}\x1b[0m\r\n",
                    p[6],
                    p[3],
                    p[2],
                    p[5],
                    if utf8 { "索引" } else { "inode" },
                    inode_pct,
                    p[1],
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
            "\x1b[36m    {} {}:{} {}:{} {} {}:{} {}:{}\x1b[0m\r\n",
            block_dev,
            if utf8 { "读" } else { "R" },
            fmt_speed(read_bps),
            if utf8 { "写" } else { "W" },
            fmt_speed(write_bps),
            disk_type,
            if utf8 { "累计读" } else { "R" },
            fmt_bytes(cum_read),
            if utf8 { "写" } else { "W" },
            fmt_bytes(cum_write),
        ));
    }
    out.push_str(gap);

    // ── 进程 Top N CPU + Top N MEM ──
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
        "\x1b[1m  {} ({}{}):\x1b[0m\r\n",
        if utf8 { "【进程】" } else { "Processes" },
        o.process_count,
        if utf8 { "个" } else { " total" },
    ));
    if utf8 {
        out.push_str(&format!("  CPU 前 {}:\r\n", proc_cpu_n));
    } else {
        out.push_str(&format!("  Top {} by CPU:\r\n", proc_cpu_n));
    }
    out.push_str(if utf8 { "    PID      CPU%    内存MB  线程  名称\r\n" } else { "    PID      CPU%    MEM MB  THR  NAME\r\n" });
    for p in by_cpu.iter().take(proc_cpu_n) {
        render_process_row(&mut out, p, p.cpu_usage as f64, name_max);
    }
    if utf8 {
        out.push_str(&format!("  内存前 {}:\r\n", proc_mem_n));
    } else {
        out.push_str(&format!("  Top {} by MEM:\r\n", proc_mem_n));
    }
    out.push_str(if utf8 { "    PID      CPU%    内存MB  线程  名称\r\n" } else { "    PID      CPU%    MEM MB  THR  NAME\r\n" });
    for p in by_mem.iter().take(proc_mem_n) {
        render_process_row(&mut out, p, p.memory_mb as f64, name_max);
    }

    // 保持光标隐藏并移到右下角，防止终端在内容末尾显示输入方块。
    out.push_str(&format!("\x1b[{};{}H\x1b[?25l", rows, cols));
    write_frame(output, &out)
}
