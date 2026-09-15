//! 横屏仪表布局。
//!
//! 布局范式取 TUI Design System 的 **Widget Dashboard**：每张卡片自包含、
//! 位置固定（不随数据重排，维持空间记忆），全部信息一屏可见无需导航。
//!
//! 视觉层级：卡片标题用 `FG_MUTED` + Bold，主数值用 `FG_EMPHASIS` + 超大字号，
//! 单位/元数据降级为 `FG_MUTED`；颜色只做语义提示，数值始终同时呈现。
//!
//! 逻辑画布 2340x1080 网格：
//! ```text
//! ┌──────────── 顶栏：设备 / 时钟 ────────────┐
//! ├─ CPU/GPU/温度 ─┬─ 内存/磁盘/进程 ─┬─ 电池/网络/服务 ─┤
//! └──────────── 底部状态条 ────────────┘
//! ```

use std::collections::HashMap;
use std::sync::OnceLock;

use chrono::Datelike;

use crate::collector::hardware::HardwareState;
use crate::collector::{BluetoothInfo, ProcessInfo, SystemOverview, WifiInfo};
use crate::store::Database;

use super::canvas::Canvas;
use super::font::Weight;
use super::theme::{Argb, Palette, Type, level, temp_level};

// ── 布局常量 ──
const PAD: i32 = 22;
const GAP: i32 = 16;
const CARD_R: i32 = 16;
const HEADER_H: i32 = 96;
const FOOTER_H: i32 = 56;
/// 迷你趋势图样本数基准（按采集间隔 5s 设计 → 约 6 分钟时间窗）
const HIST_LEN: usize = 72;

/// 迷你趋势图的实际样本上限：刷新间隔可调后按比例换算，
/// 保持时间窗（约 6 分钟）不随刷新率变化（1s→360、3s→120、5s→72、10s→36）。
pub fn hist_cap(refresh_secs: u64) -> usize {
    const SPAN_SECS: usize = HIST_LEN * 5;
    (SPAN_SECS / refresh_secs.max(1) as usize).max(HIST_LEN / 4)
}

/// 逻辑面板矩形。
#[derive(Clone, Copy)]
pub(crate) struct Pane {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 磁盘分区行。
pub struct DiskRow {
    pub mount: String,
    pub fstype: String,
    pub size: String,
    pub used: String,
    pub avail: String,
    pub pct: f64,
    pub inode_pct: f64,
}

/// 非 `SystemOverview` 的辅助数据（子进程/文件系统/无线/告警等）。
pub struct Aux {
    pub services: Vec<(String, String)>,
    pub processes: Vec<ProcessInfo>,
    pub disks: Vec<DiskRow>,
    pub wifi: WifiInfo,
    pub bt: BluetoothInfo,
    pub hw: HardwareState,
    /// (level, title, message, timestamp)
    pub alerts: Vec<(String, String, String, i64)>,
    pub net_speeds: HashMap<String, (f64, f64)>,
    pub disk_speeds: HashMap<String, (f64, f64)>,
    net_prev: HashMap<String, (u64, u64, i64)>,
    disk_prev: HashMap<String, (u64, u64, i64)>,
}

impl Aux {
    pub fn new() -> Self {
        Self {
            services: Vec::new(),
            processes: Vec::new(),
            disks: Vec::new(),
            wifi: crate::collector::network::get_wifi_info(),
            bt: crate::collector::network::get_bluetooth_info(),
            hw: crate::collector::hardware::get_state(),
            alerts: Vec::new(),
            net_speeds: HashMap::new(),
            disk_speeds: HashMap::new(),
            net_prev: HashMap::new(),
            disk_prev: HashMap::new(),
        }
    }

    /// 采集辅助数据（含带宽/磁盘速率差分）。属于阻塞操作，只在渲染线程调用。
    pub fn refresh(&mut self, o: &SystemOverview, db: Option<&Database>) {
        // 服务状态
        self.services = ["mihomo", "syncthing", "device-monitor", "xtokenhub"]
            .iter()
            .map(|s| {
                let st = std::process::Command::new("systemctl")
                    .args(["is-active", s])
                    .output()
                    .ok()
                    .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                (s.to_string(), st)
            })
            .collect();

        self.processes = crate::collector::process::list_processes();
        self.wifi = crate::collector::network::get_wifi_info();
        self.bt = crate::collector::network::get_bluetooth_info();
        self.hw = crate::collector::hardware::get_state();

        self.alerts = db
            .and_then(|d| d.get_alerts(4).ok())
            .unwrap_or_default()
            .iter()
            .filter_map(|v| {
                Some((
                    v["level"].as_str().unwrap_or("info").to_string(),
                    v["title"].as_str().unwrap_or("").to_string(),
                    v["message"].as_str().unwrap_or("").to_string(),
                    v["timestamp"].as_i64().unwrap_or(0),
                ))
            })
            .collect();

        self.disks = disk_rows();

        // 网络速率差分
        let ts = o.timestamp;
        let mut speeds = HashMap::new();
        for n in &o.network {
            if let Some(&(prx, ptx, pts)) = self.net_prev.get(&n.name) {
                let dt = (ts - pts) as f64;
                if dt > 0.0 {
                    speeds.insert(
                        n.name.clone(),
                        (
                            n.rx_bytes.saturating_sub(prx) as f64 / dt,
                            n.tx_bytes.saturating_sub(ptx) as f64 / dt,
                        ),
                    );
                }
            }
            self.net_prev.insert(n.name.clone(), (n.rx_bytes, n.tx_bytes, ts));
        }
        self.net_speeds = speeds;

        // 磁盘速率差分
        let mut dspeeds = HashMap::new();
        for dev in crate::collector::disk::list_block_devices(3) {
            let (rs, ws, _) = crate::collector::disk::get_io_stats(&dev);
            if let Some(&(prs, pws, pts)) = self.disk_prev.get(&dev) {
                let dt = (ts - pts) as f64;
                if dt > 0.0 {
                    dspeeds.insert(
                        dev.clone(),
                        (
                            rs.saturating_sub(prs) as f64 * 512.0 / dt,
                            ws.saturating_sub(pws) as f64 * 512.0 / dt,
                        ),
                    );
                }
            }
            self.disk_prev.insert(dev.clone(), (rs, ws, ts));
        }
        self.disk_speeds = dspeeds;
    }
}

/// 趋势环形缓冲。
pub struct History {
    pub cpu: Vec<f32>,
    pub mem: Vec<f32>,
    pub net: Vec<f32>,
    cur_net: f32,
}

impl History {
    pub fn new() -> Self {
        Self { cpu: Vec::new(), mem: Vec::new(), net: Vec::new(), cur_net: 0.0 }
    }

