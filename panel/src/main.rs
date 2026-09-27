//! device-monitor 物理屏面板（Slint + femtovg GPU 渲染）。
//!
//! 与旧 `--screen`（CPU 软绘 DRM 直绘）的关系：本进程是**独立面板**，
//! 通过本机 REST API（device-monitor-server :3000）与 XTokenHub（:9192）取数，
//! 用 Slint linuxkms 后端直驱 DRM，GPU 合成，支持触摸翻页/切主题。
//!
//! 运行（设备）：`SLINT_SCALE_FACTOR=2 SLINT_KMS_ROTATION=90 device-monitor-panel`
//! （环境变量缺省时代码内补默认值）。预览（macOS）：`cargo run -p device-monitor-panel -- --demo`。
//!
//! 数据节奏：
//! - overview 2s（含 CPU/内存历史、网速差分、电池）
//! - hardware 4s（GPU/充电/亮度/熄屏检测）、process 4s、disk+wifi 10s、services+alerts 30s
//! - XTokenHub REST 15s + WS 长连接（2Hz 吞吐、request.completed、stats.updated 限频刷新）
//! - 天气 curl 15min、世界时钟子进程 30s 缓存、秒级时钟 1s

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use serde_json::{json, Value};
use slint::{ModelRc, VecModel, Weak};

slint::include_modules!();

// ── 与页面约定固定的几何常量（改 .slint 必须同步）──
/// 吞吐面积图盒子（page-token.slint 实时吞吐卡）
const TPS_W: f32 = 309.0;
const TPS_H: f32 = 72.0;
/// 7 天趋势面积图盒子（质量指标卡）
const TREND_W: f32 = 353.0;
const TREND_H: f32 = 40.0;
/// 等化器柱数（CPU/内存历史，2s 采样 → 约 2 分钟窗口）
const HIST_BARS: usize = 60;
/// 吞吐滑窗点数（WS 2Hz → 约 30 秒）
const TPS_POINTS: usize = 60;

const BASE: &str = "http://127.0.0.1:3000/api";
const XTB: &str = "http://127.0.0.1:9192/api/v1";
const XTB_WS: &str = "ws://127.0.0.1:9192/api/v1/ws";

/// macOS 预览（苹方）的垂直居中补偿；设备值见 theme.slint
const MACOS_TEXT_NUDGE: f32 = 0.25;

/// 西安坐标（与旧 screen/clock.rs 一致）
const LAT: f64 = 34.3416;
const LON: f64 = 108.9398;
const SUN_H0_DEG: f64 = -0.833;

static REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);

// TPS 流限频：网关每 500ms 推一条 stats.throughput（2Hz），逐条刷 UI 会让
// Slint 渲染循环停不下来（每条消息 = 一次全屏重绘）。样本照常进滑窗，
// UI 落刷搭 1s 时钟 tick 的车（无新样本时完全不碰 UI，不产生渲染）。
static TPS_WIN: Mutex<VecDeque<f32>> = Mutex::new(VecDeque::new());
static TPS_STREAMS: AtomicI32 = AtomicI32::new(0);
static TPS_SEQ: AtomicU64 = AtomicU64::new(0);
static TPS_SEQ_DONE: AtomicU64 = AtomicU64::new(0);

fn main() {
    // linuxkms 后端在初始化时读环境变量；edition 2024 中 set_var 是 unsafe，
    // 此处尚无其他线程，设置是安全的。
    #[cfg(not(target_os = "macos"))]
    unsafe {
        if std::env::var_os("SLINT_SCALE_FACTOR").is_none() {
            std::env::set_var("SLINT_SCALE_FACTOR", "2");
        }
        if std::env::var_os("SLINT_KMS_ROTATION").is_none() {
            std::env::set_var("SLINT_KMS_ROTATION", "90");
        }
    }

    let demo = std::env::args().any(|a| a == "--demo");
    let page_arg = std::env::args()
        .position(|a| a == "--page")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);

    let app = App::new().expect("无法创建 Slint 组件（检查字体/后端依赖）");
    let weak = app.as_weak();

    // 字体与文字垂直居中补偿：.slint 里的默认值即设备值（Noto Sans CJK SC）；
    // macOS 预览字体不同（苹方），行盒度量随之不同，这里按苹方重新校准。
    // PANEL_FONT_BODY / PANEL_FONT_MONO / PANEL_TEXT_NUDGE 可现场覆盖，便于逐像素校准。
    {
        let theme = app.global::<Theme>();
        if cfg!(target_os = "macos") {
            theme.set_font_body("PingFang SC".into());
            theme.set_font_mono("Menlo".into());
            theme.set_text_nudge(MACOS_TEXT_NUDGE);
        }
        if let Ok(v) = std::env::var("PANEL_FONT_BODY") {
            theme.set_font_body(v.into());
        }
        if let Ok(v) = std::env::var("PANEL_FONT_MONO") {
            theme.set_font_mono(v.into());
        }
        if let Some(px) = std::env::var("PANEL_TEXT_NUDGE").ok().and_then(|v| v.parse::<f32>().ok()) {
            theme.set_text_nudge(px);
        }
    }

    // 主题持久化（panel-theme.txt，与旧屏 theme.txt 互不干扰）
    let dark = std::fs::read_to_string("panel-theme.txt")
        .map(|s| s.trim() != "light")
        .unwrap_or(true);
    app.global::<Theme>().set_dark(dark);
    app.global::<Ui>().set_page(page_arg.clamp(0, 2));
    {
        let w = weak.clone();
        app.global::<Ui>().on_toggle_theme(move || {
            let Some(ui) = w.upgrade() else { return };
            let theme = ui.global::<Theme>();
            let dark = !theme.get_dark();
            theme.set_dark(dark);
            let _ = std::fs::write("panel-theme.txt", if dark { "dark" } else { "light" });
        });
    }
    // 触摸调试探针：触点逻辑坐标落 panel.log（部署期诊断，代价可忽略）
    app.global::<Ui>().on_probe(|x, y, kind| {
        eprintln!("[touch-probe] {kind} x={x:.1} y={y:.1} (logical 1170x540)");
    });

    // 秒级时钟 + 时间进度 + 日出相位 + 世界时钟（子进程 30s 缓存）
    let timer = slint::Timer::default();
    {
        let w = weak.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            tick_clock(&w);
        });
    }

    if demo {
        spawn_demo(weak.clone());
    } else {
        spawn_system_poller(weak.clone());
        spawn_token_poller(weak.clone());
        spawn_token_ws(weak.clone());
        spawn_weather_feed(weak.clone());
    }

    // 启动即拉一次刷新间隔设置（仅用于展示）
    {
        let w = weak.clone();
        std::thread::spawn(move || {
            if let Ok(v) = fetch_json(&format!("{BASE}/system/refresh")) {
                let secs = jf(&v, "refresh_secs") as i32;
                w.upgrade_in_event_loop(move |ui| {
                    ui.global::<Sys>().set_refresh_secs(secs);
                })
                .ok();
            }
        });
    }

    app.run().unwrap();
}

// ────────────────────────── JSON 小工具 ──────────────────────────

fn jf(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0)
}
fn ji(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(|x| x.as_i64()).unwrap_or(0)
}
fn js(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string()
}
fn jb(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}

/// 统一响应包装 `{code, data}`；XTokenHub 与 device-monitor 同构。
fn fetch_json(url: &str) -> Result<Value, String> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(4))
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let text = resp.into_string().map_err(|e| format!("读取 {url}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("解析 {url}: {e}"))?;
    if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
        return Err(format!("{url} code!=0"));
    }
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

fn model<T: 'static + Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(rows))
}

// ────────────────────────── 格式化 ──────────────────────────

fn fmt_speed(bps: f64) -> String {
    let bps = bps.max(0.0);
    if bps >= 1_048_576.0 {
        format!("{:.1} MB/s", bps / 1_048_576.0)
    } else if bps >= 1024.0 {
        format!("{:.0} KB/s", bps / 1024.0)
    } else {
        format!("{:.0} B/s", bps)
    }
}

fn fmt_tokens(n: i64) -> String {
    let v = n as f64;
    if v >= 1e8 {
        format!("{:.2}亿", v / 1e8)
    } else if v >= 1e7 {
        format!("{:.0}万", v / 1e4)
    } else if v >= 1e4 {
        format!("{:.1}万", v / 1e4)
    } else {
        format!("{n}")
    }
}

