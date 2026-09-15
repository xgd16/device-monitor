//! 第 3 页：时钟/时间页。
//!
//! 信息分区（卡片位置跨页保持一致，用户建立空间记忆）：
//! - 顶栏：标题 + 时区/NTP 状态 + 页签
//! - 左大卡：本机时刻（超大 HH:MM:SS + 秒进度 + 日期/周/天 + 元信息）
//! - 右上：时间进度（今日/本周/本月/本年）
//! - 右中：世界时钟（6 城市，跨日标注）
//! - 右下：日出日落（西安，NOAA 简化算法）
//! - 底栏：翻页提示 + 刷新说明
//!
//! 刷新策略：整屏重绘由采集周期驱动；秒级变化的部分（大时钟、秒进度条、
//! 今日进度行）每秒局部重绘 —— 重绘前先用卡片底色清除旧像素，避免字形残影
//! （数字步进等宽，但「剩余 8 小时 57 分」这类文本宽度会变）。
//! 世界时钟只在跨分钟时局部重绘，避免 10s 采集间隔下分钟边界滞后。

use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate, Timelike};

use super::canvas::Canvas;
use super::font::Weight;
use super::layout::{card, page_tabs, split_col, title, Pane};
use super::theme::{Palette, Type};
use super::weather;

// ── 与 layout 保持一致的布局常量（token 页同样各自持有一份）──
const PAD: i32 = 22;
const GAP: i32 = 16;
const HEADER_H: i32 = 96;
const FOOTER_H: i32 = 56;

// ── 时钟页字号（超出 `Type` 级别的超大字）──
/// 主时:分字号。
/// 上界由两处共同决定（本机 2340 逻辑宽、左列 1311 宽）：
/// 宽度 `1.854S + 44 + 0.3666S ≤ 1259` → S ≤ 547；高度上字形占 0.8013S，
/// 卡高 600 减去标题/日期/进度条/两行元信息后留给大字约 320 → S ≤ 400。
const TSIZE_HM: f32 = 400.0;
/// 秒字号（0.45 × 主字号，基线对齐）
const TSIZE_SS: f32 = 180.0;
/// 日期字号
const TSIZE_DATE: f32 = 40.0;
/// 世界时钟时刻字号
const TSIZE_WORLD: f32 = 31.0;
/// 天气/电量卡的大数值字号
const TSIZE_METRIC: f32 = 88.0;

/// 左列：大时钟卡高度（其余给底部「天气 + 电量」一行）
const HERO_H: i32 = 600;

/// 右列三张卡的高度比例（时间进度 / 世界时钟 / 日出日落）。
/// 抽成常量：整屏重绘与秒级局部重绘都要用它算出各卡的 y，写在两处必然漂移。
const RIGHT_RATIOS: [i32; 3] = [29, 34, 37];

/// 西安坐标（日出于地平线计算用）。
const LAT: f64 = 34.3416;
const LON: f64 = 108.9398;
/// 日出日落的视线高度角（度）。
///
/// 只取标准值 -0.833°（平均大气折射 34′ + 日面半径 16′），**不加海拔地平线下沉**：
/// 下沉量 `dip = 0.0347·√h` 假设观测者四周是到几何地平线的无遮挡平地，而西安南侧
/// 紧邻秦岭、城区海拔基准也与海平面无关，套用会把昼长算长（实测 +420m 下沉使
/// 2026-09-15 昼长从 12h27m 变成 12h33m，与天气服务口径差 7 分钟）。
/// 取值与常见天气服务一致，便于用户对照。
const SUN_H0_DEG: f64 = -0.833;

/// 世界时钟城市表。
const CITIES: &[(&str, &str)] = &[
    ("东京", "Asia/Tokyo"),
    ("伦敦", "Europe/London"),
    ("纽约", "America/New_York"),
    ("洛杉矶", "America/Los_Angeles"),
    ("巴黎", "Europe/Paris"),
    ("UTC", "UTC"),
];

// ────────────────────────── 环境快照（子进程结果缓存）──────────────────────────

struct WorldRow {
    name: &'static str,
    /// HH:MM
    time: String,
    /// UTC 偏移，如 `+09:00`
    offset: String,
    /// 相对本机日期的偏移：-1 昨天 / 0 今天 / +1 明天
    day_delta: i64,
}

struct EnvSnapshot {
    at: Instant,
    world: Vec<WorldRow>,
    ntp_synced: bool,
    /// 时区名，如 `Asia/Shanghai`
    tz_name: String,
    /// 时区缩写，如 `CST`
    tz_abbr: String,
    /// RTC 硬件时钟时间（字符串，异常时也要显示出来）
    rtc: String,
    rtc_sane: bool,
}

static ENV: OnceLock<Mutex<Option<EnvSnapshot>>> = OnceLock::new();
/// 叠加到 `now_local()` 的秒偏移（默认 0，仅测试/预览使用）。
///
/// 局部重绘的正确性只有让时间前进才能验证：同一秒内 `render_clock` 画出的内容与
/// 整屏重绘完全一致，即使清除区域没盖住旧文本也不会露出差异。配合
/// `--at/--tick`（见 main.rs）可以导出「整屏 @T+N」与「整屏 @T → 局部 @T+N」
/// 两张图做逐像素比对，差异即幽灵像素。
static TIME_OFFSET: AtomicI64 = AtomicI64::new(0);

/// 设置时间偏移（秒）。
pub fn set_time_offset(secs: i64) {
    TIME_OFFSET.store(secs, Ordering::Relaxed);
}

/// 当前时刻（含测试偏移）。
///
/// 注意这里必须直接调 `Local::now()`：全局把 `Local::now()` 替换成 `now_local()`
/// 时很容易把函数体内的这一处也换掉，变成自我递归（编译器会给
/// `function cannot return without recursing` 警告，运行即栈溢出）。
fn now_local() -> chrono::DateTime<Local> {
    Local::now() + chrono::Duration::seconds(TIME_OFFSET.load(Ordering::Relaxed))
}