    /// 预览/导出用：填入一段合成波形，让趋势图在静态导出里也能看出形态。
    pub fn seed_wave(&mut self, cpu: f32, mem: f32, net: f32) {
        for i in 0..HIST_LEN {
            let t = i as f32 / HIST_LEN as f32 * std::f32::consts::TAU;
            self.cpu.push((cpu + cpu * 1.7 * (t * 2.0).sin()).clamp(0.5, 100.0));
            self.mem.push((mem + 6.0 * t.sin()).clamp(0.5, 100.0));
            self.net.push((net * (1.0 + 0.9 * (t * 3.0).sin())).max(0.0));
            self.cur_net = net;
        }
    }

    pub fn push(&mut self, o: &SystemOverview, a: &Aux, cap: usize) {
        push_capped(&mut self.cpu, o.cpu.overall_usage, cap);
        push_capped(&mut self.mem, o.memory.usage_percent as f32, cap);
        // 有效网卡（up 且非 lo）的 rx 速率之和
        let mut total = 0.0f32;
        for n in o.network.iter().filter(|n| n.is_up && n.name != "lo") {
            if let Some((rx, _)) = a.net_speeds.get(&n.name) {
                total += *rx as f32;
            }
        }
        self.cur_net = total;
        push_capped(&mut self.net, total, cap);
    }
}

fn push_capped(v: &mut Vec<f32>, x: f32, cap: usize) {
    v.push(x);
    if v.len() > cap {
        v.remove(0);
    }
}

// ── 顶层渲染 ──

/// 整屏重绘（数据变化时调用）。
pub fn render(c: &mut Canvas, o: &SystemOverview, a: &Aux, h: &History) {
    let (w, ht) = (c.lw, c.lh);
    c.clear(Palette::BG_BASE);

    let inner = w - PAD * 2;
    let header = Pane { x: PAD, y: PAD, w: inner, h: HEADER_H };
    let footer = Pane { x: PAD, y: ht - PAD - FOOTER_H, w: inner, h: FOOTER_H };
    let body_y = header.y + header.h + GAP;
    let body_h = footer.y - GAP - body_y;

    let usable = inner - GAP * 2;
    let c1w = (usable as f64 * 0.295).round() as i32;
    let c3w = (usable as f64 * 0.345).round() as i32;
    let c2w = usable - c1w - c3w;

    header_card(c, &header, o);
    footer_card(c, &footer, o, a);

    // 第 1 列：CPU / GPU / 温度
    let col1 = split_col(PAD, body_y, c1w, body_h, &[54, 24, 22]);
    card_cpu(c, &col1[0], o, h);
    card_gpu(c, &col1[1], a);
    card_thermal(c, &col1[2], o);

    // 第 2 列：内存 / 磁盘 / 进程
    let col2 = split_col(PAD + c1w + GAP, body_y, c2w, body_h, &[22, 26, 52]);
    card_memory(c, &col2[0], o, h);
    card_disk(c, &col2[1], a);
    card_process(c, &col2[2], o, a);

    // 第 3 列：电池 / 网络 / 服务与告警
    let col3 = split_col(PAD + c1w + GAP + c2w + GAP, body_y, c3w, body_h, &[30, 32, 38]);
    card_battery(c, &col3[0], o, a);
    card_network(c, &col3[1], o, a);
    card_service(c, &col3[2], a);
}

/// 只重绘顶栏时钟（每秒调用，避免整屏重绘）。
pub fn render_clock(c: &mut Canvas, o: &SystemOverview) {
    let inner = c.lw - PAD * 2;
    let header = Pane { x: PAD, y: PAD, w: inner, h: HEADER_H };
    clock(c, &header, o);
}

/// 按比例切分一列（最后一格吃掉余量，避免像素缝隙）。

/// 页面标签（可发现性）：高亮当前页，返回最后一个标签的右边界。
pub(crate) fn page_tabs(c: &mut Canvas, x: i32, y: i32, active: u8) -> i32 {
    const TABS: [&str; 3] = ["系统监控", "Token 用量", "时钟"];
    let mut sx = x;
    for (i, label) in TABS.iter().enumerate() {
        let on = i as u8 == active;
        let tw = c.fonts.text_width(label, Type::TINY, Weight::Bold);
        let w = tw + 34;
        let (bg, fg) = if on {
            (Palette::ACCENT, Palette::BG_BASE)
        } else {
            (Palette::BG_SURFACE_ALT, Palette::FG_MUTED)
        };
        c.round_rect(sx, y, w, 32, 16, bg);
        let (asc, _, _) = c.fonts.metrics(Type::TINY, Weight::Bold);
        let baseline = y + (32 + asc.round() as i32) / 2 - 1;
        c.text(sx + 17, baseline, label, Type::TINY, Weight::Bold, fg);
        sx += w + 10;
    }
    sx
}

pub(crate) fn split_col(x: i32, y: i32, w: i32, total_h: i32, ratios: &[i32]) -> Vec<Pane> {
    let n = ratios.len() as i32;
    let sum: i32 = ratios.iter().sum();
    let avail = total_h - GAP * (n - 1);
    let mut out = Vec::with_capacity(n as usize);
    let mut cy = y;
    let mut used = 0;
    for (i, r) in ratios.iter().enumerate() {
        let h = if i as i32 == n - 1 {
            avail - used
        } else {
            (avail as f64 * (*r as f64 / sum as f64)).round() as i32
        };
        out.push(Pane { x, y: cy, w, h });
        cy += h + GAP;
        used += h;
    }
    out
}

pub(crate) fn card(c: &mut Canvas, p: &Pane) {
    c.round_rect(p.x, p.y, p.w, p.h, CARD_R, Palette::BG_SURFACE);
}

pub(crate) fn title(c: &mut Canvas, p: &Pane, text: &str) -> i32 {
    c.text(p.x + 22, p.y + 38, text, Type::TITLE, Weight::Bold, Palette::FG_EMPHASIS);
    c.fonts.text_width(text, Type::TITLE, Weight::Bold) + 22 + p.x
}

// ── 顶栏 ──

fn header_card(c: &mut Canvas, p: &Pane, o: &SystemOverview) {
    card(c, p);
    let x = p.x + 26;
    let host = hostname();
    c.dot(x + 9, p.y + 32, 9, Palette::SUCCESS);
    c.text(x + 30, p.y + 46, &host, Type::H1, Weight::Bold, Palette::FG_EMPHASIS);
    let tw = c.fonts.text_width(&host, Type::H1, Weight::Bold);
    c.text(
        x + 30 + tw + 18,
        p.y + 46,
        "Mi Mix 3 · 物理屏仪表",
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    let sub = format!(
        "内核 {}  ·  运行 {}  ·  进程 {}  ·  负载 {:.2} / {:.2} / {:.2}",
        kernel(),
        fmt_uptime(o.uptime as u64),
        o.process_count,
        o.load_avg[0],
        o.load_avg[1],
        o.load_avg[2]
    );
    c.text(x, p.y + 82, &sub, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    page_tabs(c, p.x + p.w - 900, p.y + 32, 0);
    clock(c, p, o);
}

fn clock(c: &mut Canvas, p: &Pane, o: &SystemOverview) {
    let zone_w = 460;
    let zx = p.x + p.w - zone_w - 20;
    c.rect(zx, p.y + 4, zone_w, p.h - 8, Palette::BG_SURFACE);

    let now = chrono::Local::now();
    let t = now.format("%H:%M:%S").to_string();
    let right = p.x + p.w - 26;
    c.text_right(right, p.y + 60, &t, Type::CLOCK, Weight::Bold, Palette::FG_EMPHASIS);
    let wd = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"][now.weekday().num_days_from_monday() as usize];
    let date = format!("{} {}月{}日", wd, now.format("%m"), now.format("%d"));
    let tz = now.format("%Z").to_string();
    c.text_right(right, p.y + 88, &format!("{date}  {tz}"), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    let _ = o;
}

// ── 底栏 ──

fn footer_card(c: &mut Canvas, p: &Pane, o: &SystemOverview, a: &Aux) {
    card(c, p);
    let x = p.x + 26;
    let baseline = p.y + p.h / 2 + 8;

    let active: Vec<_> = o.network.iter().filter(|n| n.is_up && n.name != "lo").collect();
    let ip = active
        .iter()
        .flat_map(|n| n.ip_addresses.iter())
        .find(|ip| !ip.contains(':'))
        .cloned()
        .unwrap_or_else(|| "无 IPv4".to_string());
    c.text(x, baseline, "IPv4", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text(x + 62, baseline, &ip, Type::BODY, Weight::Bold, Palette::ACCENT);

    let dl: f64 = active.iter().filter_map(|n| a.net_speeds.get(&n.name)).map(|(rx, _)| *rx).sum();
    let ul: f64 = active.iter().filter_map(|n| a.net_speeds.get(&n.name)).map(|(_, tx)| *tx).sum();
    c.text(x + 420, baseline, "下行", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text(x + 482, baseline, &fmt_speed(dl), Type::BODY, Weight::Regular, Palette::INFO);
    c.text(x + 660, baseline, "上行", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text(x + 722, baseline, &fmt_speed(ul), Type::BODY, Weight::Regular, Palette::ACCENT_2);

    // 右侧信息先量宽，剩余空间才给 VPN，避免两段文字相撞
    let footer_info = format!(
        "指标采样 {}s 前 · 逻辑画布 {}×{}",
        (chrono::Utc::now().timestamp() - o.timestamp).max(0),
        c.lw,
        c.lh
    );
    let info_w = c.fonts.text_width(&footer_info, Type::LABEL, Weight::Regular);

    let vpn = if o.mihomo.available {
        format!(
            "{} · {}",
            if o.mihomo.tun_enabled { "TUN" } else { "代理" },
            clean_proxy(&o.mihomo.active_proxy)
        )
    } else {
        "未连接".to_string()
    };
    let vx = x + 900;
    let avail = (p.x + p.w - 26 - info_w - 40) - (vx + 62);
    c.text(vx, baseline, "VPN", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    let vpn_txt = truncate(c, &vpn, avail.max(80), Type::BODY, Weight::Regular);
    c.text(
        vx + 62,
        baseline,
        &vpn_txt,
        Type::BODY,
        Weight::Regular,
        if o.mihomo.available { Palette::SUCCESS } else { Palette::FG_MUTED },
    );

    c.text_right(
        p.x + p.w - 26,
        baseline,
        &footer_info,
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

// ── CPU ──

fn card_cpu(c: &mut Canvas, p: &Pane, o: &SystemOverview, h: &History) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    let _ = title(c, p, "CPU");

    // governor 胶囊（右上）
    let gov = crate::collector::cpu::get_governor().current;
    let gov = if gov.is_empty() { "n/a".to_string() } else { gov };
    c.pill(p.x + p.w - 130, y + 18, &gov, Type::TINY, Palette::ACCENT, Palette::BG_SURFACE_ALT);

    let usage = o.cpu.overall_usage as f64;
    let color = level(usage);
    c.text(x, y + 122, &format!("{usage:.1}"), Type::VALUE_XL, Weight::Bold, color);
    let num_w = c.fonts.text_width(&format!("{usage:.1}"), Type::VALUE_XL, Weight::Bold);
    c.text(x + num_w + 8, y + 122, "%", Type::VALUE_M, Weight::Regular, Palette::FG_MUTED);

    // 趋势（真实折线）
    c.text(x + 300, y + 74, "近 6 分钟", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    let (sx, sw, sh) = (x + 300, p.w - 300 - 40, 62);
    c.rect(sx, y + 84, sw, sh, Palette::BG_SURFACE_ALT);
    c.hline(sx + 1, y + 84 + sh - 4, sw - 2, 1, Palette::BORDER);
    c.sparkline(sx + 6, y + 88, sw - 12, sh - 10, &h.cpu, 100.0, color);

    // 8 核明细
    let row_h = 33;
    let mut cy = y + 152;
    for core in &o.cpu.cores {
        let u = core.usage as f64;
        let cc = level(u);
        c.text(x, cy + 20, &format!("C{}", core.id), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.bar(x + 44, cy + 8, 150, 14, u, cc);
        c.text(x + 210, cy + 20, &format!("{u:>5.1}%"), Type::BODY, Weight::Regular, cc);
        c.text(x + 330, cy + 20, &format!("{}MHz", core.frequency_mhz), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        let t = core_temp(o, core.id);
        if let Some(t) = t {
            c.text(x + 470, cy + 20, &format!("{t:.1}°C"), Type::LABEL, Weight::Regular, temp_level(t));
        }
        cy += row_h;
    }
}

fn core_temp(o: &SystemOverview, id: usize) -> Option<f64> {
    let key = format!("cpu{id}-thermal");
    o.thermal.iter().find(|t| t.name == key).map(|t| t.temp_celsius)
}

// ── GPU ──

fn card_gpu(c: &mut Canvas, p: &Pane, a: &Aux) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "GPU");
    let g = &a.hw.gpu;

    if g.suspended {
        // GPU 休眠状态
        c.text(x, y + 88, "休眠", Type::VALUE_L, Weight::Bold, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, y + 88, "Adreno 630", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.bar(x, y + 106, p.w - 44, 12, 0.0, Palette::FG_MUTED);
        c.text(
            x,
            y + 148,
            &format!("调速区间 {}–{}MHz · {}", g.min_freq_mhz, g.max_freq_mhz, g.governor),
            Type::LABEL,
            Weight::Regular,
            Palette::FG_MUTED,
        );
    } else {
        let color = if g.cur_freq_mhz >= g.max_freq_mhz && g.max_freq_mhz > 0 {
            Palette::WARNING
        } else {
            Palette::SUCCESS
        };
        c.text(x, y + 88, &format!("{}", g.cur_freq_mhz), Type::VALUE_L, Weight::Bold, color);
        let nw = c.fonts.text_width(&format!("{}", g.cur_freq_mhz), Type::VALUE_L, Weight::Bold);
        c.text(x + nw + 6, y + 88, "MHz", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, y + 88, "Adreno 630", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        let pct = if g.max_freq_mhz > 0 {
            g.cur_freq_mhz as f64 / g.max_freq_mhz as f64 * 100.0
        } else {
            0.0
        };
        c.bar(x, y + 106, p.w - 44, 12, pct, Palette::ACCENT);
        c.text(
            x,
            y + 148,
            &format!("调速区间 {}–{}MHz · {}", g.min_freq_mhz, g.max_freq_mhz, g.governor),
            Type::LABEL,
            Weight::Regular,
            Palette::FG_MUTED,
        );
    }
}

// ── 温度 ──

fn card_thermal(c: &mut Canvas, p: &Pane, o: &SystemOverview) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "温度");
    let mut zones: Vec<_> = o.thermal.iter().collect();
    zones.sort_by(|a, b| b.temp_celsius.partial_cmp(&a.temp_celsius).unwrap_or(std::cmp::Ordering::Equal));
    let half = (p.w - 44) / 2;
    for (i, t) in zones.iter().take(6).enumerate() {
        let col = i % 2;
        let row = i / 2;
        let zx = x + col as i32 * (half + 10);
        let zy = y + 62 + row as i32 * 38;
        let name = t.name.replace("-thermal", "");
        let color = temp_level(t.temp_celsius);
        let name_txt = truncate(c, &name, half - 92, Type::LABEL, Weight::Regular);
        c.text(zx, zy + 18, &name_txt, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(zx + half, zy + 18, &format!("{:.1}°C", t.temp_celsius), Type::BODY, Weight::Bold, color);
    }
}

// ── 内存 ──

fn card_memory(c: &mut Canvas, p: &Pane, o: &SystemOverview, h: &History) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "内存");
    let m = &o.memory;
    let color = level(m.usage_percent);
    c.text(x, y + 108, &format!("{:.1}", m.usage_percent), Type::VALUE_XL, Weight::Bold, color);
    let nw = c.fonts.text_width(&format!("{:.1}", m.usage_percent), Type::VALUE_XL, Weight::Bold);
    c.text(x + nw + 8, y + 108, "%", Type::VALUE_M, Weight::Regular, Palette::FG_MUTED);
    c.text(
        x + nw + 76,
        y + 108,
        &format!("{} / {} MB", m.used_mb, m.total_mb),
        Type::BODY,
        Weight::Regular,
        Palette::FG_DEFAULT,
    );
    let (msx, msw, msh) = (p.x + p.w - 260, 238, 58);
    c.rect(msx, y + 56, msw, msh, Palette::BG_SURFACE_ALT);
    c.hline(msx + 1, y + 56 + msh - 4, msw - 2, 1, Palette::BORDER);
    c.sparkline(msx + 6, y + 60, msw - 12, msh - 10, &h.mem, 100.0, color);
    c.bar(x, y + 126, p.w - 44, 14, m.usage_percent, color);
    c.text(
        x,
        y + 172,
        &format!(
            "可用 {} MB · 空闲 {} MB · 交换 {}/{} MB",
            m.available_mb, m.free_mb, m.swap_used_mb, m.swap_total_mb
        ),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

// ── 磁盘 ──

fn card_disk(c: &mut Canvas, p: &Pane, a: &Aux) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "磁盘");
    let root = a.disks.iter().find(|d| d.mount == "/").or_else(|| a.disks.first());
    if let Some(d) = root {
        let color = level(d.pct);
        c.text(x, y + 96, &format!("{:.0}", d.pct), Type::VALUE_L, Weight::Bold, color);
        let nw = c.fonts.text_width(&format!("{:.0}", d.pct), Type::VALUE_L, Weight::Bold);
        c.text(x + nw + 6, y + 96, "%", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text(
            x + nw + 62,
            y + 96,
            &format!("{} / {}", d.used, d.size),
            Type::BODY,
            Weight::Regular,
            Palette::FG_DEFAULT,
        );
        c.text_right(p.x + p.w - 22, y + 96, &format!("{} 挂载 {}", d.fstype, d.mount), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.bar(x, y + 114, p.w - 44, 14, d.pct, color);
        c.text(
            x,
            y + 158,
            &format!("可用 {} · 索引占用 {:.0}%", d.avail, d.inode_pct),
            Type::LABEL,
            Weight::Regular,
            Palette::FG_MUTED,
        );
    }
    // 块设备 I/O
    let mut ly = y + 200;
    for (dev, (r, w)) in a.disk_speeds.iter().take(2) {
        c.text(x, ly, &format!("{dev}"), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text(x + 150, ly, "读", Type::TINY, Weight::Regular, Palette::FG_MUTED);
        c.text(x + 186, ly, &fmt_speed(*r), Type::BODY, Weight::Regular, Palette::INFO);
        c.text(x + 330, ly, "写", Type::TINY, Weight::Regular, Palette::FG_MUTED);
        c.text(x + 366, ly, &fmt_speed(*w), Type::BODY, Weight::Regular, Palette::ACCENT_2);
        ly += 32;
    }
}

// ── 进程 ──

fn card_process(c: &mut Canvas, p: &Pane, o: &SystemOverview, a: &Aux) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "进程");
    c.text_right(p.x + p.w - 22, y + 38, &format!("共 {} 个", o.process_count), Type::LABEL, Weight::Regular, Palette::FG_MUTED);

    let mut list: Vec<&ProcessInfo> = a.processes.iter().collect();
    list.sort_by(|a, b| b.cpu_usage.partial_cmp(&a.cpu_usage).unwrap_or(std::cmp::Ordering::Equal));

    let (cx_pid, cx_cpu, cx_mem, cx_thr, cx_name) = (x, x + 96, x + 216, x + 350, x + 440);
    let hy = y + 74;
    c.text(cx_pid, hy, "PID", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    c.text(cx_cpu, hy, "CPU%", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    c.text(cx_mem, hy, "内存MB", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    c.text(cx_thr, hy, "线程", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    c.text(cx_name, hy, "名称", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    c.hline(x, y + 84, p.w - 44, 1, Palette::BORDER);

    let mut ry = y + 122;
    let name_w = p.w - 44 - (cx_name - x) - 8;
    for proc in list.iter().take(8) {
        let color = level(proc.cpu_usage as f64 * 1.6);
        c.text(cx_pid, ry, &format!("{}", proc.pid), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text(cx_cpu, ry, &format!("{:.1}", proc.cpu_usage), Type::BODY, Weight::Regular, color);
        c.text(cx_mem, ry, &format!("{}", proc.memory_mb), Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        c.text(cx_thr, ry, &format!("{}", proc.threads), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        let proc_name = truncate(c, &proc.name, name_w, Type::BODY, Weight::Regular);
        c.text(cx_name, ry, &proc_name, Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        ry += 34;
    }
}

// ── 电池 ──

fn card_battery(c: &mut Canvas, p: &Pane, o: &SystemOverview, a: &Aux) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "电池");
    let b = &o.battery;

    let (label, color) = match b.status.as_str() {
        "Charging" => ("充电中", Palette::SUCCESS),
        "Full" => ("已充满", Palette::SUCCESS),
        "Discharging" => ("放电中", Palette::ACCENT),
        _ => (b.status.as_str(), Palette::FG_MUTED),
    };
    c.pill(p.x + p.w - 160, y + 18, label, Type::TINY, color, Palette::BG_SURFACE_ALT);

    let pct = if b.display_capacity_pct > 0 { b.display_capacity_pct } else { b.capacity };
    let bcolor = if pct < 20 {
        Palette::ERROR
    } else if pct < 50 {
        Palette::WARNING
    } else {
        Palette::SUCCESS
    };
    c.text(x, y + 100, &format!("{pct}"), Type::VALUE_XL, Weight::Bold, bcolor);
    let nw = c.fonts.text_width(&format!("{pct}"), Type::VALUE_XL, Weight::Bold);
    c.text(x + nw + 8, y + 100, "%", Type::VALUE_M, Weight::Regular, Palette::FG_MUTED);
    c.text_right(
        p.x + p.w - 22,
        y + 100,
        &format!("{:.1}V  {:.0}mA", b.voltage_v, b.current_ma),
        Type::BODY,
        Weight::Regular,
        Palette::FG_DEFAULT,
    );
    c.bar(x, y + 118, p.w - 44, 16, pct as f64, bcolor);

    let cw = (p.w - 44) / 3;
    let remain = if b.time_left_min > 0 {
        format!("{}h{}m", b.time_left_min / 60, b.time_left_min % 60)
    } else if b.time_left_min < 0 {
        format!(
            "充{}h{}m",
            b.time_left_min.unsigned_abs() / 60,
            b.time_left_min.unsigned_abs() % 60
        )
    } else {
        "--".to_string()
    };
    let items = [
        ("功率", format!("{:.1} W", b.power_w), Palette::FG_DEFAULT),
        ("温度", format!("{:.1} °C", b.temp_celsius), temp_level(b.temp_celsius)),
        ("剩余", remain, Palette::FG_DEFAULT),
    ];
    for (i, (k, v, vc)) in items.iter().enumerate() {
        let kx = x + i as i32 * cw;
        c.text(kx, y + 168, k, Type::TINY, Weight::Regular, Palette::FG_MUTED);
        c.text(kx, y + 196, v, Type::VALUE_M, Weight::Bold, *vc);
    }

    c.text(
        x,
        y + 226,
        &format!(
            "上限 {}% · 充电功率上限 {:.1}W · 背光 {}%{}",
            b.effective_max_pct,
            a.hw.charging.power_w.max(0.0),
            a.hw.brightness.percent,
            if b.is_degraded { " · 容量已下降" } else { "" }
        ),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

// ── 网络 ──

fn card_network(c: &mut Canvas, p: &Pane, o: &SystemOverview, a: &Aux) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "网络与无线");

    let w = &a.wifi;
    if w.connected {
        let ssid = truncate(c, &w.ssid, p.w - 44 - 160, Type::BODY, Weight::Bold);
        c.text(x, y + 70, &ssid, Type::BODY, Weight::Bold, Palette::FG_EMPHASIS);
        c.dot(p.x + p.w - 32, y + 62, 8, Palette::SUCCESS);
        c.text_right(
            p.x + p.w - 52,
            y + 70,
            &format!("{} dBm", w.signal_dbm),
            Type::BODY,
            Weight::Regular,
            signal_color(w.signal_dbm),
        );
        let detail = truncate(
            c,
            &format!("{} {} 信道{}  {}", w.band, w.frequency_mhz, w.channel, w.bitrate),
            p.w - 44,
            Type::LABEL,
            Weight::Regular,
        );
        c.text(x, y + 100, &detail, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    } else {
        c.text(x, y + 70, "WiFi 未连接", Type::BODY, Weight::Bold, Palette::FG_MUTED);
    }

    // 接口速率（左）+ 蓝牙（右）同一行
    let ry = y + 134;
    for n in o.network.iter().filter(|n| n.is_up && n.name != "lo").take(1) {
        let (rx, tx) = a.net_speeds.get(&n.name).copied().unwrap_or((0.0, 0.0));
        c.text(x, ry, &n.name, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text(x + 120, ry, "↓", Type::TINY, Weight::Regular, Palette::INFO);
        c.text(x + 144, ry, &fmt_speed(rx), Type::BODY, Weight::Regular, Palette::INFO);
        c.text(x + 300, ry, "↑", Type::TINY, Weight::Regular, Palette::ACCENT_2);
        c.text(x + 324, ry, &fmt_speed(tx), Type::BODY, Weight::Regular, Palette::ACCENT_2);
    }
    let bt = if a.bt.powered {
        format!("蓝牙 已连 {}", a.bt.devices.iter().filter(|d| d.connected).count())
    } else {
        "蓝牙 关".to_string()
    };
    c.text_right(
        p.x + p.w - 22,
        ry,
        &bt,
        Type::LABEL,
        Weight::Regular,
        if a.bt.powered { Palette::ACCENT } else { Palette::FG_MUTED },
    );

    // VPN / 代理（固定行位；链路逐段清洗，避免 emoji/网址段撑破版面）
    c.hline(x, y + 160, p.w - 44, 1, Palette::BORDER);
    if o.mihomo.available {
        let kind = if o.mihomo.tun_enabled { "TUN" } else { "代理" };
        let node = clean_proxy(&o.mihomo.active_proxy);
        let node_txt = truncate(c, &node, p.w - 44 - 140, Type::BODY, Weight::Bold);
        c.text(x, y + 188, kind, Type::TINY, Weight::Regular, Palette::FG_MUTED);
        c.text(x + 52, y + 188, &node_txt, Type::BODY, Weight::Bold, Palette::SUCCESS);
        c.text_right(
            p.x + p.w - 22,
            y + 188,
            &format!("连接 {}", o.mihomo.connection_count),
            Type::LABEL,
            Weight::Regular,
            Palette::FG_MUTED,
        );
        c.text(
            x,
            y + 216,
            &format!("{} 模式", o.mihomo.mode),
            Type::LABEL,
            Weight::Regular,
            Palette::FG_MUTED,
        );
        c.text_right(
            p.x + p.w - 22,
            y + 216,
            &format!(
                "↓{} ↑{}",
                fmt_bytes(o.mihomo.download_total as f64),
                fmt_bytes(o.mihomo.upload_total as f64)
            ),
            Type::LABEL,
            Weight::Regular,
            Palette::FG_MUTED,
        );
        if !o.mihomo.proxy_chain.is_empty() {
            let chain: Vec<String> = o.mihomo.proxy_chain.iter().map(|s| clean_proxy(s)).collect();
            let chain_txt = truncate(c, &chain.join(" > "), p.w - 44, Type::TINY, Weight::Regular);
            c.text(x, y + 240, &format!("链路 {chain_txt}"), Type::TINY, Weight::Regular, Palette::ACCENT_2);
        }
    } else {
        c.text(x, y + 188, "VPN 未连接", Type::BODY, Weight::Regular, Palette::FG_MUTED);
    }
}

fn signal_color(dbm: i32) -> u32 {
    if dbm >= -60 {
        Palette::SUCCESS
    } else if dbm >= -75 {
        Palette::WARNING
    } else {
        Palette::ERROR
    }
}

// ── 服务与告警 ──

fn card_service(c: &mut Canvas, p: &Pane, a: &Aux) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "服务与告警");

    let mut sx = x;
    for (name, status) in &a.services {
        let (label, color) = match status.as_str() {
            "active" => ("运行", Palette::SUCCESS),
            "inactive" => ("停止", Palette::WARNING),
            "failed" => ("失败", Palette::ERROR),
            _ => ("未知", Palette::FG_MUTED),
        };
        let text = format!("{name} {label}");
        let w = c.fonts.text_width(&text, Type::TINY, Weight::Bold) + 34;
        c.round_rect(sx, y + 56, w, 32, 16, Palette::BG_SURFACE_ALT);
        c.dot(sx + 16, y + 72, 5, color);
        c.text(sx + 26, y + 78, &text, Type::TINY, Weight::Bold, Palette::FG_DEFAULT);
        sx += w + 10;
    }

    c.hline(x, y + 102, p.w - 44, 1, Palette::BORDER);

    // 底部硬件行占固定高度，告警条数据此推算，保证分隔线不会再压到文字
    let hw_top = p.y + p.h - 70;
    let alert_top = y + 136;
    let row_h = 34;
    let max_rows = (((hw_top - 16 - alert_top) / row_h) + 1).clamp(1, 3) as usize;

    if a.alerts.is_empty() {
        c.text(x, alert_top, "暂无告警", Type::BODY, Weight::Regular, Palette::FG_MUTED);
    }
    for (i, (lvl, t, msg, ts)) in a.alerts.iter().take(max_rows).enumerate() {
        let ry = alert_top + i as i32 * row_h;
        let color = match lvl.as_str() {
            "error" | "critical" => Palette::ERROR,
            "warning" => Palette::WARNING,
            _ => Palette::INFO,
        };
        c.dot(x + 7, ry - 6, 7, color);
        c.text(x + 26, ry, t, Type::BODY, Weight::Bold, color);
        let tw = c.fonts.text_width(t, Type::BODY, Weight::Bold);
        let when = chrono::DateTime::from_timestamp(*ts, 0)
            .map(|d| {
                use chrono::TimeZone;
                chrono::Local.from_utc_datetime(&d.naive_utc()).format("%H:%M").to_string()
            })
            .unwrap_or_default();
        let when_w = c.fonts.text_width(&when, Type::LABEL, Weight::Regular);
        let msg_max = (p.x + p.w - 22 - when_w - 30) - (x + 26 + tw + 14);
        let msg_txt = truncate(c, msg, msg_max.max(60), Type::LABEL, Weight::Regular);
        c.text(x + 26 + tw + 14, ry, &msg_txt, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, ry, &when, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    }

    c.hline(x, hw_top, p.w - 44, 1, Palette::BORDER);
    let hw = &a.hw;
    let on_off = |b: bool| if b { "开" } else { "关" };
    let mut hx = x;
    let items: [(&str, String, Argb); 4] = [
        (
            "屏幕",
            format!("{} {}%", on_off(hw.screen_on), hw.brightness.percent),
            Palette::FG_DEFAULT,
        ),
        (
            "充电",
            if hw.charging.charge_mode == "power_only" {
                "仅供电".to_string()
            } else if hw.charging.charger_online {
                "在线".to_string()
            } else {
                "离线".to_string()
            },
            if hw.charging.charger_online { Palette::SUCCESS } else { Palette::FG_MUTED },
        ),
        ("WiFi省电", on_off(hw.wifi_power_save.enabled).to_string(), Palette::FG_DEFAULT),
        (
            "扬声器",
            format!(
                "{} {}%",
                if hw.speaker.muted { "静音" } else { "正常" },
                hw.speaker.volume_percent
            ),
            Palette::FG_DEFAULT,
        ),
    ];
    for (k, v, vc) in items.iter() {
        c.text(hx, hw_top + 24, k, Type::TINY, Weight::Regular, Palette::FG_MUTED);
        c.text(hx, hw_top + 50, v, Type::LABEL, Weight::Regular, *vc);
        hx += (p.w - 44) / 4;
    }
}

// ── 工具 ──

pub(crate) fn truncate(c: &mut Canvas, s: &str, max_w: i32, size: f32, weight: Weight) -> String {
    if c.fonts.text_width(s, size, weight) <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    let ell = c.fonts.text_width("…", size, weight);
    let mut w = 0;
    for ch in s.chars() {
        let cw = c.fonts.advance(ch, size, weight).round() as i32;
        if w + cw + ell > max_w {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

pub fn fmt_bytes(bytes: f64) -> String {
    if bytes >= 1073741824.0 {
        format!("{:.1}GB", bytes / 1073741824.0)
    } else if bytes >= 1048576.0 {
        format!("{:.1}MB", bytes / 1048576.0)
    } else if bytes >= 1024.0 {
        format!("{:.0}KB", bytes / 1024.0)
    } else {
        format!("{:.0}B", bytes)
    }
}

pub fn fmt_speed(bps: f64) -> String {
    let bps = if bps > 0.0 { bps } else { 0.0 };
    if bps >= 1048576.0 {
        format!("{:.1} MB/s", bps / 1048576.0)
    } else if bps >= 1024.0 {
        format!("{:.0} KB/s", bps / 1024.0)
    } else {
        format!("{:.0} B/s", bps)
    }
}

fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{d}天{h}时{m}分")
    } else if h > 0 {
        format!("{h}时{m}分")
    } else {
        format!("{m}分")
    }
}

fn clean_proxy(name: &str) -> String {
    // 复用 TUI 已验证的清洗：国旗 emoji → ISO 码（SG/US…），去掉「网址:」段与变体选择符
    crate::tui::clean_proxy_name(name)
}

fn hostname() -> String {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "device".to_string())
    })
    .clone()
}

fn kernel() -> String {
    static K: OnceLock<String> = OnceLock::new();
    K.get_or_init(|| {
        let v = std::fs::read_to_string("/proc/version").unwrap_or_default();
        let mut it = v.split_whitespace();
        it.next();
        it.next();
        it.next().unwrap_or("unknown").to_string()
    })
    .clone()
}

/// `df -T` 解析磁盘行。
fn disk_rows() -> Vec<DiskRow> {
    let out = match std::process::Command::new("df").args(["-h", "-T"]).output() {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() >= 7 && p[0].starts_with("/dev/") {
            let pct = p[5].trim_end_matches('%').parse::<f64>().unwrap_or(0.0);
            let (_, _, _, inode_pct) = crate::collector::disk::get_inode_info(p[6]);
            rows.push(DiskRow {
                mount: p[6].to_string(),
                fstype: p[1].to_string(),
                size: p[2].to_string(),
                used: p[3].to_string(),
                avail: p[4].to_string(),
                pct,
                inode_pct,
            });
        }
    }
    rows
}