fn fmt_mb(mb: i64) -> String {
    if mb >= 1024 {
        format!("{:.1} GB", mb as f64 / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

fn fmt_uptime(secs: i64) -> String {
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

/// 复用 TUI 已验证的代理名清洗：国旗 emoji → ISO 码、去变体选择符与「网址:」段。
fn clean_proxy(name: &str) -> String {
    let mut out = String::new();
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if ('\u{1F1E6}'..='\u{1F1FF}').contains(&c) {
            if let Some(&n) = chars.peek()
                && ('\u{1F1E6}'..='\u{1F1FF}').contains(&n)
            {
                out.push(char::from_u32(c as u32 - 0x1F1E6 + 65).unwrap_or('?'));
                out.push(char::from_u32(n as u32 - 0x1F1E6 + 65).unwrap_or('?'));
                chars.next();
                continue;
            }
            continue;
        }
        if c == '\u{FE0F}' || c == '\u{200D}' {
            continue;
        }
        out.push(c);
    }
    if let Some(pos) = out.find("://") {
        let tail = &out[pos..];
        let cut = tail.find(' ').map(|e| pos + e).unwrap_or(out.len());
        out = format!("{}{}", out[..pos].trim_end(), &out[cut..]);
    }
    out.trim().to_string()
}

// ────────────────────────── 图形路径生成 ──────────────────────────

/// 折线 Path commands（值自动按 max 缩放到盒高，底部留 2px）。
fn polyline(vals: &[f32], w: f32, h: f32, col: &str) -> (String, String) {
    let n = vals.len();
    if n < 2 {
        return (String::new(), String::new());
    }
    let max = (vals.iter().cloned().fold(1.0f32, f32::max) * 1.15).max(1.0);
    let step = (w - 6.0) / (n - 1) as f32;
    let mut pts = String::new();
    for (i, v) in vals.iter().enumerate() {
        let x = 3.0 + i as f32 * step;
        let y = h - 2.0 - (v.max(0.0) / max).clamp(0.0, 1.0) * (h - 6.0);
        pts.push_str(&if i == 0 {
            format!("M {x:.1} {y:.1} ")
        } else {
            format!("L {x:.1} {y:.1} ")
        });
    }
    let line = pts.trim().to_string();
    let area = format!("{line} L {:.1} {:.1} L 3.0 {:.1} Z", w - 3.0, h - 1.0, h - 1.0);
    let _ = col;
    (line, area)
}

/// 环形仪表值弧（120x120 盒，圆心 60,60 半径 53，12 点起顺时针）。
fn ring_arc(pct: f64) -> String {
    let p = (pct / 100.0).clamp(0.0, 1.0);
    if p <= 0.004 {
        return String::new();
    }
    let full = p >= 0.996;
    let theta = if full { std::f64::consts::TAU - 0.02 } else { p * std::f64::consts::TAU };
    let x = 60.0 + 53.0 * theta.sin();
    let y = 60.0 - 53.0 * theta.cos();
    let large = if theta > std::f64::consts::PI { 1 } else { 0 };
    format!("M 60 7 A 53 53 0 {large} 1 {x:.2} {y:.2}")
}

// ────────────────────────── 时钟页 tick ──────────────────────────

struct SunState {
    rise: f64,
    set: f64,
    noon: f64,
    day_len: f64,
    tomorrow: Option<(f64, f64)>,
    from_api: bool,
    at: Instant,
}

static SUN: OnceLock<Mutex<Option<SunState>>> = OnceLock::new();
static WORLD: OnceLock<Mutex<WorldCache>> = OnceLock::new();

type CityRowSigned = (String, String, String, String); // (name, mark, offset, time)
type WorldCache = Option<(Instant, Vec<CityRowSigned>)>;

fn sun() -> &'static Mutex<Option<SunState>> {
    SUN.get_or_init(|| Mutex::new(None))
}
fn world() -> &'static Mutex<WorldCache> {
    WORLD.get_or_init(|| Mutex::new(None))
}

/// NOAA 简化太阳位置公式（移植自 screen/clock.rs，西安坐标）。
fn sun_times(date: NaiveDate) -> Option<(f64, f64)> {
    const DEG: f64 = std::f64::consts::PI / 180.0;
    let days = (date - NaiveDate::from_ymd_opt(2000, 1, 1)?).num_days() as f64;
    let n = days + 0.0008;
    let j_star = n - LON / 360.0;
    let m = (357.5291 + 0.985_600_28 * j_star).rem_euclid(360.0);
    let c = 1.9148 * (m * DEG).sin() + 0.0200 * (2.0 * m * DEG).sin() + 0.0003 * (3.0 * m * DEG).sin();
    let lambda = (m + c + 180.0 + 102.9372).rem_euclid(360.0);
    let j_transit = 2_451_545.0 + j_star + 0.0053 * (m * DEG).sin() - 0.0069 * (2.0 * lambda * DEG).sin();
    let sin_delta = (lambda * DEG).sin() * (23.44 * DEG).sin();
    let cos_delta = (1.0 - sin_delta * sin_delta).sqrt();
    let h0 = SUN_H0_DEG * DEG;
    let cos_omega = (h0.sin() - (LAT * DEG).sin() * sin_delta) / ((LAT * DEG).cos() * cos_delta);
    if !(-1.0..=1.0).contains(&cos_omega) {
        return None;
    }
    let omega = cos_omega.acos() / DEG;
    let tz = Local::now().offset().local_minus_utc() as f64 / 3600.0;
    let to_local = |j: f64| ((j + 0.5 + tz / 24.0).rem_euclid(1.0)) * 24.0;
    Some((to_local(j_transit - omega / 360.0), to_local(j_transit + omega / 360.0)))
}

fn solar_noon(date: NaiveDate) -> Option<f64> {
    sun_times(date).map(|_| {
        const DEG: f64 = std::f64::consts::PI / 180.0;
        let days = (date - NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()).num_days() as f64;
        let n = days + 0.0008;
        let j_star = n - LON / 360.0;
        let m = (357.5291 + 0.985_600_28 * j_star).rem_euclid(360.0);
        let c = 1.9148 * (m * DEG).sin() + 0.0200 * (2.0 * m * DEG).sin() + 0.0003 * (3.0 * m * DEG).sin();
        let lambda = (m + c + 180.0 + 102.9372).rem_euclid(360.0);
        let j_transit = 2_451_545.0 + j_star + 0.0053 * (m * DEG).sin()
            - 0.0069 * (2.0 * lambda * DEG).sin();
        let tz = Local::now().offset().local_minus_utc() as f64 / 3600.0;
        ((j_transit + 0.5 + tz / 24.0).rem_euclid(1.0)) * 24.0
    })
}

fn hhmm(hours: f64) -> String {
    let h = hours.rem_euclid(24.0);
    let m = (h.fract() * 60.0).round() as i64;
    let h = h as i64 + m / 60;
    format!("{:02}:{:02}", h % 24, m % 60)
}

fn hm_span(hours: f64) -> String {
    let total = (hours * 60.0).round().max(0.0) as i64;
    format!("{}h{:02}m", total / 60, total % 60)
}

fn parse_hhmm(s: &str) -> Option<f64> {
    let (h, m) = s.split_once(':')?;
    Some(h.trim().parse::<f64>().ok()? + m.trim().parse::<f64>().ok()? / 60.0)
}

fn days_in_year(y: i32) -> i64 {
    if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 }
}

fn month_days(y: i32, m: u32) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - NaiveDate::from_ymd_opt(y, m, 1).unwrap())
        .num_days()
}