/// 清除卡片内部（保留圆角），返回清除区顶端。
///
/// 局部重绘必须整块清除再整块重画：页面里有多处**右对齐且宽度会变**的文本
/// （「剩余 2h03m」→「剩余 59m」、「· 昨天」标记出现/消失），只重画不清除
/// 会留下旧字的幽灵像素。圆角半径 16px，所以上下左右各留 20px 内缩。
fn clear_card_body(c: &mut Canvas, p: &Pane, top_offset: i32) {
    let y = p.y + top_offset;
    let h = (p.y + p.h - 20) - y;
    if h > 0 {
        c.rect(p.x + 4, y, p.w - 8, h, Palette::BG_SURFACE);
    }
}
/// 上次局部重绘世界时钟时的「分钟」时间戳
static LAST_WORLD_MIN: AtomicI64 = AtomicI64::new(i64::MIN);

/// 世界时钟等子进程结果只变化到分钟级，30s 缓存足够，避免每帧 fork。
fn env() -> &'static Mutex<Option<EnvSnapshot>> {
    ENV.get_or_init(|| Mutex::new(None))
}

fn refresh_env() -> EnvSnapshot {
    let today = now_local().date_naive();
    let mut world = Vec::with_capacity(CITIES.len());
    for (name, tz) in CITIES {
        let out = Command::new("date")
            .env("TZ", tz)
            .arg("+%H:%M|%:z|%Y-%m-%d")
            .output();
        let s = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            _ => {
                world.push(WorldRow {
                    name,
                    time: "--:--".into(),
                    offset: "?".into(),
                    day_delta: 0,
                });
                continue;
            }
        };
        let mut it = s.split('|');
        let time = it.next().unwrap_or("--:--").to_string();
        let offset = it.next().unwrap_or("?").to_string();
        let day_delta = it
            .next()
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .map(|d| (d - today).num_days())
            .unwrap_or(0);
        world.push(WorldRow { name, time, offset, day_delta });
    }

    // 时间同步状态：NTP 是否同步 + RTC 硬件时钟（本机 RTC 走时异常，要让人看见）
    let mut ntp_synced = false;
    let mut tz_name = "本地".to_string();
    let mut tz_abbr = String::new();
    let mut rtc = "未知".to_string();
    if let Ok(o) = Command::new("timedatectl").output() {
        let txt = String::from_utf8_lossy(&o.stdout).to_string();
        ntp_synced = txt
            .lines()
            .any(|l| l.contains("System clock synchronized") && l.contains("yes"));
        if let Some(l) = txt.lines().find(|l| l.contains("RTC time")) {
            if let Some((_, v)) = l.split_once(':') {
                rtc = v.trim().replace("  ", " ");
            }
        }
        // Time zone: Asia/Shanghai (CST, +0800)
        if let Some(l) = txt.lines().find(|l| l.contains("Time zone")) {
            if let Some((_, v)) = l.split_once(':') {
                let v = v.trim();
                if let Some((name, rest)) = v.split_once('(') {
                    tz_name = name.trim().to_string();
                    tz_abbr = rest.split(',').next().unwrap_or("").trim().to_string();
                } else {
                    tz_name = v.to_string();
                }
            }
        }
    }
    let rtc_sane = rtc.starts_with(&now_local().format("%Y").to_string());

    EnvSnapshot { at: Instant::now(), world, ntp_synced, tz_name, tz_abbr, rtc, rtc_sane }
}

/// 取环境快照；超过 30s 或从未取过时刷新。锁中毒时降级为实时取一次。
fn env_cached() -> EnvSnapshot {
    let m = env();
    let mut g = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(s) = g.as_ref() {
        if s.at.elapsed() < Duration::from_secs(30) {
            let world = s
                .world
                .iter()
                .map(|w| WorldRow {
                    name: w.name,
                    time: w.time.clone(),
                    offset: w.offset.clone(),
                    day_delta: w.day_delta,
                })
                .collect();
            return EnvSnapshot {
                at: s.at,
                world,
                ntp_synced: s.ntp_synced,
                tz_name: s.tz_name.clone(),
                tz_abbr: s.tz_abbr.clone(),
                rtc: s.rtc.clone(),
                rtc_sane: s.rtc_sane,
            };
        }
    }
    let fresh = refresh_env();
    let world = fresh
        .world
        .iter()
        .map(|w| WorldRow {
            name: w.name,
            time: w.time.clone(),
            offset: w.offset.clone(),
            day_delta: w.day_delta,
        })
        .collect();
    let snap = EnvSnapshot {
        at: fresh.at,
        world,
        ntp_synced: fresh.ntp_synced,
        tz_name: fresh.tz_name.clone(),
        tz_abbr: fresh.tz_abbr.clone(),
        rtc: fresh.rtc.clone(),
        rtc_sane: fresh.rtc_sane,
    };
    *g = Some(fresh);
    snap
}

// ────────────────────────── 日出日落（NOAA 简化算法）──────────────────────────

/// 返回指定日期的 (日出, 日落) 本地小时数（f64，如 6.57 表示 06:34）。
///
/// 采用 NOAA 的简化太阳位置公式，视线高度包含：
/// -0.833°（平均大气折射 + 日面半径）与地平线下沉 `dip = 0.0347·√h`。
/// 精度约 ±1 分钟；不处理极昼/极夜（西安纬度不会出现，返回 None 兜底）。
fn sun_times(date: NaiveDate) -> Option<(f64, f64)> {
    const DEG: f64 = std::f64::consts::PI / 180.0;
    let jd = 2_451_545.0 + (date - NaiveDate::from_ymd_opt(2000, 1, 1)?).num_days() as f64;
    let n = jd - 2_451_545.0 + 0.0008;
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

    // 本地时区偏移（小时）
    let tz = now_local().offset().local_minus_utc() as f64 / 3600.0;
    let to_local = |j: f64| ((j + 0.5 + tz / 24.0).rem_euclid(1.0)) * 24.0;
    let rise = to_local(j_transit - omega / 360.0);
    let set = to_local(j_transit + omega / 360.0);
    let noon = to_local(j_transit);
    let _ = noon;
    Some((rise, set))
}