fn short_span(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    if s >= 86400 {
        let d = s / 86400;
        let h = (s % 86400) / 3600;
        if h > 0 { format!("{d} 天 {h} 时") } else { format!("{d} 天") }
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

fn tick_clock(w: &Weak<App>) {
    let Some(ui) = w.upgrade() else { return };
    let clk = ui.global::<Clk>();
    let now = Local::now();

    clk.set_hh(now.format("%H").to_string().into());
    clk.set_mm(now.format("%M").to_string().into());
    clk.set_ss(now.format("%S").to_string().into());
    let sec = now.second();
    clk.set_sec_pct(sec as f32 / 60.0 * 100.0);
    clk.set_sec_str(format!("第 {sec} 秒 / 60").into());

    // 呼吸/脉冲相位：1Hz 步进（替代 iteration-count:-1 无限动画；后者令渲染循环不空闲）
    let secf = sec as f32;
    let breathe = |t: f32, period: f32| 0.5 - 0.5 * (std::f32::consts::TAU * t / period).cos();
    clk.set_glow_a(0.78 + 0.17 * breathe(secf, 10.4));
    clk.set_glow_b(0.40 + 0.12 * breathe(secf + 3.2, 12.8));
    clk.set_sun_halo(0.45 + 0.50 * breathe(secf, 2.0));
    ui.global::<Ui>().set_live_pulse(0.30 + 0.65 * breathe(secf, 2.0));

    // XTokenHub TPS：把 WS 2Hz 样本限量到 1Hz 落 UI（无新样本则不碰，不产生渲染）
    {
        let seq = TPS_SEQ.load(Ordering::Relaxed);
        if seq != TPS_SEQ_DONE.swap(seq, Ordering::Relaxed) {
            let vals: Vec<f32> = {
                let wnd = TPS_WIN.lock().unwrap_or_else(|p| p.into_inner());
                wnd.iter().copied().collect()
            };
            let (line, area) = polyline(&vals, TPS_W, TPS_H, "");
            let cur = vals.last().copied().unwrap_or(0.0);
            let avg = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f32>() / vals.len() as f32 };
            let peak = vals.iter().cloned().fold(0.0f32, f32::max);
            let info = format!("滑窗均值 {avg:.0} · 峰值 {peak:.0} tokens/s · 采样 {}/{TPS_POINTS}", vals.len());
            let tok = ui.global::<Tok>();
            tok.set_tps(cur);
            tok.set_tps_str(format!("{cur:.0}").into());
            tok.set_streams(TPS_STREAMS.load(Ordering::Relaxed));
            tok.set_tps_line(line.into());
            tok.set_tps_area(area.into());
            tok.set_tps_info(info.into());
        }
    }

    let wd = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
        [now.weekday().num_days_from_monday() as usize];
    clk.set_date_line(
        format!("{}月{}日  {wd}", now.month(), now.day()).into(),
    );

    let left_sec = 86400.0
        - (now.hour() as f64 * 3600.0 + now.minute() as f64 * 60.0 + now.second() as f64);
    let ls = left_sec as i64;
    clk.set_meta1(
        format!(
            "第 {} 周 · 第 {} 天 / {} · 距零点 {:02}:{:02}:{:02}",
            now.iso_week().week(),
            now.ordinal(),
            days_in_year(now.year()),
            ls / 3600,
            (ls % 3600) / 60,
            ls % 60
        )
        .into(),
    );
    clk.set_meta2(
        format!(
            "时区 {} {} · 时间戳 {} · 第 {} 季度",
            now.format("%Z"),
            now.format("%:z"),
            now.timestamp(),
            (now.month() - 1) / 3 + 1
        )
        .into(),
    );

    // 时间进度（今日/本周/本月/本年）
    let sec_day = now.hour() as f64 * 3600.0 + now.minute() as f64 * 60.0 + now.second() as f64;
    let wd_f = now.weekday().num_days_from_monday() as f64;
    let dim = month_days(now.year(), now.month()) as f64;
    let doy = now.ordinal() as f64;
    let diy = days_in_year(now.year()) as f64;
    let rows = vec![
        ("今日", sec_day / 86400.0 * 100.0, short_span(86400.0 - sec_day)),
        ("本周", (wd_f * 86400.0 + sec_day) / (7.0 * 86400.0) * 100.0, short_span(7.0 * 86400.0 - wd_f * 86400.0 - sec_day)),
        ("本月", ((now.day() as f64 - 1.0) * 86400.0 + sec_day) / (dim * 86400.0) * 100.0, short_span((dim - now.day() as f64) * 86400.0 + left_sec)),
        ("本年", ((doy - 1.0) * 86400.0 + sec_day) / (diy * 86400.0) * 100.0, short_span((diy - doy) * 86400.0 + left_sec)),
    ];
    let prog: Vec<crate::ProgRow> = rows
        .into_iter()
        .map(|(l, p, r)| crate::ProgRow {
            label: l.into(),
            pct: p as f32,
            pct_str: format!("{p:.1}").into(),
            left: r.into(),
        })
        .collect();
    clk.set_prog(model(prog));

    // 世界时钟（30s 缓存子进程）
    {
        let mut g = world().lock().unwrap_or_else(|p| p.into_inner());
        let stale = g.as_ref().map(|(t, _)| t.elapsed() >= Duration::from_secs(30)).unwrap_or(true);
        if stale {
            *g = Some((Instant::now(), fetch_world()));
        }
        if let Some((_, rows)) = g.as_ref() {
            let cities: Vec<crate::CityRow> = rows
                .iter()
                .map(|(n, m, o, t)| crate::CityRow {
                    name: n.into(),
                    mark: m.into(),
                    offset: o.into(),
                    time: t.into(),
                })
                .collect();
            clk.set_world(model(cities));
        }
    }

    // 日出日落：30s 重算一次（吸收天气更新），相位每秒推进
    {
        let mut g = sun().lock().unwrap_or_else(|p| p.into_inner());
        let stale = g.as_ref().map(|s| s.at.elapsed() >= Duration::from_secs(30)).unwrap_or(true);
        if stale {
            let today = now.date_naive();
            let (rise, set, from_api) = weather_sun(today);
            let noon = solar_noon(today).unwrap_or(12.0);
            let tomorrow = weather_sun2().or_else(|| today.succ_opt().and_then(sun_times));
            *g = Some(SunState {
                rise,
                set,
                noon,
                day_len: (set - rise).rem_euclid(24.0),
                tomorrow,
                from_api,
                at: Instant::now(),
            });
        }
        if let Some(s) = g.as_ref() {
            let now_h = now.hour() as f64 + now.minute() as f64 / 60.0 + now.second() as f64 / 3600.0;
            let is_day = now_h >= s.rise && now_h <= s.set;
            let night_len = (24.0 - s.day_len).max(0.01);
            let (pct, line) = if is_day {
                (
                    (now_h - s.rise) / s.day_len * 100.0,
                    format!("白天 · 剩余 {}", hm_span(s.set - now_h)),
                )
            } else {
                let elapsed = if now_h < s.rise { 24.0 - s.set + now_h } else { now_h - s.set };
                (
                    elapsed / night_len * 100.0,
                    format!("夜间 · 距日出 {}", hm_span((24.0 - now_h + s.rise).rem_euclid(24.0))),
                )
            };
            clk.set_sun_rise(hhmm(s.rise).into());
            clk.set_sun_set(hhmm(s.set).into());
            clk.set_day_len(hm_span(s.day_len).into());
            clk.set_noon(hhmm(s.noon).into());
            clk.set_is_day(is_day);
            clk.set_phase_pct(pct as f32);
            clk.set_phase_line(line.into());
            clk.set_sun_src(if s.from_api { "来源 天气接口" } else { "来源 本地推算" }.into());
            if let Some((r2, s2)) = s.tomorrow {
                let len2 = (s2 - r2).rem_euclid(24.0);
                let delta = ((len2 - s.day_len) * 60.0).round() as i64;
                clk.set_tomorrow(
                    format!(
                        "明日 {} / {} · 昼长 {}（{}{}m）",
                        hhmm(r2),
                        hhmm(s2),
                        hm_span(len2),
                        if delta > 0 { "+" } else { "" },
                        delta
                    )
                    .into(),
                );
            }
            // 弧线（固定 420x76 盒）+ 太阳位置
            clk.set_sun_arc("M 12 66 L 408 66 M 12 66 A 377.5 377.5 0 0 1 408 66".into());
            let t = if is_day {
                ((now_h - s.rise) / s.day_len).clamp(0.0, 1.0)
            } else if now_h < s.rise {
                0.0
            } else {
                1.0
            };
            let half = (198.0_f64 / 377.5).asin();
            let phi = -half + t * 2.0 * half;
            clk.set_sun_x((210.0 + 377.5 * phi.sin()) as f32);
            clk.set_sun_y((387.5 - 377.5 * phi.cos()) as f32);
        }
    }
}

type SunPair = Option<(f64, f64)>;

static WEATHER: OnceLock<Mutex<WeatherNow>> = OnceLock::new();

#[derive(Default, Clone)]
struct WeatherNow {
    ok: bool,
    #[allow(dead_code)]
    updated: Option<Instant>,
    temp_c: f64,
    feels_c: f64,
    humidity: f64,
    code: u32,
    wind_ms: f64,
    wind_dir: f64,
    t_max: f64,
    t_min: f64,
    precip_prob: f64,
    precip_mm: f64,
    sunrise: Option<String>,
    sunset: Option<String>,
    sunrise2: Option<String>,
    sunset2: Option<String>,
}

fn weather_cell() -> &'static Mutex<WeatherNow> {
    WEATHER.get_or_init(|| Mutex::new(WeatherNow::default()))
}

/// 当日 (日出, 日落, 是否来自天气接口)：优先接口，退回本地推算。
fn weather_sun(today: NaiveDate) -> (f64, f64, bool) {
    let g = weather_cell().lock().unwrap_or_else(|p| p.into_inner());
    if let (Some(r), Some(st)) = (g.sunrise.as_deref(), g.sunset.as_deref())
        && let (Some(rh), Some(sh)) = (parse_hhmm(r), parse_hhmm(st))
    {
        return (rh, sh, true);
    }
    drop(g);
    let (r, s) = sun_times(today).unwrap_or((6.5, 18.5));
    (r, s, false)
}

fn weather_sun2() -> SunPair {
    let g = weather_cell().lock().unwrap_or_else(|p| p.into_inner());
    let r = g.sunrise2.as_deref().and_then(parse_hhmm);
    let s = g.sunset2.as_deref().and_then(parse_hhmm);
    r.zip(s)
}

fn fetch_world() -> Vec<CityRowSigned> {
    const CITIES: &[(&str, &str)] = &[
        ("东京", "Asia/Tokyo"),
        ("伦敦", "Europe/London"),
        ("纽约", "America/New_York"),
        ("洛杉矶", "America/Los_Angeles"),
        ("巴黎", "Europe/Paris"),
        ("UTC", "UTC"),
    ];
    let today = Local::now().date_naive();
    let mut out = Vec::new();
    for (name, tz) in CITIES {
        let ok = std::process::Command::new("date")
            .env("TZ", tz)
            .arg("+%H:%M|%z|%Y-%m-%d")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        match ok {
            Some(s) => {
                let mut it = s.split('|');
                let time = it.next().unwrap_or("--:--").to_string();
                let raw = it.next().unwrap_or("?");
                // +0900 → +09:00（%z 两侧平台都支持）
                let offset = if raw.len() == 5 {
                    format!("{}:{}", &raw[..3], &raw[3..])
                } else {
                    raw.to_string()
                };
                let mark = it
                    .next()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
                    .map(|d| match (d - today).num_days() {
                        -1 => " · 昨天",
                        1 => " · 明天",
                        _ => "",
                    })
                    .unwrap_or("");
                out.push((name.to_string(), mark.to_string(), offset, time));
            }
            None => out.push((name.to_string(), String::new(), "?".into(), "--:--".into())),
        }
    }
    out
}

/// 天气线程：curl Open-Meteo（设备无 TLS 版 ureq，直连失败再试绕代理），15 分钟一刷。
fn spawn_weather_feed(w: Weak<App>) {
    std::thread::Builder::new()
        .name("weather-feed".into())
        .spawn(move || loop {
            if let Some(wv) = fetch_weather() {
                let ok = wv.ok;
                let note = if ok {
                    "西安 · Open-Meteo".to_string()
                } else {
                    "离线 · 显示上次数据".to_string()
                };
                let (temp_s, desc, wet) = if ok {
                    (
                        format!("{:.0}", wv.temp_c),
                        code_desc(wv.code).to_string(),
                        is_wet(wv.code),
                    )
                } else {
                    ("--".into(), "--".into(), false)
                };
                let feels = format!("体感 {:.0}°", wv.feels_c);
                let wind = format!("{} {:.1} m/s", wind_dir_cn(wv.wind_dir), wv.wind_ms);
                let range = format!(
                    "今日 {:.0}~{:.0}°C · 降水概率 {:.0}%",
                    wv.t_min, wv.t_max, wv.precip_prob
                );
                let extra = format!(
                    "湿度 {:.0}% · 降水 {:.1}mm · 日出 {} · 日落 {}",
                    wv.humidity,
                    wv.precip_mm,
                    wv.sunrise.clone().unwrap_or_else(|| "--:--".into()),
                    wv.sunset.clone().unwrap_or_else(|| "--:--".into()),
                );
                {
                    let mut g = weather_cell().lock().unwrap_or_else(|p| p.into_inner());
                    if ok {
                        *g = wv.clone();
                    }
                }
                let ok2 = ok;
                w.upgrade_in_event_loop(move |ui| {
                    let clk = ui.global::<Clk>();
                    clk.set_w_ok(ok2);
                    clk.set_w_temp(temp_s.into());
                    clk.set_w_desc(desc.into());
                    clk.set_w_wet(wet);
                    clk.set_w_feels(feels.into());
                    clk.set_w_wind(wind.into());
                    clk.set_w_range(range.into());
                    clk.set_w_extra(extra.into());
                    clk.set_w_note(note.into());
                })
                .ok();
            }
            std::thread::sleep(Duration::from_secs(900));
        })
        .expect("weather-feed 线程启动失败");
}

fn fetch_weather() -> Option<WeatherNow> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={LAT}&longitude={LON}\
         &current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,\
wind_speed_10m,wind_direction_10m,precipitation\
         &daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset\
         &wind_speed_unit=ms&timezone=Asia%2FShanghai&forecast_days=2"
    );
    let attempts: [Vec<&str>; 2] = [
        vec!["-s", "--max-time", "20", "-H", "Accept: application/json", &url],
        vec!["-s", "--max-time", "20", "--noproxy", "*", "-H", "Accept: application/json", &url],
    ];
    for args in &attempts {
        if let Ok(out) = std::process::Command::new("curl").args(args).output()
            && out.status.success()
            && let Some(w) = parse_weather(&String::from_utf8_lossy(&out.stdout))
        {
            return Some(WeatherNow {
                ok: true,
                updated: Some(Instant::now()),
                ..w
            });
        }
    }
    // 失败：保留上次数据（ok 置 false 让界面标注离线）
    let g = weather_cell().lock().unwrap_or_else(|p| p.into_inner());
    let mut w = g.clone();
    w.ok = false;
    drop(g);
    Some(w)
}

fn parse_weather(body: &str) -> Option<WeatherNow> {
    let v: Value = serde_json::from_str(body).ok()?;
    let cur = v.get("current")?;
    let daily = v.get("daily")?;
    let num = |o: &Value, k: &str| o.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let nth = |o: &Value, k: &str, i: usize| -> Option<String> {
        o.get(k)?.as_array()?.get(i)?.as_str().map(|s| s.to_string())
    };
    let first_num = |o: &Value, k: &str| {
        o.get(k)
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let hhmm_of = |s: Option<String>| -> Option<String> {
        s.and_then(|s| s.split('T').nth(1).map(|t| t.chars().take(5).collect()))
    };
    Some(WeatherNow {
        temp_c: num(cur, "temperature_2m"),
        feels_c: num(cur, "apparent_temperature"),
        humidity: num(cur, "relative_humidity_2m"),
        code: num(cur, "weather_code") as u32,
        wind_ms: num(cur, "wind_speed_10m"),
        wind_dir: num(cur, "wind_direction_10m"),
        t_max: first_num(daily, "temperature_2m_max"),
        t_min: first_num(daily, "temperature_2m_min"),
        precip_prob: first_num(daily, "precipitation_probability_max"),
        precip_mm: num(cur, "precipitation"),
        sunrise: hhmm_of(nth(daily, "sunrise", 0)),
        sunset: hhmm_of(nth(daily, "sunset", 0)),
        sunrise2: hhmm_of(nth(daily, "sunrise", 1)),
        sunset2: hhmm_of(nth(daily, "sunset", 1)),
        ..Default::default()
    })
}

fn code_desc(c: u32) -> &'static str {
    match c {
        0 => "晴",
        1 => "少云",
        2 => "多云",
        3 => "阴",
        45 | 48 => "雾",
        51 => "小毛毛雨",
        53 => "毛毛雨",
        55 => "大毛毛雨",
        61 => "小雨",
        63 => "中雨",
        65 => "大雨",
        66 | 67 => "冻雨",
        71 => "小雪",
        73 => "中雪",
        75 => "大雪",
        80 => "小阵雨",
        81 => "中阵雨",
        82 => "大阵雨",
        95 => "雷阵雨",
        96 | 99 => "雷阵雨伴冰雹",
        _ => "未知",
    }
}

fn wind_dir_cn(deg: f64) -> &'static str {
    const DIRS: [&str; 8] = ["北", "东北", "东", "东南", "南", "西南", "西", "西北"];
    let idx = (((deg % 360.0) + 360.0) % 360.0 / 45.0).round() as usize % 8;
    DIRS[idx]
}

fn is_wet(c: u32) -> bool {
    matches!(c, 51..=67 | 71..=77 | 80..=82 | 85 | 86 | 95..=99)
}

// ────────────────────────── 系统轮询线程 ──────────────────────────

/// hardware 侧最新值（apply_overview 组合电池胶囊/底部信息行时读取；
/// protocol_label 只在 hardware 接口里有，overview 的 battery 节点没有）。
struct HwLatest {
    charger_online: bool,
    chg_vi: String,
    brightness: String,
    protocol: String,
}

static HW_LATEST: Mutex<HwLatest> = Mutex::new(HwLatest {
    charger_online: false,
    chg_vi: String::new(),
    brightness: String::new(),
    protocol: String::new(),
});