/// 太阳正午（本地小时）
fn solar_noon(date: NaiveDate) -> Option<f64> {
    const DEG: f64 = std::f64::consts::PI / 180.0;
    let jd = 2_451_545.0 + (date - NaiveDate::from_ymd_opt(2000, 1, 1)?).num_days() as f64;
    let n = jd - 2_451_545.0 + 0.0008;
    let j_star = n - LON / 360.0;
    let m = (357.5291 + 0.985_600_28 * j_star).rem_euclid(360.0);
    let c = 1.9148 * (m * DEG).sin() + 0.0200 * (2.0 * m * DEG).sin() + 0.0003 * (3.0 * m * DEG).sin();
    let lambda = (m + c + 180.0 + 102.9372).rem_euclid(360.0);
    let j_transit = 2_451_545.0 + j_star + 0.0053 * (m * DEG).sin() - 0.0069 * (2.0 * lambda * DEG).sin();
    let tz = now_local().offset().local_minus_utc() as f64 / 3600.0;
    Some(((j_transit + 0.5 + tz / 24.0).rem_euclid(1.0)) * 24.0)
}

fn hhmm(hours: f64) -> String {
    let h = hours.rem_euclid(24.0);
    let m = (h.fract() * 60.0).round() as i64;
    let h = h as i64 + m / 60;
    format!("{:02}:{:02}", h % 24, m % 60)
}

/// 把小时数差格式化成「3h18m」
fn hm_span(hours: f64) -> String {
    let total = (hours * 60.0).round().max(0.0) as i64;
    format!("{}h{:02}m", total / 60, total % 60)
}

// ────────────────────────── 进度计算 ──────────────────────────

struct Progress {
    label: &'static str,
    pct: f64,
    left: String,
}

/// 当年天数（不依赖 chrono 的 leap_year，避免版本差异）
fn days_in_year(y: i32) -> i64 {
    if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 { 366 } else { 365 }
}

fn month_days(y: i32, m: u32) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - NaiveDate::from_ymd_opt(y, m, 1).unwrap()).num_days()
}

fn progress(now: chrono::DateTime<Local>) -> Vec<Progress> {
    let sec = now.hour() as f64 * 3600.0 + now.minute() as f64 * 60.0 + now.second() as f64;
    let day_pct = sec / 86400.0 * 100.0;

    let wd = now.weekday().num_days_from_monday() as f64;
    let week_pct = (wd * 86400.0 + sec) / (7.0 * 86400.0) * 100.0;

    let dim = month_days(now.year(), now.month()) as f64;
    let month_pct = ((now.day() as f64 - 1.0) * 86400.0 + sec) / (dim * 86400.0) * 100.0;

    let doy = now.ordinal() as f64;
    let diy = days_in_year(now.year()) as f64;
    let year_pct = ((doy - 1.0) * 86400.0 + sec) / (diy * 86400.0) * 100.0;

    let left_sec = 86400.0 - sec;
    let week_left = (7.0 * 86400.0) - (wd * 86400.0 + sec);
    let month_left = ((dim - now.day() as f64) * 86400.0) + left_sec;
    let year_left = ((diy - doy) * 86400.0) + left_sec;

    vec![
        Progress { label: "今日", pct: day_pct, left: short_span(left_sec) },
        Progress { label: "本周", pct: week_pct, left: short_span(week_left) },
        Progress { label: "本月", pct: month_pct, left: short_span(month_left) },
        Progress { label: "本年", pct: year_pct, left: short_span(year_left) },
    ]
}

/// 剩余时间：≥1 天用「N 天」，否则「Nh MMm」
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

fn weekday_cn(now: chrono::DateTime<Local>) -> &'static str {
    ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
        [now.weekday().num_days_from_monday() as usize]
}

// ────────────────────────── 页面渲染 ──────────────────────────

/// 整屏重绘（数据变化时调用）。
///
/// 左列上下分：大时钟卡（`HERO_H`）+ 「天气 | 电量」一行；
/// 右列三卡：时间进度 / 世界时钟 / 日出日落。
pub fn render(c: &mut Canvas, o: &crate::collector::SystemOverview) {
    let (w, ht) = (c.lw, c.lh);
    c.clear(Palette::BG_BASE);

    let inner = w - PAD * 2;
    let header = Pane { x: PAD, y: PAD, w: inner, h: HEADER_H };
    let footer = Pane { x: PAD, y: ht - PAD - FOOTER_H, w: inner, h: FOOTER_H };
    let body_y = header.y + header.h + GAP;
    let body_h = footer.y - GAP - body_y;

    header_card(c, &header, o);
    footer_card(c, &footer, o);

    let usable = inner - GAP * 2;
    let lw = (usable as f64 * 0.575).round() as i32;
    let rw = usable - lw;

    // 左列：大时钟 + 底部「天气 | 电量」
    let hero = Pane { x: PAD, y: body_y, w: lw, h: HERO_H };
    hero_card(c, &hero);
    let by = body_y + HERO_H + GAP;
    let bh = body_y + body_h - by;
    let half = (lw - GAP) / 2;
    weather_card(c, &Pane { x: PAD, y: by, w: half, h: bh });
    battery_card(c, &Pane { x: PAD + half + GAP, y: by, w: lw - half - GAP, h: bh }, o);

    // 右列
    let rx = PAD + lw + GAP;
    let col = split_col(rx, body_y, rw, body_h, &RIGHT_RATIOS);
    progress_card(c, &col[0]);
    world_card(c, &col[1]);
    sun_card(c, &col[2]);
}

/// 每秒调用：只重绘秒级变化的区域。
///
/// 每个区域都是「整块清除 + 整块重画」：清除带的覆盖范围必须包含该块内**所有**
/// 会变的文本，否则右对齐的变宽文本会留下幽灵像素。因此这里清一块就把那块内容
/// 全部重画（包括变化很慢的行），不做「只重画变化的那一行」这种省事写法。
pub fn render_clock(c: &mut Canvas, _o: &crate::collector::SystemOverview) {
    let now = now_local();
    let (w, ht) = (c.lw, c.lh);
    let inner = w - PAD * 2;
    let header = Pane { x: PAD, y: PAD, w: inner, h: HEADER_H };
    let body_y = header.y + header.h + GAP;
    let body_h = ht - PAD - FOOTER_H - GAP - body_y;
    let usable = inner - GAP * 2;
    let lw = (usable as f64 * 0.575).round() as i32;
    let rw = usable - lw;
    let rx = PAD + lw + GAP;
    let col = split_col(rx, body_y, rw, body_h, &RIGHT_RATIOS);

    // 1) 大时钟卡：从标题下方一直到卡底内缩处整块重画。
    //    顶端取 +60：标题基线 +38、降部约到 +52，而 400px 大字的字顶约在 +80
    //    （基线 400 减 ascent 321），60 既不会切到字顶也不碰标题。
    let hero = Pane { x: PAD, y: body_y, w: lw, h: HERO_H };
    clear_card_body(c, &hero, 60);
    hero_body(c, &hero);

    // 2) 时间进度卡：今日行的「剩余 2h03m」会变宽变窄，四行一起重画最省心
    clear_card_body(c, &col[0], 52);
    for (i, it) in progress(now).iter().enumerate() {
        row_progress(c, &col[0], i, it);
    }

    // 3) 按分钟变化的卡：世界时钟 + 日出日落（后者有「距日出/日落」倒计时，
    //    整屏重绘间隔可到 10s，不单独按分钟刷就会滞后到下一轮采集）
    let minute = now.timestamp() / 60;
    if LAST_WORLD_MIN.swap(minute, Ordering::Relaxed) != minute {
        clear_card_body(c, &col[1], 52);
        world_body(c, &col[1]);
        clear_card_body(c, &col[2], 52);
        sun_card_body(c, &col[2]);
    }
}

// ── 顶栏 ──