fn spawn_system_poller(w: Weak<App>) {
    std::thread::Builder::new()
        .name("sys-poll".into())
        .spawn(move || {
            let mut prev_net: HashMap<String, (u64, u64, i64)> = HashMap::new();
            let mut cpu_hist: Vec<f32> = Vec::new();
            let mut mem_hist: Vec<f32> = Vec::new();
            let mut fail = 0u32;
            let mut it = 0u64;
            loop {
                it += 1;
                match fetch_json(&format!("{BASE}/system/overview")) {
                    Ok(ov) => {
                        fail = 0;
                        let ts = jf(&ov, "timestamp") as i64;
                        // 网速差分
                        let mut rx_sum = 0.0f64;
                        let mut tx_sum = 0.0f64;
                        let mut ip = "--".to_string();
                        if let Some(nets) = ov.get("network").and_then(|v| v.as_array()) {
                            for n in nets {
                                if !jb(n, "is_up") || js(n, "name") == "lo" {
                                    continue;
                                }
                                let name = js(n, "name");
                                let rx = jf(n, "rx_bytes") as u64;
                                let tx = jf(n, "tx_bytes") as u64;
                                if ip == "--"
                                    && let Some(v4) = n
                                        .get("ip_addresses")
                                        .and_then(|v| v.as_array())
                                        .and_then(|a| a.iter().find(|i| !i.as_str().unwrap_or("").contains(':')))
                                        .and_then(|i| i.as_str())
                                {
                                    ip = v4.to_string();
                                }
                                if let Some(&(prx, ptx, pts)) = prev_net.get(&name) {
                                    let dt = (ts - pts) as f64;
                                    if dt > 0.0 {
                                        rx_sum += rx.saturating_sub(prx) as f64 / dt;
                                        tx_sum += tx.saturating_sub(ptx) as f64 / dt;
                                    }
                                }
                                prev_net.insert(name, (rx, tx, ts));
                            }
                        }
                        // 历史环形（等化器）
                        let cpu = ov.pointer("/cpu/overall_usage").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let mem_pct = ov
                            .pointer("/memory/usage_percent")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        push_hist(&mut cpu_hist, cpu as f32, HIST_BARS);
                        push_hist(&mut mem_hist, mem_pct as f32, HIST_BARS);

                        apply_overview(&w, &ov, &ip, rx_sum, tx_sum, &cpu_hist, &mem_hist);
                    }
                    Err(e) => {
                        fail += 1;
                        tracing_or_log(&format!("overview 拉取失败({fail}): {e}"));
                        if fail >= 3 {
                            let w2 = w.clone();
                            w2.upgrade_in_event_loop(|ui| {
                                ui.global::<Sys>().set_online(false);
                            })
                            .ok();
                        }
                    }
                }

                if it.is_multiple_of(2) {
                    if let Ok(v) = fetch_json(&format!("{BASE}/hardware")) {
                        apply_hardware(&w, &v);
                    }
                    if let Ok(v) = fetch_json(&format!("{BASE}/process")) {
                        apply_process(&w, &v);
                    }
                }
                if it.is_multiple_of(5) {
                    if let Ok(v) = fetch_json(&format!("{BASE}/disk")) {
                        apply_disk(&w, &v);
                    }
                    if let Ok(v) = fetch_json(&format!("{BASE}/network/wifi")) {
                        apply_wifi(&w, &v);
                    }
                }
                if it.is_multiple_of(15) {
                    apply_services(&w);
                    if let Ok(v) = fetch_json(&format!("{BASE}/alerts")) {
                        apply_alerts(&w, &v);
                    }
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
        .expect("sys-poll 线程启动失败");
}

fn push_hist(v: &mut Vec<f32>, x: f32, cap: usize) {
    v.push(x);
    while v.len() > cap {
        v.remove(0);
    }
}

fn tracing_or_log(msg: &str) {
    eprintln!("[panel] {msg}");
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|| "device".into())
        })
}

fn battery_status_cn(status: &str) -> (String, bool) {
    match status {
        "Charging" => ("充电中".into(), true),
        "Full" => ("已充满".into(), false),
        "Discharging" => ("放电中".into(), false),
        "Not charging" => ("未充电".into(), false),
        other => (other.to_string(), false),
    }
}

fn apply_overview(
    w: &Weak<App>,
    ov: &Value,
    ip: &str,
    rx: f64,
    tx: f64,
    cpu_hist: &[f32],
    mem_hist: &[f32],
) {
    let ov = ov.clone();
    let cpu_usage = ov.pointer("/cpu/overall_usage").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let cores: Vec<crate::CoreRow> = ov
        .pointer("/cpu/cores")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|c| crate::CoreRow {
                    id: jf(c, "id") as i32,
                    usage: jf(c, "usage") as f32,
                    freq: format!("{}", jf(c, "frequency_mhz") as i64).into(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut thermal: Vec<(String, f64)> = ov
        .get("thermal")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|t| {
                    (
                        js(t, "name").replace("-thermal", ""),
                        jf(t, "temp_celsius"),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    thermal.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let thermal_rows: Vec<crate::ThermalRow> = thermal
        .into_iter()
        .take(6)
        .map(|(n, t)| crate::ThermalRow {
            name: n.into(),
            temp: t as f32,
            temp_str: format!("{t:.1}").into(),
        })
        .collect();

    let mem_used = ji(ov.pointer("/memory").unwrap_or(&json!({})), "used_mb");
    let mem = ov.get("memory").cloned().unwrap_or(json!({}));
    let mem_total = ji(&mem, "total_mb");
    let mem_avail = ji(&mem, "available_mb");
    let swap_u = ji(&mem, "swap_used_mb");
    let swap_t = ji(&mem, "swap_total_mb");

    let bat = ov.get("battery").cloned().unwrap_or(json!({}));
    let bat_pct = if ji(&bat, "display_capacity_pct") > 0 {
        ji(&bat, "display_capacity_pct")
    } else {
        ji(&bat, "capacity")
    };
    let (charger_online, hw_protocol, hw_brightness) = {
        let g = HW_LATEST.lock().unwrap_or_else(|p| p.into_inner());
        (g.charger_online, g.protocol.clone(), g.brightness.clone())
    };
    let (status_cn, charging) = battery_status_cn(&js(&bat, "status"));
    let protocol = if hw_protocol.is_empty() { js(&bat, "protocol_label") } else { hw_protocol };
    let pill = if protocol.is_empty() {
        status_cn.to_string()
    } else {
        format!("{status_cn} · {protocol}")
    };
    let remain = match ji(&bat, "time_left_min") {
        0 => "--".to_string(),
        n if n > 0 => format!("{}h{}m", n / 60, n % 60),
        n => format!("充{}h{}m", (-n) / 60, (-n) % 60),
    };
    let health = jf(&bat, "health_percent");
    let health_s = if health > 0.0 {
        format!("健康 {:.0}%（实际 {:.1}Ah） · ", health, jf(&bat, "capacity_mah") / 1000.0)
    } else {
        String::new()
    };
    let limit_base = format!(
        "{}上限 {}% · 背光 {}%{}",
        health_s,
        ji(&bat, "effective_max_pct"),
        hw_brightness,
        if jb(&bat, "is_degraded") {
            " · 容量已下降"
        } else if jb(&bat, "at_charge_limit") {
            " · 已到充电上限"
        } else {
            ""
        }
    );
    let bat_limit = if charger_online {
        let g = HW_LATEST.lock().unwrap_or_else(|p| p.into_inner());
        format!("限流 {} · {}", g.chg_vi, limit_base)
    } else {
        limit_base
    };

    let mihomo = ov.get("mihomo").cloned().unwrap_or(json!({}));
    let uptime = jf(&ov, "uptime") as i64;
    let load = ov.get("load_avg").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .map(|v| format!("{:.2}", v.as_f64().unwrap_or(0.0)))
            .collect::<Vec<_>>()
            .join(" / ")
    }).unwrap_or_else(|| "-- / -- / --".into());

    let cpu_hist_rows: Vec<f32> = pad_hist(cpu_hist, HIST_BARS);
    let mem_hist_rows: Vec<f32> = pad_hist(mem_hist, HIST_BARS);

    let w2 = w.clone();
    let ip = ip.to_string();
    w2.upgrade_in_event_loop(move |ui| {
        let sys = ui.global::<Sys>();
        sys.set_online(true);
        sys.set_hostname(hostname().into());
        sys.set_ip(ip.into());
        sys.set_uptime(fmt_uptime(uptime).into());
        sys.set_load(load.into());
        sys.set_procs_count(format!("{}", ji(&ov, "process_count")).into());
        sys.set_cpu_usage(cpu_usage as f32);
        sys.set_cpu_str(format!("{cpu_usage:.1}").into());
        sys.set_cores(model(cores));
        sys.set_cpu_history(model(cpu_hist_rows));
        sys.set_governor({
            // governor 不在 overview 里，由 hardware 轮询补充；此处保留旧值
            let g = sys.get_governor().to_string();
            if g.is_empty() { "--".into() } else { g.into() }
        });

        let thermal_model = thermal_rows;
        sys.set_thermal(model(thermal_model));

        let mem_pct = jf(&mem, "usage_percent");
        sys.set_mem_pct(mem_pct as f32);
        sys.set_mem_str(format!("{mem_pct:.1}").into());
        sys.set_mem_used(fmt_mb(mem_used).into());
        sys.set_mem_total(fmt_mb(mem_total).into());
        sys.set_mem_avail(fmt_mb(mem_avail).into());
        sys.set_mem_swap(format!("{swap_u}/{swap_t} MB").into());
        sys.set_mem_history(model(mem_hist_rows));

        sys.set_bat_pct(bat_pct as f32);
        sys.set_bat_str(format!("{bat_pct}").into());
        sys.set_bat_pill(pill.into());
        sys.set_bat_charging(charging);
        sys.set_bat_protocol(protocol.into());
        sys.set_bat_voltage(format!("{:.2}", jf(&bat, "voltage_v")).into());
        sys.set_bat_current(format!("{:.0}", jf(&bat, "current_ma").abs()).into());
        sys.set_bat_power(format!("{:.1} W", jf(&bat, "power_w")).into());
        let bt = jf(&bat, "temp_celsius");
        sys.set_bat_temp(bt as f32);
        sys.set_bat_temp_str(format!("{bt:.1} °C").into());
        sys.set_bat_remain(remain.into());
        sys.set_bat_limit(bat_limit.into());
        sys.set_bat_ring(ring_arc(bat_pct as f64).into());

        sys.set_rx(fmt_speed(rx).into());
        sys.set_tx(fmt_speed(tx).into());
        let vpn_ok = jb(&mihomo, "available");
        sys.set_vpn_ok(vpn_ok);
        sys.set_vpn_kind(if jb(&mihomo, "tun_enabled") { "TUN" } else { "代理" }.into());
        sys.set_vpn_node(if vpn_ok {
            clean_proxy(&js(&mihomo, "active_proxy")).into()
        } else {
            "未连接".into()
        });
        sys.set_vpn_conn(format!("连接 {}", ji(&mihomo, "connection_count")).into());
        sys.set_vpn_mode(format!("{} 模式", js(&mihomo, "mode")).into());
    })
    .ok();
}

fn pad_hist(v: &[f32], cap: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; cap];
    let n = v.len().min(cap);
    out[cap - n..].copy_from_slice(&v[v.len() - n..]);
    out
}

fn apply_hardware(w: &Weak<App>, v: &Value) {
    let gpu = v.get("gpu").cloned().unwrap_or(json!({}));
    let charging = v.get("charging").cloned().unwrap_or(json!({}));
    let brightness = v.get("brightness").cloned().unwrap_or(json!({}));
    let suspended = jb(&gpu, "suspended");
    let cur = jf(&gpu, "cur_freq_mhz") as i64;
    let min = jf(&gpu, "min_freq_mhz") as i64;
    let max = jf(&gpu, "max_freq_mhz") as i64;
    let gov = js(&gpu, "governor");
    let gpu_pct = if max > 0 { cur as f64 / max as f64 * 100.0 } else { 0.0 };
    let freq_s = if suspended { "休眠".to_string() } else { format!("{cur}") };
    let range = format!("调速区间 {min}–{max}MHz · {}", if gov.is_empty() { "n/a" } else { &gov });
    let charger_online = jb(&charging, "charger_online");
    let chg_v = if jf(&charging, "charger_voltage_uv") > 0.0 {
        format!("{:.2}", jf(&charging, "charger_voltage_uv") / 1_000_000.0)
    } else {
        "--".to_string()
    };
    let i_ua = jf(&charging, "charger_current_ua").abs();
    let chg_i = if i_ua <= 0.0 {
        "--".to_string()
    } else if i_ua >= 1_000_000.0 {
        format!("{:.2} A", i_ua / 1_000_000.0)
    } else {
        format!("{} mA", i_ua / 1000.0)
    };
    let protocol = js(&charging, "protocol_label");
    #[allow(unused_variables)]
    let current_max = jf(&charging, "current_max_ua") as i64;
    let limit_s = if current_max >= 1_000_000 {
        format!("{:.1}A", current_max as f64 / 1_000_000.0)
    } else {
        format!("{current_max}mA")
    };
    let br = jf(&brightness, "percent") as i64;
    // 熄屏检测（msm 把 bl_power=4 当 DPMS）
    let screen_on = std::fs::read_to_string("/sys/class/backlight/ae94000.dsi.0/bl_power")
        .map(|s| s.trim() != "4")
        .unwrap_or(true);
    {
        let mut g = HW_LATEST.lock().unwrap_or_else(|p| p.into_inner());
        *g = HwLatest {
            charger_online,
            chg_vi: format!("{chg_v}V/{chg_i}"),
            brightness: format!("{br}"),
            protocol: protocol.clone(),
        };
    }
    let _ = limit_s;

    w.upgrade_in_event_loop(move |ui| {
        let sys = ui.global::<Sys>();
        sys.set_gpu_suspended(suspended);
        sys.set_gpu_freq(freq_s.into());
        sys.set_gpu_range(range.into());
        sys.set_gpu_pct(gpu_pct as f32);
        sys.set_governor(if gov.is_empty() { "n/a".into() } else { gov.into() });
        sys.set_bat_chg_v(chg_v.into());
        sys.set_bat_chg_i(chg_i.into());
        ui.global::<Ui>().set_screen_on(screen_on);
    })
    .ok();
}

fn apply_process(w: &Weak<App>, v: &Value) {
    let mut rows: Vec<(i64, String, f64, i64)> = v
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| (ji(p, "pid"), js(p, "name"), jf(p, "cpu_usage"), ji(p, "memory_mb")))
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    let count = rows.len();
    let procs: Vec<crate::ProcRow> = rows
        .into_iter()
        .take(5)
        .map(|(pid, name, cpu, mem)| crate::ProcRow {
            pid_str: format!("{pid}").into(),
            name: name.into(),
            cpu: cpu as f32,
            cpu_str: format!("{cpu:.1}").into(),
            mem: fmt_mb(mem).into(),
        })
        .collect();
    w.upgrade_in_event_loop(move |ui| {
        let sys = ui.global::<Sys>();
        sys.set_procs(model(procs));
        sys.set_procs_count(format!("{count}").into());
    })
    .ok();
}

fn apply_disk(w: &Weak<App>, v: &Value) {
    let root = v
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|d| js(d, "mount") == "/")
                .or_else(|| a.first())
        })
        .cloned()
        .unwrap_or(json!({}));
    let pct = jf(&root, "usage_percent");
    let used = ji(&root, "used_mb");
    let total = ji(&root, "total_mb");
    let fs = js(&root, "fstype");
    let inode = jf(&root, "inode_percent");
    w.upgrade_in_event_loop(move |ui| {
        let sys = ui.global::<Sys>();
        sys.set_disk_pct(pct as f32);
        sys.set_disk_str(format!("{pct:.0}").into());
        sys.set_disk_used(fmt_mb(used).into());
        sys.set_disk_total(fmt_mb(total).into());
        sys.set_disk_fs(fs.into());
        sys.set_disk_inode(format!("{inode:.0}%").into());
    })
    .ok();
}

fn apply_wifi(w: &Weak<App>, v: &Value) {
    let connected = jb(v, "connected");
    let ssid = js(v, "ssid");
    let signal = jf(v, "signal_dbm") as i64;
    let band = js(v, "band");
    let ch = jf(v, "channel") as i64;
    let bitrate = js(v, "bitrate");
    let detail = if connected {
        format!("{band} · 信道{ch} · {bitrate}")
    } else {
        String::new()
    };
    w.upgrade_in_event_loop(move |ui| {
        let sys = ui.global::<Sys>();
        sys.set_wifi_ok(connected);
        sys.set_wifi_ssid(if connected { ssid.into() } else { "WiFi 未连接".into() });
        sys.set_wifi_signal(signal as i32);
        sys.set_wifi_signal_str(if connected { format!("{signal} dBm").into() } else { "--".into() });
        sys.set_wifi_detail(detail.into());
    })
    .ok();
}

fn apply_services(w: &Weak<App>) {
    let mut rows = Vec::new();
    for s in ["mihomo", "syncthing", "device-monitor", "xtokenhub"] {
        let state = std::process::Command::new("systemctl")
            .args(["is-active", s])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        let label = match state.as_str() {
            "active" => "运行",
            "inactive" => "停止",
            "failed" => "失败",
            _ => "未知",
        };
        rows.push(crate::SvcRow {
            name: s.into(),
            state: label.into(),
        });
    }
    w.upgrade_in_event_loop(move |ui| {
        ui.global::<Sys>().set_services(model(rows));
    })
    .ok();
}

fn apply_alerts(w: &Weak<App>, v: &Value) {
    let first = v.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
    let (title, msg, time, level) = if first.is_null() {
        (String::new(), String::new(), String::new(), "info".to_string())
    } else {
        let ts = ji(&first, "timestamp");
        let time = chrono::DateTime::from_timestamp(ts, 0)
            .map(|d| {
                use chrono::TimeZone;
                Local.from_utc_datetime(&d.naive_utc()).format("%H:%M").to_string()
            })
            .unwrap_or_default();
        (
            js(&first, "title"),
            js(&first, "message"),
            time,
            js(&first, "level"),
        )
    };
    w.upgrade_in_event_loop(move |ui| {
        let sys = ui.global::<Sys>();
        sys.set_alert_title(title.into());
        sys.set_alert_msg(msg.into());
        sys.set_alert_time(time.into());
        sys.set_alert_level(level.into());
    })
    .ok();
}

// ────────────────────────── Token 页轮询 ──────────────────────────

fn spawn_token_poller(w: Weak<App>) {
    std::thread::Builder::new()
        .name("xtb-poll".into())
        .spawn(move || loop {
            let midnight = Local::now()
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .and_then(|d| Local.from_local_datetime(&d).single())
                .map(|d| d.timestamp())
                .unwrap_or(0);
            let t0 = Instant::now();
            let today = fetch_json(&format!("{XTB}/stats/summary?since={midnight}")).ok();
            let d7 = fetch_json(&format!("{XTB}/stats/summary?hours=168")).ok();
            let trend = fetch_json(&format!("{XTB}/stats/trend?hours=168&bucket=day")).ok();
            let by_model = fetch_json(&format!("{XTB}/stats/by-model?hours=168")).ok();
            let by_channel = fetch_json(&format!("{XTB}/stats/by-channel?hours=168")).ok();
            let by_key = fetch_json(&format!("{XTB}/stats/by-key?hours=168")).ok();
            let lifetime = fetch_json(&format!("{XTB}/stats/lifetime")).ok();
            let forecast = fetch_json(&format!("{XTB}/stats/cost/forecast")).ok();
            let billing = fetch_json(&format!("{XTB}/settings/billing")).ok();
            let balances = fetch_json(&format!("{XTB}/channels/balances")).ok();
            let channels = fetch_json(&format!("{XTB}/channels")).ok();
            let rest_ok = today.is_some();
            let http_ms = t0.elapsed().as_millis();

            apply_token(
                &w,
                today.as_ref(),
                d7.as_ref(),
                trend.as_ref(),
                by_model.as_ref(),
                by_channel.as_ref(),
                by_key.as_ref(),
                lifetime.as_ref(),
                forecast.as_ref(),
                billing.as_ref(),
                balances.as_ref(),
                channels.as_ref(),
                rest_ok,
                http_ms,
            );

            // 15s 轮询；stats.updated 事件把间隔压到 5s
            let mut waited = 0u64;
            while waited < 15 {
                std::thread::sleep(Duration::from_millis(500));
                waited += 1; // 最快 5s 一刷：sleep 10 拍后允许
                if waited >= 10 && REFRESH_REQUESTED.swap(false, Ordering::Relaxed) {
                    break;
                }
            }
        })
        .expect("xtb-poll 线程启动失败");
}