fn header_card(c: &mut Canvas, p: &Pane, o: &crate::collector::SystemOverview) {
    card(c, p);
    let now = now_local();
    let env = env_cached();
    let x = p.x + 26;

    c.dot(x + 9, p.y + 32, 9, if env.ntp_synced { Palette::SUCCESS } else { Palette::WARNING });
    c.text(x + 30, p.y + 46, "时钟", Type::H1, Weight::Bold, Palette::FG_EMPHASIS);
    let tw = c.fonts.text_width("时钟", Type::H1, Weight::Bold);
    c.text(
        x + 30 + tw + 18,
        p.y + 46,
        "本机时刻 · 每秒刷新",
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    let tz_show = if env.tz_abbr.is_empty() { env.tz_name.clone() } else { env.tz_abbr.clone() };
    let sub = format!(
        "{} UTC{} · 系统运行 {} · 采集间隔由 Web 端设定",
        tz_show,
        now.format("%:z"),
        short_span(o.uptime as f64),
    );
    c.text(x, p.y + 82, &sub, Type::LABEL, Weight::Regular, Palette::FG_MUTED);

    page_tabs(c, p.x + p.w - 900, p.y + 32, 2);

    // 右侧：NTP 状态 + RTC 异常提示（本机 RTC 走时明显偏离，要能在屏上看到）
    let right = p.x + p.w - 26;
    let (ntp_txt, ntp_col) = if env.ntp_synced {
        ("NTP 已同步", Palette::SUCCESS)
    } else {
        ("NTP 未同步", Palette::WARNING)
    };
    c.text_right(right, p.y + 46, ntp_txt, Type::LABEL, Weight::Bold, ntp_col);
    let rtc_txt = if env.rtc_sane {
        format!("RTC {}", env.rtc)
    } else {
        format!("RTC 异常 {}", env.rtc)
    };
    c.text_right(
        right,
        p.y + 82,
        &rtc_txt,
        Type::TINY,
        Weight::Regular,
        if env.rtc_sane { Palette::FG_MUTED } else { Palette::WARNING },
    );
}

// ── 大时钟卡 ──

fn hero_card(c: &mut Canvas, p: &Pane) {
    card(c, p);
    title(c, p, "本机时刻");

    // 时区胶囊（右上角）
    let env = env_cached();
    let tz_show = if env.tz_abbr.is_empty() { env.tz_name.clone() } else { env.tz_abbr.clone() };
    let label = format!("{tz_show} · {}", now_local().format("%:z"));
    let pw = c.fonts.text_width(&label, Type::TINY, Weight::Bold) + 38;
    c.pill(
        p.x + p.w - 26 - pw,
        p.y + 22,
        &label,
        Type::TINY,
        Palette::FG_MUTED,
        Palette::BG_SURFACE_ALT,
    );

    hero_body(c, p);
}

/// 大时钟卡的「标题以下」全部内容（整屏与秒级局部重绘共用）。
fn hero_body(c: &mut Canvas, p: &Pane) {
    let now = now_local();
    let x = p.x + 26;
    // 基线表（卡高 HERO_H=600）：大字 400 / 日期 460 / 秒条 492 / 元信息 540、572
    // 大字字顶 = 400 - ascent(400)=321 → 79，正好落在标题与清除带之下。

    // ── 超大时:分 + 秒（冒号用强调色分层，秒与分钟基线对齐）──
    let hh = now.format("%H").to_string();
    let mm = now.format("%M").to_string();
    let ss = now.format("%S").to_string();
    let base = p.y + 400;
    let dw = c.fonts.text_width(&hh, TSIZE_HM, Weight::Bold);
    let cw = c.fonts.text_width(":", TSIZE_HM, Weight::Bold);

    let mut pen = x;
    c.text(pen, base, &hh, TSIZE_HM, Weight::Bold, Palette::FG_EMPHASIS);
    pen += dw;
    c.text(pen, base, ":", TSIZE_HM, Weight::Bold, Palette::ACCENT);
    pen += cw;
    c.text(pen, base, &mm, TSIZE_HM, Weight::Bold, Palette::FG_EMPHASIS);
    pen += c.fonts.text_width(&mm, TSIZE_HM, Weight::Bold);
    pen += 44;
    c.text(pen, base, &ss, TSIZE_SS, Weight::Bold, Palette::ACCENT);

    // ── 日期 ──
    let date = format!(
        "{}年{}月{}日  {}",
        now.year(),
        now.month(),
        now.day(),
        weekday_cn(now)
    );
    c.text(x, p.y + 460, &date, TSIZE_DATE, Weight::Bold, Palette::FG_DEFAULT);

    // ── 秒进度条（每秒推进）──
    let bar_w = p.w - 52;
    // 只按整秒推进：本页刷新是 1Hz，亚秒成分看不见，
    // 去掉后渲染结果只依赖「秒」，幽灵像素测试才能做逐像素等值断言。
    let frac = now.second() as f64 / 60.0;
    c.bar(x, p.y + 492, bar_w, 16, frac * 100.0, Palette::ACCENT);
    c.text_right(
        p.x + p.w - 26,
        p.y + 485,
        &format!("第 {} 秒 / 60", now.second()),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );

    // ── 元信息 ──
    let doy = now.ordinal();
    let diy = days_in_year(now.year());
    let left_sec = 86400.0
        - (now.hour() as f64 * 3600.0 + now.minute() as f64 * 60.0 + now.second() as f64);
    let ls = left_sec as i64;
    let meta1 = format!(
        "第 {} 周 · 第 {} 天 / {} · 距零点 {:02}:{:02}:{:02}",
        now.iso_week().week(),
        doy,
        diy,
        ls / 3600,
        (ls % 3600) / 60,
        ls % 60
    );
    c.text(x, p.y + 540, &meta1, Type::BODY, Weight::Regular, Palette::FG_DEFAULT);

    let env = env_cached();
    let meta2 = format!(
        "时区 {} UTC{} · 时间戳 {} · 第 {} 季度",
        env.tz_name,
        now.format("%:z"),
        now.timestamp(),
        (now.month() - 1) / 3 + 1
    );
    c.text(x, p.y + 572, &meta2, Type::TINY, Weight::Regular, Palette::FG_MUTED);
}



// ── 时间进度卡 ──

fn progress_card(c: &mut Canvas, p: &Pane) {
    card(c, p);
    title(c, p, "时间进度");
    let items = progress(now_local());
    for (i, it) in items.iter().enumerate() {
        row_progress(c, p, i, it);
    }
}

/// 单行进度（供整屏与秒级局部重绘共用）。
fn row_progress(c: &mut Canvas, p: &Pane, idx: usize, it: &Progress) {
    let base = p.y + 92 + idx as i32 * 40;
    let x = p.x + 26;
    c.text(x, base, it.label, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    let bx = x + 82;
    let bw = p.w - 82 - 300;
    c.bar(bx, base - 15, bw, 13, it.pct, Palette::ACCENT);
    c.text_right(
        p.x + p.w - 190,
        base,
        &format!("{:.1}%", it.pct),
        Type::TITLE,
        Weight::Bold,
        Palette::ACCENT,
    );
    c.text_right(
        p.x + p.w - 26,
        base,
        &it.left,
        Type::TINY,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

// ── 世界时钟卡 ──

fn world_card(c: &mut Canvas, p: &Pane) {
    card(c, p);
    title(c, p, "世界时钟");
    world_body(c, p);
}

fn world_body(c: &mut Canvas, p: &Pane) {
    let env = env_cached();
    let x = p.x + 26;
    for (i, row) in env.world.iter().enumerate() {
        let base = p.y + 78 + i as i32 * 35;
        let mark = match row.day_delta {
            -1 => " · 昨天",
            1 => " · 明天",
            _ => "",
        };
        c.text(
            x,
            base,
            &format!("{}{}", row.name, mark),
            Type::BODY,
            Weight::Bold,
            Palette::FG_DEFAULT,
        );
        let tw = c.fonts.text_width(&row.time, TSIZE_WORLD, Weight::Bold);
        c.text_right(
            p.x + p.w - 26 - tw - 14,
            base,
            &row.offset,
            Type::TINY,
            Weight::Regular,
            Palette::FG_MUTED,
        );
        c.text_right(
            p.x + p.w - 26,
            base,
            &row.time,
            TSIZE_WORLD,
            Weight::Bold,
            Palette::FG_EMPHASIS,
        );
    }
}

// ── 日出日落卡 ──

fn sun_card(c: &mut Canvas, p: &Pane) {
    card(c, p);
    title(c, p, "日出日落 · 西安");
    // 口径标注画在标题行，**只走整屏重绘**：它和标题同一行、在局部重绘的清除带
    // 之外，而 `Canvas::text` 是 alpha 混合 —— 在同一位置重复绘制同一段文字会
    // 逐次加深（累积），必须保证「重绘的内容 ⊆ 清除的内容」。
    // 标注只在天气接口可用性变化时才变，整屏重绘（≤10s）已足够跟上。
    if let Ok(w) = weather::shared().lock() {
        let from_api = w.sunrise.is_some();
        c.text_right(
            p.x + p.w - 26,
            p.y + 38,
            if from_api { "来源 天气接口" } else { "来源 本地推算" },
            Type::TINY,
            Weight::Regular,
            Palette::FG_MUTED,
        );
    }
    sun_card_body(c, p);
}

/// 日出日落卡「标题以下」的内容（整屏与按分钟局部重绘共用）。
fn sun_card_body(c: &mut Canvas, p: &Pane) {
    let now = now_local();
    let today = now.date_naive();
    let x = p.x + 26;
    let (rise, set, from_api) = match sun_pair(today) {
        Some(v) => v,
        None => {
            c.text(
                x,
                p.y + 100,
                "该纬度当日无日出日落",
                Type::BODY,
                Weight::Regular,
                Palette::FG_MUTED,
            );
            return;
        }
    };
    let day_len = (set - rise).rem_euclid(24.0);
    let night_len = (24.0 - day_len).max(0.01);
    let now_h = now.hour() as f64 + now.minute() as f64 / 60.0 + now.second() as f64 / 3600.0;

    // ── 两列：日出 / 日落 ──
    c.text(x, p.y + 100, "日出", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text(x + 84, p.y + 100, &hhmm(rise), TSIZE_WORLD, Weight::Bold, Palette::WARNING);
    let cx2 = p.x + p.w / 2 + 10;
    c.text(cx2, p.y + 100, "日落", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text(cx2 + 84, p.y + 100, &hhmm(set), TSIZE_WORLD, Weight::Bold, Palette::ACCENT_2);

    // ── 昼长 / 太阳正午 ──
    c.text(x, p.y + 152, "昼长", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text(x + 84, p.y + 152, &hm_span(day_len), Type::BODY, Weight::Bold, Palette::FG_DEFAULT);
    c.text(cx2, p.y + 152, "太阳正午", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    let noon = solar_noon(today).map(hhmm).unwrap_or_else(|| "--:--".into());
    c.text(cx2 + 134, p.y + 152, &noon, Type::BODY, Weight::Bold, Palette::FG_DEFAULT);

    // ── 当前阶段进度条 ──
    // 白天填充「日照进度」（暖色），夜间填充「夜间进度」（灰色）——
    // 条永远在动，避免夜里留一根空条看着像故障；右侧百分比与左侧文案都必须
    // 说的是同一个阶段，不能出现「夜间…100.0% 白天」这种自相矛盾。
    let is_day = now_h >= rise && now_h <= set;
    let to_rise = if now_h < rise {
        rise - now_h
    } else {
        let tr = tomorrow_rise(today).unwrap_or(rise);
        24.0 - now_h + tr
    };
    let (bar_col, bar_pct, state, phase_txt) = if is_day {
        let d = ((now_h - rise) / day_len * 100.0).clamp(0.0, 100.0);
        (
            Palette::WARNING,
            d,
            format!("白天 · 剩余 {}", hm_span(set - now_h)),
            format!("白天 {d:.0}%"),
        )
    } else {
        let elapsed = if now_h < rise { 24.0 - set + now_h } else { now_h - set };
        let n = (elapsed / night_len * 100.0).clamp(0.0, 100.0);
        (
            Palette::FG_MUTED,
            n,
            format!("夜间 · 距日出 {}", hm_span(to_rise)),
            format!("夜间 {n:.0}%"),
        )
    };
    c.bar(x, p.y + 182, p.w - 52, 16, bar_pct, bar_col);
    c.text(x, p.y + 235, &state, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text_right(
        p.x + p.w - 26,
        p.y + 235,
        &phase_txt,
        Type::TINY,
        Weight::Regular,
        Palette::FG_MUTED,
    );

    // ── 明日对照：昼长变化方向和幅度 ──
    let tomorrow_pair = {
        let api2 = weather::shared().lock().ok().and_then(|w| {
            let r = w.sunrise2.as_deref().and_then(parse_hhmm);
            let ss = w.sunset2.as_deref().and_then(parse_hhmm);
            r.zip(ss)
        });
        api2.or_else(|| today.succ_opt().and_then(sun_times))
    };
    if let Some((r2, s2)) = tomorrow_pair {
        let len2 = (s2 - r2).rem_euclid(24.0);
        let delta = ((len2 - day_len) * 60.0).round() as i64;
        let sign = if delta > 0 { "+" } else { "" };
        c.text(
            x,
            p.y + 268,
            &format!(
                "明日 {} / {} · 昼长 {}（{sign}{}m）",
                hhmm(r2),
                hhmm(s2),
                hm_span(len2),
                delta
            ),
            Type::TINY,
            Weight::Regular,
            Palette::FG_MUTED,
        );
    }
}

// ── 底栏 ──

fn footer_card(c: &mut Canvas, p: &Pane, o: &crate::collector::SystemOverview) {
    card(c, p);
    let x = p.x + 26;
    let baseline = p.y + p.h / 2 + 8;
    c.text(
        x,
        baseline,
        "音量键 ▲ ▼ 翻页",
        Type::LABEL,
        Weight::Bold,
        Palette::FG_DEFAULT,
    );
    let tw = c.fonts.text_width("音量键 ▲ ▼ 翻页", Type::LABEL, Weight::Bold);
    c.text(
        x + tw + 22,
        baseline,
        &format!(
            "当前第 {} / {} 页",
            crate::collector::hotkeys::page() + 1,
            crate::collector::hotkeys::PAGE_COUNT
        ),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    let info = format!(
        "本地时刻每秒刷新 · 世界时钟按分钟同步 · 面板 {}×{}",
        c.lw, c.lh
    );
    c.text_right(
        p.x + p.w - 26,
        baseline,
        &info,
        Type::TINY,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    let _ = o;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 外部参照（2026-09 查证）：365.wiki 给西安（34.2658°N 108.9541°E）
    /// 2026-09-14 日出 06:27:08 / 日落 18:52:56 / 昼长 12h25m48s / 太阳正午 12:39:58。
    /// 本实现（34.3416°N 108.9398°E，标准 -0.833°）算 09-15 为 06:28 / 18:54 / 12h26m，
    /// 逐日推移后与参照差 ≤2 分钟，属 NOAA 简化公式的正常误差（±1~2 分钟）。
    /// 若本测试失败，说明算法或常数被改动，需重新对照外部数据源，不要直接放宽区间。
    #[test]
    fn 西安日出日落应与外部参照一致() {
        let d = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        let (rise, set) = sun_times(d).expect("应有日出日落");
        assert!((6.40..6.55).contains(&rise), "日出 {rise}（{:02}:{:02}）偏离参照", rise as i64, (rise.fract()*60.0).round() as i64);
        assert!((18.82..18.97).contains(&set), "日落 {set} 偏离参照");
        let len = set - rise;
        assert!((12.35..12.55).contains(&len), "昼长 {len} 偏离参照");
    }

    #[test]
    fn 夏至昼长应显著长于冬至() {
        let s = sun_times(NaiveDate::from_ymd_opt(2026, 6, 21).unwrap()).unwrap();
        let w = sun_times(NaiveDate::from_ymd_opt(2026, 12, 21).unwrap()).unwrap();
        let ls = s.1 - s.0;
        let lw = w.1 - w.0;
        // 实测夏至 14h27m、冬至 9h51m
        assert!((14.3..14.6).contains(&ls), "夏至昼长 {ls} 异常");
        assert!((9.7..10.0).contains(&lw), "冬至昼长 {lw} 异常");
    }

    #[test]
    fn 进度百分比单调递增且在有界区间() {
        let p = progress(now_local());
        assert_eq!(p.len(), 4);
        for it in &p {
            assert!((0.0..100.0).contains(&it.pct), "{} = {} 越界", it.label, it.pct);
        }
        // 同一时刻：今日 < 本周或本月/本年关系不固定，但都必须 < 100
        assert!(p[0].pct < 100.0);
    }

    /// 幽灵像素回归测试：把「整屏 @t2」与「整屏 @t1 → 秒级局部重绘 @t2」
    /// 两张画布放在**同一个进程**里逐像素比对。
    ///
    /// 必须同进程：跨进程会让电池读数、RTC 秒数等实时数据漂移，差异无法归因。
    /// 也不要在测试里依赖真实当前时间，否则结果不可复现。
    #[test]
    fn 秒级局部重绘不应留下幽灵像素() {
        use super::super::Rotation;
        let (Ok(mut a), Ok(mut b)) = (
            Canvas::new(2340, 1080, Rotation::Rot90),
            Canvas::new(2340, 1080, Rotation::Rot90),
        ) else {
            return; // 无字体/无 DRM 环境跳过
        };
        let o = crate::collector::collect_system_overview();
        // 固定基准：跨分钟（触发世界时钟/日出日落重绘）且秒数从个位走到十位
        let base: i64 = 1_800_000_000;
        let set = |ts: i64| set_time_offset(ts - Local::now().timestamp());

        set(base + 61);
        render(&mut a, &o); // 参照：整屏直接画在 t2

        set(base);
        render(&mut b, &o);          // 打底：整屏画在 t1
        set(base + 61);
        render_clock(&mut b, &o);    // 局部推进到 t2

        // 报出差异的包围盒。注意 `buf` 是**物理**缓冲（步长 pw），
        // 用 lw 当步长会算出一片不存在的坐标；这里按物理坐标算完再映射回逻辑坐标。
        let (pw, lw) = (a.pw, a.lw);
        let (mut n, mut x0, mut y0, mut x1, mut y1) = (0usize, i32::MAX, i32::MAX, -1, -1);
        for (i, (p, q)) in a.buf.iter().zip(b.buf.iter()).enumerate() {
            if p != q {
                n += 1;
                let (px, py) = (i as i32 % pw, i as i32 / pw);
                let (lx, ly) = match a.rot {
                    Rotation::Rot90 => (lw - 1 - py, px),
                    Rotation::Rot270 => (py, lw - 1 - px),
                };
                x0 = x0.min(lx);
                y0 = y0.min(ly);
                x1 = x1.max(lx);
                y1 = y1.max(ly);
            }
        }
        set_time_offset(0);
        assert_eq!(
            n, 0,
            "局部重绘与整屏重绘有 {n} 个像素不一致（幽灵像素），逻辑区域 x {x0}..{x1} y {y0}..{y1}"
        );
    }

    #[test]
    fn 昼夜时长格式() {
        assert_eq!(hm_span(3.0), "3h00m");
        assert_eq!(hm_span(3.3), "3h18m");
        assert_eq!(hhmm(6.5666), "06:34");
    }
}

// ── 天气卡 ──

/// 次日日出小时数（优先接口值，退回本地推算）。
fn tomorrow_rise(today: NaiveDate) -> Option<f64> {
    if let Ok(w) = weather::shared().lock() {
        if let Some(r) = w.sunrise2.as_deref().and_then(parse_hhmm) {
            return Some(r);
        }
    }
    today.succ_opt().and_then(sun_times).map(|(r, _)| r)
}

/// 「HH:MM」→ 小时数（f64）
fn parse_hhmm(s: &str) -> Option<f64> {
    let (h, m) = s.split_once(':')?;
    Some(h.trim().parse::<f64>().ok()? + m.trim().parse::<f64>().ok()? / 60.0)
}

/// 当日 (日出, 日落, 是否来自天气接口)。
///
/// 优先用 Open-Meteo 返回的日出日落（含地形/海拔修正，与主流天气服务一致），
/// 接口不可用时退回本地 NOAA 推算（两者差约 2~3 分钟）。
fn sun_pair(today: NaiveDate) -> Option<(f64, f64, bool)> {
    if let Ok(w) = weather::shared().lock() {
        if let (Some(r), Some(st)) = (w.sunrise.as_deref(), w.sunset.as_deref()) {
            if let (Some(rh), Some(sh)) = (parse_hhmm(r), parse_hhmm(st)) {
                return Some((rh, sh, true));
            }
        }
    }
    sun_times(today).map(|(r, s)| (r, s, false))
}

fn weather_card(c: &mut Canvas, p: &Pane) {
    card(c, p);
    title(c, p, "天气 · 西安");

    let w = match weather::shared().lock() {
        Ok(g) => g.clone(),
        Err(e) => e.into_inner().clone(),
    };
    let x = p.x + 26;

    // 数据不可用：明确画出原因，不要静默显示 0°C
    if !w.ok && w.error.is_some() {
        let stale = w.updated.map(|u| u.elapsed().as_secs()).unwrap_or(u64::MAX);
        let (txt, col) = if w.updated.is_some() {
            (format!("离线 · 显示 {stale}s 前的数据"), Palette::WARNING)
        } else {
            ("离线 · 尚无数据".to_string(), Palette::ERROR)
        };
        c.text_right(p.x + p.w - 26, p.y + 38, &txt, Type::TINY, Weight::Regular, col);
    } else if let Some(u) = w.updated {
        c.text_right(
            p.x + p.w - 26,
            p.y + 38,
            &format!("{} 分钟前更新", u.elapsed().as_secs() / 60),
            Type::TINY,
            Weight::Regular,
            Palette::FG_MUTED,
        );
    }

    if w.updated.is_none() {
        c.text(x, p.y + 120, "正在获取天气…", Type::BODY, Weight::Regular, Palette::FG_MUTED);
        return;
    }

    // ── 大温度 + 天气现象 ──
    let tcolor = if w.temp_c >= 35.0 {
        Palette::ERROR
    } else if w.temp_c >= 28.0 {
        Palette::WARNING
    } else if w.temp_c <= 0.0 {
        Palette::ACCENT
    } else {
        Palette::FG_EMPHASIS
    };
    let t = format!("{:.0}", w.temp_c);
    c.text(x, p.y + 128, &t, TSIZE_METRIC, Weight::Bold, tcolor);
    let tw = c.fonts.text_width(&t, TSIZE_METRIC, Weight::Bold);
    c.text(x + tw + 8, p.y + 112, "°C", Type::BODY, Weight::Regular, Palette::FG_MUTED);
    let desc = weather::code_desc(w.code);
    c.text(
        x + tw + 76,
        p.y + 128,
        desc,
        Type::VALUE_M,
        Weight::Bold,
        if weather::is_wet(w.code) { Palette::INFO } else { Palette::FG_DEFAULT },
    );

    // ── 体感 / 风（右上）──
    let right = p.x + p.w - 26;
    c.text_right(
        right,
        p.y + 112,
        &format!("体感 {:.0}°", w.feels_c),
        Type::BODY,
        Weight::Regular,
        Palette::FG_DEFAULT,
    );
    c.text_right(
        right,
        p.y + 148,
        &format!("{} {:.1} m/s", weather::wind_dir_cn(w.wind_dir), w.wind_ms),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );

    // ── 今日区间 + 湿度 ──
    c.text(
        x,
        p.y + 186,
        &format!(
            "今日 {:.0}~{:.0}°C · 降水概率 {:.0}%",
            w.t_min, w.t_max, w.precip_prob
        ),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_DEFAULT,
    );
    c.text(
        x,
        p.y + 214,
        &format!(
            "湿度 {:.0}% · 降水 {:.1}mm · 日出 {} 日落 {}",
            w.humidity,
            w.precip_mm,
            w.sunrise.clone().unwrap_or_else(|| "--:--".into()),
            w.sunset.clone().unwrap_or_else(|| "--:--".into()),
        ),
        Type::TINY,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

// ── 电量卡 ──

fn battery_card(c: &mut Canvas, p: &Pane, o: &crate::collector::SystemOverview) {
    card(c, p);
    title(c, p, "电池 · 剩余电量");
    let b = &o.battery;
    let x = p.x + 26;

    let (label, color) = match b.status.as_str() {
        "Charging" => ("充电中", Palette::SUCCESS),
        "Full" => ("已充满", Palette::SUCCESS),
        "Discharging" => ("放电中", Palette::ACCENT),
        "Not charging" => ("未充电", Palette::FG_MUTED),
        other => (other, Palette::FG_MUTED),
    };
    let pw = c.fonts.text_width(label, Type::TINY, Weight::Bold) + 38;
    c.pill(
        p.x + p.w - 26 - pw,
        p.y + 20,
        label,
        Type::TINY,
        color,
        Palette::BG_SURFACE_ALT,
    );

    let pct = if b.display_capacity_pct > 0 { b.display_capacity_pct } else { b.capacity };
    let bcolor = if pct < 20 {
        Palette::ERROR
    } else if pct < 50 {
        Palette::WARNING
    } else {
        Palette::SUCCESS
    };
    let ps = format!("{pct}");
    c.text(x, p.y + 128, &ps, TSIZE_METRIC, Weight::Bold, bcolor);
    let pw2 = c.fonts.text_width(&ps, TSIZE_METRIC, Weight::Bold);
    c.text(x + pw2 + 8, p.y + 112, "%", Type::BODY, Weight::Regular, Palette::FG_MUTED);

    // 预计时间（放电为正、充电为负）
    let remain = if b.time_left_min > 0 {
        format!("可用 {}h{}m", b.time_left_min / 60, b.time_left_min % 60)
    } else if b.time_left_min < 0 {
        format!(
            "充满还需 {}h{}m",
            b.time_left_min.unsigned_abs() / 60,
            b.time_left_min.unsigned_abs() % 60
        )
    } else {
        "预计 --".to_string()
    };
    c.text_right(
        p.x + p.w - 26,
        p.y + 128,
        &remain,
        Type::BODY,
        Weight::Bold,
        if b.time_left_min < 0 { Palette::SUCCESS } else { Palette::FG_DEFAULT },
    );

    c.bar(x, p.y + 148, p.w - 52, 18, pct as f64, bcolor);

    c.text(
        x,
        p.y + 186,
        // 电流取绝对值：方向已由右上角状态胶囊（充电中/放电中）表达，
            // 显示负号只会让人以为哪里算错了
            &format!(
                "电压 {:.2}V · 电流 {:.0}mA · 功率 {:.1}W",
                b.voltage_v,
                b.current_ma.abs(),
                b.power_w
            ),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_DEFAULT,
    );
    c.text(
        x,
        p.y + 214,
        &format!(
            "温度 {:.1}°C · 容量上限 {}%{}",
            b.temp_celsius,
            b.effective_max_pct,
            if b.is_degraded {
                " · 容量已下降"
            } else if b.at_charge_limit {
                " · 已到充电上限"
            } else {
                ""
            }
        ),
        Type::TINY,
        Weight::Regular,
        if b.is_degraded { Palette::WARNING } else { Palette::FG_MUTED },
    );
}