#[allow(clippy::too_many_arguments)]
fn apply_token(
    w: &Weak<App>,
    today: Option<&Value>,
    d7: Option<&Value>,
    trend: Option<&Value>,
    by_model: Option<&Value>,
    by_channel: Option<&Value>,
    by_key: Option<&Value>,
    lifetime: Option<&Value>,
    forecast: Option<&Value>,
    billing: Option<&Value>,
    balances: Option<&Value>,
    _channels: Option<&Value>,
    rest_ok: bool,
    http_ms: u128,
) {
    let empty = json!({});
    let today = today.unwrap_or(&empty);
    let d7 = d7.unwrap_or(&empty);
    let rate = billing
        .map(|b| jf(b, "usd_cny_rate"))
        .filter(|r| *r > 0.0)
        .unwrap_or(7.2);
    let currency = billing
        .map(|b| js(b, "display_currency"))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "CNY".into());
    let is_cny = currency.eq_ignore_ascii_case("CNY");
    let money = |usd: f64| {
        if is_cny {
            format!("¥{:.2}", usd * rate)
        } else {
            format!("${usd:.2}")
        }
    };

    let total_req = ji(today, "total_requests");
    let success_rate = if total_req > 0 {
        ji(today, "success_requests") as f64 / total_req as f64 * 100.0
    } else {
        100.0
    };

    let trend_pts: Vec<f32> = trend
        .and_then(|t| t.as_array())
        .map(|a| a.iter().map(|p| ji(p, "requests") as f32).collect())
        .unwrap_or_default();
    let (trend_line, trend_area) = polyline(&trend_pts, TREND_W, TREND_H, "");

    let total_all: i64 = by_model
        .and_then(|m| m.as_array())
        .map(|a| a.iter().map(|r| ji(r, "requests")).sum())
        .unwrap_or(0);
    let models: Vec<crate::RankRow> = by_model
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .take(5)
                .map(|r| {
                    let share = if total_all > 0 {
                        ji(r, "requests") as f64 / total_all as f64 * 100.0
                    } else {
                        0.0
                    };
                    crate::RankRow {
                        name: js(r, "name").into(),
                        reqs: format!("{}", ji(r, "requests")).into(),
                        tokens: fmt_tokens(ji(r, "total_tokens")).into(),
                        cost: money(jf(r, "cost_usd")).into(),
                        share: share as f32,
                        share_str: format!("{share:.0}%").into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let max_reqs = by_channel
        .and_then(|m| m.as_array())
        .map(|a| a.iter().map(|r| ji(r, "requests")).max().unwrap_or(1).max(1))
        .unwrap_or(1);
    let chans: Vec<crate::ChanRow> = by_channel
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .take(4)
                .map(|r| {
                    let name = js(r, "name");
                    let bal = balances
                        .and_then(|b| b.get("items").and_then(|i| i.as_array()))
                        .and_then(|items| {
                            items
                                .iter()
                                .find(|x| js(x, "channel_name") == name)
                                .and_then(|x| x.get("balance"))
                                .cloned()
                        });
                    let bal_s = match bal {
                        Some(b) if jf(&b, "total") > 0.0 => {
                            let cur = js(&b, "currency");
                            format!("余额 {}{:.2}", if cur == "CNY" { "¥" } else { "$" }, jf(&b, "total"))
                        }
                        _ => "余额 —".to_string(),
                    };
                    let avg_s = jf(r, "avg_ms") / 1000.0;
                    crate::ChanRow {
                        name: name.into(),
                        reqs: format!("{}", ji(r, "requests")).into(),
                        avg: format!("{avg_s:.1}s").into(),
                        cost: money(jf(r, "cost_usd")).into(),
                        balance: bal_s.into(),
                        share: (ji(r, "requests") as f64 / max_reqs as f64 * 100.0) as f32,
                        health: avg_s as f32,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let key_total: i64 = by_key
        .and_then(|m| m.as_array())
        .map(|a| a.iter().map(|r| ji(r, "requests")).sum())
        .unwrap_or(0);
    let keys: Vec<crate::KeyRow> = by_key
        .and_then(|m| m.as_array())
        .map(|a| {
            a.iter()
                .take(4)
                .map(|r| {
                    let share = if key_total > 0 {
                        ji(r, "requests") as f64 / key_total as f64 * 100.0
                    } else {
                        0.0
                    };
                    crate::KeyRow {
                        name: js(r, "name").into(),
                        reqs: format!("{} 次", ji(r, "requests")).into(),
                        share: format!("{share:.0}%").into(),
                        cost: money(jf(r, "cost_usd")).into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let lt = lifetime.unwrap_or(&empty);
    let fc = forecast.unwrap_or(&empty);
    let proj = match fc.get("projected_usd").and_then(|v| v.as_f64()) {
        Some(v) => format!("预测今日 {}", money(v)),
        None => "预测今日 样本不足".to_string(),
    };
    let err_total = ji(d7, "total_requests");
    let err_cnt = ji(d7, "error_requests");
    let err_pct = if err_total > 0 { err_cnt as f64 / err_total as f64 * 100.0 } else { 0.0 };
    let avg_ms = jf(d7, "avg_duration_ms");
    let hit7 = jf(d7, "cache_hit_rate") * 100.0;
    let budget = if jf(billing.unwrap_or(&empty), "monthly_budget_usd") > 0.0 {
        format!("月预算 {}", money(jf(billing.unwrap_or(&empty), "monthly_budget_usd")))
    } else {
        "未设月预算".to_string()
    };

    let t_tokens_s = fmt_tokens(ji(today, "total_tokens"));
    let t_cached_s = fmt_tokens(ji(today, "cached_tokens"));
    let t_cost_s = money(jf(today, "cost_usd"));
    let t_hit_s = format!("{:.1}%", jf(today, "cache_hit_rate") * 100.0);
    let success_s = format!("{success_rate:.1}");
    let daily_avg_s = format!("日均 {}", money(jf(fc, "daily_avg_usd")));
    let avg_lat_s = format!("{:.1}s", avg_ms / 1000.0);
    let cache7_s = format!("{hit7:.0}%");
    let trend_info = format!(
        "{} 次请求 / {} 次错误 · 输入 {} · 输出 {}",
        err_total,
        err_cnt,
        fmt_tokens(ji(d7, "prompt_tokens")),
        fmt_tokens(ji(d7, "completion_tokens")),
    );
    let lt_tokens_s = fmt_tokens(ji(lt, "total_tokens"));
    let peak_day = js(lt, "peak_day");
    let lt_peak_s = format!("{} ({})", fmt_tokens(ji(lt, "peak_day_tokens")),
        if peak_day.len() >= 10 { &peak_day[5..10] } else { &peak_day });
    let lt_streak_s = format!("{} / {} 天", ji(lt, "current_streak"), ji(lt, "max_streak"));
    let rest_info = format!("{http_ms}ms");

    w.upgrade_in_event_loop(move |ui| {
        let tok = ui.global::<Tok>();
        tok.set_rest_ok(rest_ok);
        tok.set_today_reqs(format!("{total_req}").into());
        tok.set_success_rate(success_rate as f32);
        tok.set_success_str(success_s.into());
        tok.set_t_tokens(t_tokens_s.into());
        tok.set_t_cached(t_cached_s.into());
        tok.set_t_cost(t_cost_s.into());
        tok.set_t_hit(t_hit_s.into());
        tok.set_forecast(proj.into());
        tok.set_daily_avg(daily_avg_s.into());
        tok.set_models(model(models));
        tok.set_chans(model(chans));
        tok.set_keys(model(keys));
        tok.set_err_pct(err_pct as f32);
        tok.set_err_str(format!("{err_pct:.1}").into());
        tok.set_avg_lat(avg_lat_s.into());
        tok.set_avg_lat_num((avg_ms / 1000.0) as f32);
        tok.set_cache7(cache7_s.into());
        tok.set_cache7_num(hit7 as f32);
        tok.set_trend_line(trend_line.into());
        tok.set_trend_area(trend_area.into());
        tok.set_trend_info(trend_info.into());
        tok.set_lt_tokens(lt_tokens_s.into());
        tok.set_lt_peak(lt_peak_s.into());
        tok.set_lt_streak(lt_streak_s.into());
        tok.set_budget(budget.into());
        tok.set_rest_info(rest_info.into());
        if !rest_ok {
            tok.set_gw_state("REST 异常".into());
        }
    })
    .ok();
}

/// WS 长连接：2Hz 吞吐滑窗（UI 由 1s tick 限频落刷）+ 最近一次调用 + stats.updated 刷新信号。
fn spawn_token_ws(w: Weak<App>) {
    std::thread::Builder::new()
        .name("xtb-ws".into())
        .spawn(move || {
            loop {
                match tungstenite::connect(XTB_WS) {
                    Err(e) => {
                        set_ws_ok(&w, false, Some(format!("连接失败: {e}")));
                        std::thread::sleep(Duration::from_secs(3));
                    }
                    Ok((mut socket, _)) => {
                        if let tungstenite::stream::MaybeTlsStream::Plain(s) = socket.get_ref() {
                            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                        }
                        set_ws_ok(&w, true, None);
                        loop {
                            match socket.read() {
                                Ok(tungstenite::Message::Text(t)) => {
                                    handle_ws_msg(&w, &t);
                                }
                                Ok(tungstenite::Message::Binary(b)) => {
                                    if let Ok(s) = std::str::from_utf8(&b) {
                                        handle_ws_msg(&w, s);
                                    }
                                }
                                Ok(tungstenite::Message::Close(_)) => {
                                    set_ws_ok(&w, false, Some("被服务端关闭".into()));
                                    break;
                                }
                                Ok(_) => {}
                                Err(tungstenite::Error::Io(e))
                                    if e.kind() == std::io::ErrorKind::WouldBlock
                                        || e.kind() == std::io::ErrorKind::TimedOut =>
                                {
                                    // 读超时：顺带消费刷新请求
                                    if REFRESH_REQUESTED.swap(false, Ordering::Relaxed) {
                                        // 留给 REST 线程处理（它有自己的节流）
                                    }
                                }
                                Err(e) => {
                                    set_ws_ok(&w, false, Some(format!("读取失败: {e}")));
                                    break;
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_secs(3));
                    }
                }
            }
        })
        .expect("xtb-ws 线程启动失败");
}

fn set_ws_ok(w: &Weak<App>, ok: bool, err: Option<String>) {
    w.upgrade_in_event_loop(move |ui| {
        let tok = ui.global::<Tok>();
        tok.set_ws_ok(ok);
        if ok {
            tok.set_gw_state("正常".into());
        } else if tok.get_rest_ok() {
            tok.set_gw_state("WS 断开".into());
        }
        if let Some(e) = err {
            tok.set_last_error(e.into());
        } else {
            tok.set_last_error("".into());
        }
    })
    .ok();
}

fn handle_ws_msg(w: &Weak<App>, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return };
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "stats.throughput" => {
            let p = v.get("payload").cloned().unwrap_or(json!({}));
            let val = jf(&p, "tokens_per_sec") as f32;
            let streams = ji(&p, "active_streams") as i32;
            {
                let mut wnd = TPS_WIN.lock().unwrap_or_else(|p| p.into_inner());
                wnd.push_back(val);
                while wnd.len() > TPS_POINTS {
                    wnd.pop_front();
                }
            }
            TPS_STREAMS.store(streams, Ordering::Relaxed);
            TPS_SEQ.fetch_add(1, Ordering::Relaxed);
            // 不在这里刷 UI：交给 tick_clock 每秒统一落（否则 2Hz 消息持续喂渲染循环）
        }
        "request.completed" => {
            let p = v.get("payload").cloned().unwrap_or(json!({}));
            let cost = jf(&p, "cost_usd");
            // 币种/汇率不在 WS 事件里，用默认汇率展示（REST 轮询会带准确币种）
            let money = format!("¥{:.4}", cost * 7.2);
            let last = format!(
                "{} · {} · {} · {} tokens · {} · {}s",
                js(&p, "model"),
                js(&p, "channel_name"),
                js(&p, "key_name"),
                fmt_tokens(ji(&p, "total_tokens")),
                money,
                jf(&p, "duration_ms") / 1000.0,
            );
            w.upgrade_in_event_loop(move |ui| {
                ui.global::<Tok>().set_last_req(last.into());
            })
            .ok();
        }
        "stats.updated" => {
            REFRESH_REQUESTED.store(true, Ordering::Relaxed);
        }
        _ => {}
    }
}

// ────────────────────────── demo 模式（macOS 预览） ──────────────────────────

fn spawn_demo(w: Weak<App>) {
    // 系统侧：每 2s 生成一份合成 overview
    {
        let w = w.clone();
        std::thread::spawn(move || {
            let mut t = 0f64;
            loop {
                let cpu = 28.0 + 22.0 * (t * 0.9).sin() + 6.0 * (t * 3.1).sin();
                let cores: Vec<Value> = (0..8)
                    .map(|i| {
                        json!({
                            "id": i,
                            "usage": (cpu * (0.6 + 0.1 * i as f64)).clamp(1.0, 99.0),
                            "frequency_mhz": (1400.0 + 1200.0 * (t * 0.4 + i as f64).sin()) as i64,
                        })
                    })
                    .collect();
                let thermal: Vec<Value> = ["cpu0", "cpu4", "gpu-top", "qcom-battery", "cpu7", "gpu-bottom"]
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        json!({
                            "name": format!("{n}-thermal"),
                            "temp_celsius": 38.0 + 12.0 * (t * 0.5 + i as f64 * 0.7).sin(),
                        })
                    })
                    .collect();
                let net_rx = 300_000.0 * (1.0 + (t * 1.3).sin()).max(0.1);
                let ov = json!({
                    "cpu": {"overall_usage": cpu, "cores": cores},
                    "memory": {"usage_percent": 61.0 + 5.0 * (t * 0.3).sin(), "used_mb": 3780, "total_mb": 5888,
                               "available_mb": 1980, "swap_used_mb": 120, "swap_total_mb": 2048},
                    "thermal": thermal,
                    "battery": {"capacity": 78, "display_capacity_pct": 78, "status": "Discharging",
                                "voltage_v": 3.92, "current_ma": -812.0, "power_w": 3.18,
                                "temp_celsius": 31.5, "time_left_min": 386,
                                "effective_max_pct": 99, "health_percent": 65.5,
                                "capacity_mah": 1780.0, "is_degraded": true, "at_charge_limit": false},
                    "network": [{"name": "wlan0", "is_up": true, "ip_addresses": ["192.168.31.23"],
                                 "rx_bytes": (t * 900_000.0) as u64, "tx_bytes": (t * 120_000.0) as u64}],
                    "mihomo": {"available": true, "tun_enabled": true, "mode": "rule",
                               "active_proxy": "🇸🇬 新加坡 01 · iec.cloudflare.com", "connection_count": 18,
                               "download_total": 1024.0*1024.0*1024.0*31.0, "upload_total": 1024.0*1024.0*512.0},
                    "process_count": 239,
                    "uptime": 86_400 * 3 + 52_000,
                    "load_avg": [cpu / 40.0, cpu / 55.0, cpu / 70.0],
                    "timestamp": chrono::Utc::now().timestamp(),
                });
                let rx = net_rx * (0.9 + 0.2 * (t * 2.2).sin());
                let tx = 40_000.0 + 30_000.0 * (t * 1.7).sin();
                let hist = demo_hist(t, 1.0);
                let mem_hist = demo_hist(t * 0.4, 0.12);
                apply_overview(&w, &ov, "192.168.31.23", rx, tx, &hist, &mem_hist);

                let hw = json!({
                    "gpu": {"cur_freq_mhz": (305.0 + 500.0 * (t * 0.8).sin().abs()) as i64,
                            "min_freq_mhz": 305, "max_freq_mhz": 845, "governor": "schedutil", "suspended": false},
                    "charging": {"charger_online": false, "protocol_label": "", "charger_voltage_uv": 0,
                                 "charger_current_ua": 0, "current_max_ua": 1_500_000},
                    "brightness": {"percent": 42},
                });
                apply_hardware(&w, &hw);
                apply_disk(&w, &json!([{"mount": "/", "usage_percent": 62.4, "used_mb": 45_600,
                                        "total_mb": 96_000, "fstype": "ext4", "inode_percent": 4.1}]));
                apply_wifi(&w, &json!({"connected": true, "ssid": "Home-5G", "signal_dbm": -52,
                                       "band": "5GHz", "channel": 44, "bitrate": "866.7 Mbit/s"}));
                apply_process(&w, &json!([
                    {"pid": 812, "name": "xtokenhub", "cpu_usage": 23.4, "memory_mb": 412},
                    {"pid": 331, "name": "mihomo", "cpu_usage": 8.1, "memory_mb": 188},
                    {"pid": 1, "name": "init", "cpu_usage": 0.1, "memory_mb": 12},
                    {"pid": 902, "name": "device-monitor-server", "cpu_usage": 3.2, "memory_mb": 96},
                    {"pid": 77, "name": "slint-panel", "cpu_usage": 6.5, "memory_mb": 132},
                ]));
                apply_services(&w);
                apply_alerts(&w, &json!([
                    {"level": "warning", "title": "内存偏高", "message": "使用率 71% 超过阈值 70%", "timestamp": chrono::Utc::now().timestamp() - 600},
                ]));
                t += 2.0;
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }
    // Token 侧：合成 REST 数据 + 模拟 2Hz 吞吐
    {
        let w = w.clone();
        std::thread::spawn(move || {
            loop {
                let models = json!([
                    {"name": "glm-4.7-plus", "requests": 1284, "total_tokens": 3_842_100, "cost_usd": 4.83},
                    {"name": "claude-sonnet-4-6", "requests": 642, "total_tokens": 2_105_400, "cost_usd": 12.94},
                    {"name": "deepseek-v4", "requests": 388, "total_tokens": 988_200, "cost_usd": 0.41},
                    {"name": "qwen3.5-max", "requests": 201, "total_tokens": 512_800, "cost_usd": 0.62},
                    {"name": "gemini-3-flash", "requests": 96, "total_tokens": 180_400, "cost_usd": 0.12},
                ]);
                let chans = json!([
                    {"name": "bigmodel 官方", "requests": 1284, "avg_ms": 1820.0, "cost_usd": 4.83},
                    {"name": "cc.ovicm.us", "requests": 642, "avg_ms": 4310.0, "cost_usd": 12.94},
                    {"name": "deepseek 官方", "requests": 388, "avg_ms": 940.0, "cost_usd": 0.41},
                    {"name": "aliyun dashscope", "requests": 201, "avg_ms": 11_200.0, "cost_usd": 0.62},
                ]);
                let keys = json!([
                    {"name": "macbook-claude-code", "requests": 1102, "cost_usd": 11.2},
                    {"name": "home-assistant", "requests": 903, "cost_usd": 3.1},
                    {"name": "openclaw-bot", "requests": 406, "cost_usd": 2.4},
                    {"name": "ci-pipeline", "requests": 200, "cost_usd": 0.6},
                ]);
                let trend = Value::Array(
                    (0..7)
                        .map(|i| json!({"requests": (600.0 + 380.0 * (i as f64 * 1.1).sin().abs()) as i64}))
                        .collect(),
                );
                apply_token(
                    &w,
                    Some(&json!({"total_requests": 183, "success_requests": 181, "total_tokens": 812_400,
                                 "cached_tokens": 402_100, "cost_usd": 2.16, "cache_hit_rate": 0.495})),
                    Some(&json!({"total_requests": 2611, "error_requests": 47, "avg_duration_ms": 3180.0,
                                 "cache_hit_rate": 0.61, "prompt_tokens": 4_120_000, "completion_tokens": 1_820_000})),
                    Some(&trend),
                    Some(&models),
                    Some(&chans),
                    Some(&keys),
                    Some(&json!({"total_tokens": 38_420_000, "peak_day_tokens": 4_210_000,
                                 "peak_day": "2026-09-23", "current_streak": 41, "max_streak": 58})),
                    Some(&json!({"projected_usd": 2.84, "daily_avg_usd": 2.31})),
                    Some(&json!({"display_currency": "CNY", "usd_cny_rate": 7.19, "monthly_budget_usd": 200.0})),
                    Some(&json!({"items": [
                        {"channel_name": "bigmodel 官方", "balance": {"currency": "CNY", "total": 186.4}},
                        {"channel_name": "cc.ovicm.us", "balance": {"currency": "USD", "total": 23.7}},
                    ]})),
                    Some(&json!({"items": [
                        {"id": 1, "name": "bigmodel 官方", "provider": "", "status": 1},
                        {"id": 2, "name": "cc.ovicm.us", "provider": "", "status": 1},
                    ]})),
                    true,
                    42,
                );
                set_ws_ok(&w, true, None);
                std::thread::sleep(Duration::from_secs(15));
            }
        });
    }
    {
        let w = w.clone();
        std::thread::spawn(move || {
            let mut tps: VecDeque<f32> = VecDeque::new();
            let mut t = 0f64;
            loop {
                let burst = (t % 40.0 < 12.0) as i32;
                let v = if burst == 1 {
                    (30.0 + 25.0 * (t * 2.7).sin()).max(2.0) as f32
                } else {
                    0.0
                };
                tps.push_back(v);
                while tps.len() > TPS_POINTS {
                    tps.pop_front();
                }
                let vals: Vec<f32> = tps.iter().copied().collect();
                let (line, area) = polyline(&vals, TPS_W, TPS_H, "");
                let cur = v;
                let streams = if burst == 1 { 2 } else { 0 };
                let last = "glm-4.7-plus · bigmodel 官方 · macbook-claude-code · 1.2万 tokens · ¥2.8140 · 6.4s";
                w.upgrade_in_event_loop(move |ui| {
                    let tok = ui.global::<Tok>();
                    tok.set_tps(cur);
                    tok.set_tps_str(format!("{cur:.0}").into());
                    tok.set_streams(streams);
                    tok.set_tps_line(line.into());
                    tok.set_tps_area(area.into());
                    tok.set_last_req(last.into());
                })
                .ok();
                t += 0.5;
                std::thread::sleep(Duration::from_millis(500));
            }
        });
    }
    // demo 天气
    {
        let w = w.clone();
        std::thread::spawn(move || {
            loop {
                w.upgrade_in_event_loop(|ui| {
                    let clk = ui.global::<Clk>();
                    clk.set_w_ok(true);
                    clk.set_w_temp_c(21.9);
                    clk.set_w_temp("22".into());
                    clk.set_w_desc("多云".into());
                    clk.set_w_wet(false);
                    clk.set_w_feels("体感 20°".into());
                    clk.set_w_wind("东南 3.2 m/s".into());
                    clk.set_w_range("今日 17~27°C · 降水概率 10%".into());
                    clk.set_w_extra("湿度 52% · 降水 0.0mm · 日出 06:28 · 日落 18:52".into());
                    clk.set_w_note("西安 · Open-Meteo".into());
                })
                .ok();
                std::thread::sleep(Duration::from_secs(900));
            }
        });
    }
}

fn demo_hist(t: f64, amp: f64) -> Vec<f32> {
    (0..HIST_BARS)
        .map(|i| {
            let x = t + i as f64 * 0.35;
            (50.0 + 45.0 * (x * 0.9).sin() * amp).clamp(2.0, 100.0) as f32
        })
        .collect()
}
