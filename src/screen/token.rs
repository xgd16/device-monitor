//! XTokenHub 用量仪表（物理屏第 2 页）。
//!
//! 数据源**完全走 XTokenHub 自己的接口**（不新建接口、不读库）：
//! - REST：`GET /api/v1/stats/*`、`/channels`、`/channels/balances`、`/settings/billing`
//! - WebSocket：`GET /api/v1/ws`，事件信封 `{type, payload, ts}`
//!   - `stats.throughput`（2Hz）→ `{tokens_per_sec, active_streams}` 实时吞吐
//!   - `request.completed` → 完整 RequestLog（最近一次调用）
//!   - `stats.updated` → 提示统计已变化（据此限频刷新 REST）
//!   - `channel.balance_updated` / `channel.status_changed` / `channel.probe_result`
//!
//! 后台线程负责 HTTP 轮询 + WS 长连接，渲染线程只读快照；版式沿用第 1 页规范
//! （Widget Dashboard + 语义色板 + 层次：标题 muted、主数值 emphasis 大字、元数据降级）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::TimeZone;

use serde::Deserialize;

use super::canvas::Canvas;
use super::font::Weight;
use super::layout::{Pane, card, split_col, title, truncate};
use super::theme::{Argb, Palette, Type};

const BASE: &str = "http://127.0.0.1:9192/api/v1";
const WS_URL: &str = "ws://127.0.0.1:9192/api/v1/ws";
/// REST 快照刷新间隔（秒）；收到 stats.updated 时最多提前到 5s 一次
const HTTP_REFRESH_SECS: u64 = 15;
const HTTP_MIN_GAP_SECS: u64 = 5;
/// 实时吞吐滑窗点数（WS 2Hz/秒 → 120 点约 1 分钟）
const TPS_POINTS: usize = 120;

// ────────────────────────── 接口数据结构 ──────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub total_requests: i64,
    pub success_requests: i64,
    pub error_requests: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    pub cache_hit_rate: f64,
    pub avg_duration_ms: f64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TrendPoint {
    pub date: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub error_requests: i64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GroupRow {
    pub name: String,
    pub requests: i64,
    pub total_tokens: i64,
    pub cached_tokens: i64,
    pub cache_rate: f64,
    pub avg_ms: f64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Lifetime {
    pub total_tokens: i64,
    pub peak_day_tokens: i64,
    pub peak_day: String,
    pub max_duration_ms: i64,
    pub current_streak: i64,
    pub max_streak: i64,
    pub active_days: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Forecast {
    pub period: String,
    pub spent_usd: f64,
    /// 样本不足时服务端返回 null
    pub projected_usd: Option<f64>,
    pub burn_per_hour_usd: Option<f64>,
    pub daily_avg_usd: f64,
    pub confidence: String,
    pub reason: String,
    pub budget_usd: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Billing {
    pub display_currency: String,
    pub usd_cny_rate: f64,
    pub monthly_budget_usd: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Balance {
    pub currency: String,
    pub total: f64,
    pub granted: f64,
    pub topped_up: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct BalanceItem {
    pub channel_id: i64,
    pub channel_name: String,
    pub supported: bool,
    pub ok: bool,
    pub balance: Option<Balance>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ChannelItem {
    pub id: i64,
    pub name: String,
    pub provider: String,
    pub status: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BalanceResp {
    items: Vec<BalanceItem>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ChannelsResp {
    items: Vec<ChannelItem>,
}

// ────────────────────────── 数据快照 + 后台采集 ──────────────────────────

/// 实时事件状态（来自 WS）
#[derive(Default)]
pub struct Live {
    /// 仅需要"连上没连上"，不记录收发量/事件明细
    pub ws_connected: bool,
    /// 最近一次完成的请求摘要："模型 · 渠道 · key · tokens · ¥ · 耗时"
    pub last_req: String,
    /// tokens/s 滑窗
    pub tps: VecDeque<f32>,
    pub active_streams: i64,
}

/// 页面所需的全部数据（REST 快照 + WS 实时状态）
#[derive(Default)]
pub struct TokenFeed {
    pub summary_today: Summary,
    pub summary_7d: Summary,
    pub trend: Vec<TrendPoint>,
    pub by_model: Vec<GroupRow>,
    pub by_channel: Vec<GroupRow>,
    pub by_key: Vec<GroupRow>,
    pub lifetime: Lifetime,
    pub forecast: Forecast,
    pub billing: Billing,
    pub balances: Vec<BalanceItem>,
    pub channels: Vec<ChannelItem>,
    pub live: Live,

    pub http_ok: bool,
    pub http_at: Option<Instant>,
    pub http_ms: u128,
    pub last_error: Option<String>,
    last_http: Option<Instant>,
}

impl TokenFeed {
    pub fn new() -> Self {
        Self {
            billing: Billing { display_currency: "CNY".into(), usd_cny_rate: 7.2, ..Default::default() },
            ..Default::default()
        }
    }

    fn currency(&self) -> &str {
        if self.billing.display_currency.is_empty() { "CNY" } else { &self.billing.display_currency }
    }

    fn rate(&self) -> f64 {
        if self.billing.usd_cny_rate > 0.0 { self.billing.usd_cny_rate } else { 7.2 }
    }

    /// 拉一遍全部 REST 端点（只读，总耗时毫秒级）。
    pub fn refresh_http(&mut self) {
        let t0 = Instant::now();
        let mut err: Option<String> = None;

        macro_rules! get_json {
            ($path:expr, $ty:ty) => {
                match http_get::<$ty>(&format!("{BASE}{}", $path)) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        err = Some(e);
                        None
                    }
                }
            };
        }

        // 今日（本地日历日 00:00 起，用 since=unix 秒；不是滚动 24h）
        let midnight = chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|d| chrono::Local.from_local_datetime(&d).single())
            .map(|d| d.timestamp())
            .unwrap_or(0);
        let today_path = format!("/stats/summary?since={midnight}");
        if let Some(s) = get_json!(&today_path, Summary) {
            self.summary_today = s;
        }
        // 7 天汇总（质量指标）
        if let Some(s) = get_json!("/stats/summary?hours=168", Summary) {
            self.summary_7d = s;
        }
        if let Some(v) = get_json!("/stats/trend?hours=168&bucket=day", Vec<TrendPoint>) {
            self.trend = v;
        }
        if let Some(v) = get_json!("/stats/by-model?hours=168", Vec<GroupRow>) {
            self.by_model = v;
        }
        if let Some(v) = get_json!("/stats/by-channel?hours=168", Vec<GroupRow>) {
            self.by_channel = v;
        }
        if let Some(v) = get_json!("/stats/by-key?hours=168", Vec<GroupRow>) {
            self.by_key = v;
        }
        if let Some(v) = get_json!("/stats/lifetime", Lifetime) {
            self.lifetime = v;
        }
        if let Some(v) = get_json!("/stats/cost/forecast", Forecast) {
            self.forecast = v;
        }
        if let Some(v) = get_json!("/settings/billing", Billing) {
            if !v.display_currency.is_empty() {
                self.billing = v;
            }
        }
        if let Some(v) = get_json!("/channels/balances", BalanceResp) {
            self.balances = v.items;
        }
        if let Some(v) = get_json!("/channels", ChannelsResp) {
            self.channels = v.items;
        }

        self.http_ms = t0.elapsed().as_millis();
        self.http_at = Some(Instant::now());
        self.last_http = self.http_at;
        self.http_ok = err.is_none();
        self.last_error = err;
    }

    /// 处理一条 WS 消息。
    pub fn apply_ws(&mut self, text: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
            return;
        };
        let typ = v["type"].as_str().unwrap_or("");
        if typ.is_empty() {
            return;
        }

        match typ {
            "stats.throughput" => {
                let tps = v["payload"]["tokens_per_sec"].as_f64().unwrap_or(0.0) as f32;
                self.live.active_streams = v["payload"]["active_streams"].as_i64().unwrap_or(0);
                self.live.tps.push_back(tps);
                while self.live.tps.len() > TPS_POINTS {
                    self.live.tps.pop_front();
                }
            }
            "request.completed" => {
                let p = &v["payload"];
                let model = p["model"].as_str().unwrap_or("-");
                let channel = p["channel_name"].as_str().unwrap_or("-");
                let key = p["key_name"].as_str().unwrap_or("-");
                let tokens = p["total_tokens"].as_i64().unwrap_or(0);
                let cost = p["cost_usd"].as_f64().unwrap_or(0.0);
                let ms = p["duration_ms"].as_i64().unwrap_or(0);
                let rate = self.rate();
                let money = if self.currency().eq_ignore_ascii_case("CNY") {
                    format!("¥{:.4}", cost * rate)
                } else {
                    format!("${cost:.4}")
                };
                self.live.last_req = format!(
                    "{model} · {channel} · {key} · {} tokens · {money} · {}s",
                    fmt_tokens(tokens),
                    ms as f64 / 1000.0
                );
            }
            "stats.updated" => {
                // 统计发生了变化：限频刷新 REST（≥5s 一次），避免请求风暴
                let due = self
                    .last_http
                    .map(|t| t.elapsed().as_secs() >= HTTP_MIN_GAP_SECS)
                    .unwrap_or(true);
                if due {
                    self.refresh_http();
                }
            }
            "channel.balance_updated" => {
                let name = v["payload"]["channel_name"].as_str().unwrap_or("");
                if let Some(b) = v["payload"]["balance"].clone().into() {
                    let item: Option<BalanceItem> = serde_json::from_value(serde_json::json!({
                        "channel_id": v["payload"]["channel_id"],
                        "channel_name": name,
                        "supported": true,
                        "ok": true,
                        "balance": b,
                    }))
                    .ok();
                    if let Some(item) = item {
                        if let Some(slot) = self
                            .balances
                            .iter_mut()
                            .find(|x| x.channel_name == item.channel_name)
                        {
                            *slot = item;
                        } else {
                            self.balances.push(item);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// GET + 解析 `{code,message,data}`
fn http_get<T: for<'de> Deserialize<'de>>(url: &str) -> Result<T, String> {
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(6))
        .call()
        .map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let text = resp.into_string().map_err(|e| format!("读取 {url} 失败: {e}"))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("解析 {url} 失败: {e}"))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
        return Err(format!("{url} 返回 code={code} {msg}"));
    }
    let data = v.get("data").cloned().unwrap_or(serde_json::Value::Null);
    serde_json::from_value(data).map_err(|e| format!("解析 {url} 的 data 失败: {e}"))
}

/// 切到 Token 页时请求尽快刷一次 REST。
///
/// **只能置标志，不能就地刷**：`refresh_http()` 走 ureq，单请求超时 6s、
/// 一次还连发好几个；以前渲染线程在切页分支里同步调用它，网络一慢就把
/// 「按键 → 翻页」整条路径堵住。现在由后台 feed 线程消费这个标志。
static REFRESH_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 请求后台线程尽快刷新一次（非阻塞，可在渲染线程调用）。
pub fn request_refresh() {
    REFRESH_REQUESTED.store(true, Ordering::Relaxed);
}

fn take_refresh_request() -> bool {
    REFRESH_REQUESTED.swap(false, Ordering::Relaxed)
}

/// 启动后台采集线程（HTTP 轮询 + WS 长连接），返回共享快照。
pub fn start_feed() -> Arc<Mutex<TokenFeed>> {
    let feed = Arc::new(Mutex::new(TokenFeed::new()));
    let handle = feed.clone();
    let spawned = std::thread::Builder::new()
        .name("xtokenhub-feed".into())
        .spawn(move || {
            if let Ok(mut f) = handle.lock() {
                f.refresh_http();
            }
            let mut last_http = Instant::now();
            loop {
                // 一次 WS 会话（阻塞直到断开）
                if let Err(e) = ws_session(&handle, &mut last_http) {
                    tracing::warn!("xtokenhub: WS 断开 → {e}（3s 后重连）");
                    if let Ok(mut f) = handle.lock() {
                        f.live.ws_connected = false;
                        f.last_error = Some(e);
                    }
                }
                // 断线退避 3s，期间保持 REST 新鲜
                for _ in 0..6 {
                    std::thread::sleep(Duration::from_millis(500));
                    // 被「切到 Token 页」请求刷新时立刻刷；但至少隔 5s，
                    // 避免用户连按翻页把接口刷爆
                    let due = last_http.elapsed().as_secs() >= HTTP_REFRESH_SECS
                        || (take_refresh_request() && last_http.elapsed().as_secs() >= 5);
                    if due {
                        if let Ok(mut f) = handle.lock() {
                            f.refresh_http();
                        }
                        last_http = Instant::now();
                    }
                }
            }
        })
        .expect("无法启动 xtokenhub-feed 线程");
    tracing::info!("xtokenhub: 采集线程已启动 ({:?})", spawned.thread().name());
    feed
}

/// 单次 WS 会话：连接 → 读消息（读超时用于定期刷新 REST）→ 断开时返回 Err。
fn ws_session(feed: &Arc<Mutex<TokenFeed>>, last_http: &mut Instant) -> Result<(), String> {
    let (mut socket, _resp) =
        tungstenite::connect(WS_URL).map_err(|e| format!("连接 {WS_URL} 失败: {e}"))?;
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = socket.get_ref() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(5000)));
    }
    {
        let mut f = feed.lock().map_err(|e| e.to_string())?;
        f.live.ws_connected = true;
        f.last_error = None;
    }
    tracing::info!("xtokenhub: WS 已连接 {WS_URL}");

    loop {
        match socket.read() {
            Ok(tungstenite::Message::Text(t)) => {
                if let Ok(mut f) = feed.lock() {
                    f.apply_ws(t.as_str());
                }
            }
            Ok(tungstenite::Message::Binary(b)) => {
                if let Ok(text) = std::str::from_utf8(&b) {
                    if let Ok(mut f) = feed.lock() {
                        f.apply_ws(text);
                    }
                }
            }
            Ok(tungstenite::Message::Close(c)) => {
                return Err(format!("WS 被服务端关闭: {}", c.map(|f| f.to_string()).unwrap_or_default()))
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // 读超时：顺带看看 REST 是否需要刷新
                // （切到 Token 页的 request_refresh 标志也在这里消费，
                //   否则 WS 正常连接时标志永远没人处理）
                let due = last_http.elapsed().as_secs() >= HTTP_REFRESH_SECS
                    || (take_refresh_request() && last_http.elapsed().as_secs() >= 5);
                if due {
                    if let Ok(mut f) = feed.lock() {
                        f.refresh_http();
                    }
                    *last_http = Instant::now();
                }
            }
            Err(e) => return Err(format!("WS 读取失败: {e}")),
        }
    }
}

// ────────────────────────── 数值格式 ──────────────────────────

/// tokens 紧凑显示（中文单位：万 / 亿）
pub fn fmt_tokens(n: i64) -> String {
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

fn fmt_money(usd: f64, feed: &TokenFeed) -> String {
    if feed.currency().eq_ignore_ascii_case("CNY") {
        format!("¥{:.2}", usd * feed.rate())
    } else {
        format!("${usd:.2}")
    }
}

fn fmt_money_precise(usd: f64, feed: &TokenFeed) -> String {
    if feed.currency().eq_ignore_ascii_case("CNY") {
        format!("¥{:.4}", usd * feed.rate())
    } else {
        format!("${usd:.4}")
    }
}

fn mmdd(d: &str) -> String {
    d.get(5..10).unwrap_or(d).to_string()
}

/// 在 `max_w` 内选最大可用字号（BODY → LABEL → TINY），仍放不下才截断。
fn fit_text(c: &mut Canvas, s: &str, max_w: i32, weight: Weight) -> (String, f32) {
    for t in [Type::BODY, Type::LABEL, Type::TINY] {
        if c.fonts.text_width(s, t, weight) <= max_w {
            return (s.to_string(), t);
        }
    }
    (truncate(c, s, max_w, Type::TINY, weight), Type::TINY)
}

// ────────────────────────── 页面渲染 ──────────────────────────

pub fn render(c: &mut Canvas, o: &crate::collector::SystemOverview, f: &TokenFeed) {
    let (w, ht) = (c.lw, c.lh);
    c.clear(Palette::BG_BASE);

    let inner = w - 44;
    let header = Pane { x: 22, y: 22, w: inner, h: 96 };
    let footer = Pane { x: 22, y: ht - 78, w: inner, h: 56 };
    let body_y = header.y + header.h + 16;
    let body_h = footer.y - 16 - body_y;

    header_card(c, &header, o, f);
    footer_card(c, &footer, f);

    let usable = inner - 32;
    let c1w = (usable as f64 * 0.295).round() as i32;
    let c3w = (usable as f64 * 0.345).round() as i32;
    let c2w = usable - c1w - c3w;

    let col1 = split_col(22, body_y, c1w, body_h, &[50, 50]);
    card_today(c, &col1[0], f);
    card_throughput(c, &col1[1], f);

    let col2 = split_col(22 + c1w + 16, body_y, c2w, body_h, &[50, 50]);
    card_models(c, &col2[0], f);
    card_channels(c, &col2[1], f);

    let col3 = split_col(22 + c1w + 16 + c2w + 16, body_y, c3w, body_h, &[40, 33, 27]);
    card_quality(c, &col3[0], f);
    card_keys(c, &col3[1], f);
    card_gateway(c, &col3[2], f);
}

/// 只重绘顶栏时钟（每秒调用）
pub fn render_clock(c: &mut Canvas, o: &crate::collector::SystemOverview) {
    let header = Pane { x: 22, y: 22, w: c.lw - 44, h: 96 };
    clock(c, &header, o);
}

fn header_card(c: &mut Canvas, p: &Pane, o: &crate::collector::SystemOverview, f: &TokenFeed) {
    card(c, p);
    let x = p.x + 26;
    let ok = f.http_ok;
    c.dot(
        x + 9,
        p.y + 32,
        9,
        if ok && f.live.ws_connected {
            Palette::SUCCESS
        } else if ok {
            Palette::WARNING
        } else {
            Palette::ERROR
        },
    );
    c.text(x + 30, p.y + 46, "XTokenHub", Type::H1, Weight::Bold, Palette::FG_EMPHASIS);
    let tw = c.fonts.text_width("XTokenHub", Type::H1, Weight::Bold);
    c.text(
        x + 30 + tw + 18,
        p.y + 46,
        "LLM 网关 · 用量与费用",
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    let sub = format!(
        "累计 {} tokens  ·  峰值日 {} ({} )  ·  活跃 {} 天 / 连续 {} 天  ·  数据源 /api/v1 + WS",
        fmt_tokens(f.lifetime.total_tokens),
        fmt_tokens(f.lifetime.peak_day_tokens),
        if f.lifetime.peak_day.is_empty() { "-" } else { &f.lifetime.peak_day },
        f.lifetime.active_days,
        f.lifetime.current_streak
    );
    c.text(x, p.y + 82, &sub, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    super::layout::page_tabs(c, p.x + p.w - 900, p.y + 32, 1);
    clock(c, p, o);
}

fn clock(c: &mut Canvas, p: &Pane, _o: &crate::collector::SystemOverview) {
    let zone_w = 460;
    c.rect(p.x + p.w - zone_w - 20, p.y + 4, zone_w, p.h - 8, Palette::BG_SURFACE);
    let now = chrono::Local::now();
    let right = p.x + p.w - 26;
    c.text_right(
        right,
        p.y + 60,
        &now.format("%H:%M:%S").to_string(),
        Type::CLOCK,
        Weight::Bold,
        Palette::FG_EMPHASIS,
    );
    use chrono::Datelike;
    let wd = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
        [now.weekday().num_days_from_monday() as usize];
    c.text_right(
        right,
        p.y + 88,
        &format!("{} {}月{}日", wd, now.format("%m"), now.format("%d")),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

fn footer_card(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let baseline = p.y + p.h / 2 + 8;
    let x = p.x + 26;
    c.text(x, baseline, "音量 上/下", Type::LABEL, Weight::Bold, Palette::ACCENT);
    c.text(x + 130, baseline, "切换页面", Type::LABEL, Weight::Regular, Palette::FG_MUTED);

    let http = f
        .http_at
        .map(|t| format!("{}s 前·{}ms", t.elapsed().as_secs(), f.http_ms))
        .unwrap_or_else(|| "未加载".into());
    c.text(
        x + 290,
        baseline,
        &format!("REST 刷新 {http}"),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    let ws = if f.live.ws_connected { "WS 已连" } else { "WS 未连接" };
    c.text(
        x + 620,
        baseline,
        ws,
        Type::LABEL,
        Weight::Regular,
        if f.live.ws_connected { Palette::SUCCESS } else { Palette::WARNING },
    );
    c.text_right(
        p.x + p.w - 26,
        baseline,
        &format!("今日 {} · 近 7 天 {}", fmt_money(f.summary_today.cost_usd, f), fmt_money(f.summary_7d.cost_usd, f)),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

/// 今日用量（24h 汇总 + 费用预测）
fn card_today(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "今日用量（今日 00:00 起）");
    let s = &f.summary_today;
    let rate_ok = if s.total_requests > 0 {
        s.success_requests as f64 / s.total_requests as f64 * 100.0
    } else {
        100.0
    };
    c.text(x, y + 104, &format!("{}", s.total_requests), Type::VALUE_XL, Weight::Bold, Palette::ACCENT);
    let nw = c.fonts.text_width(&format!("{}", s.total_requests), Type::VALUE_XL, Weight::Bold);
    c.text(x + nw + 8, y + 104, "次请求", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    let (rtxt, rcol) = if rate_ok >= 98.0 {
        (format!("成功 {rate_ok:.1}%"), Palette::SUCCESS)
    } else if rate_ok >= 95.0 {
        (format!("成功 {rate_ok:.1}%"), Palette::WARNING)
    } else {
        (format!("成功 {rate_ok:.1}%"), Palette::ERROR)
    };
    c.pill(p.x + p.w - 210, y + 18, &rtxt, Type::TINY, rcol, Palette::BG_SURFACE_ALT);

    let rows: [(&str, String, Argb); 4] = [
        ("Tokens", fmt_tokens(s.total_tokens), Palette::FG_EMPHASIS),
        ("其中缓存命中", fmt_tokens(s.cached_tokens), Palette::INFO),
        ("费用", fmt_money(s.cost_usd, f), Palette::FG_EMPHASIS),
        ("缓存命中率", format!("{:.1}%", s.cache_hit_rate * 100.0), Palette::SUCCESS),
    ];
    let mut ry = y + 158;
    for (k, v, vc) in rows.iter() {
        c.text(x, ry, k, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, ry, v, Type::VALUE_M, Weight::Bold, *vc);
        ry += 42;
    }
    c.hline(x, y + 336, p.w - 44, 1, Palette::BORDER);
    let proj = match f.forecast.projected_usd {
        Some(v) => format!("预测今日 {}", fmt_money(v, f)),
        None => "预测今日 样本不足".to_string(),
    };
    c.text(x, y + 366, &proj, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.text_right(
        p.x + p.w - 22,
        y + 366,
        &format!("日均 {}", fmt_money(f.forecast.daily_avg_usd, f)),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

/// 实时吞吐（WS 2Hz 推送的 tokens/s 滑窗）
fn card_throughput(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "实时吞吐（WebSocket）");

    let vals: Vec<f32> = f.live.tps.iter().copied().collect();
    let cur = vals.last().copied().unwrap_or(0.0);
    c.text(x, y + 104, &format!("{cur:.0}"), Type::VALUE_XL, Weight::Bold, Palette::INFO);
    let nw = c.fonts.text_width(&format!("{cur:.0}"), Type::VALUE_XL, Weight::Bold);
    c.text(x + nw + 8, y + 104, "tokens/s", Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    c.pill(
        p.x + p.w - 210,
        y + 18,
        &format!("活跃流 {}", f.live.active_streams),
        Type::TINY,
        if f.live.active_streams > 0 { Palette::SUCCESS } else { Palette::FG_MUTED },
        Palette::BG_SURFACE_ALT,
    );

    let (sx, sw) = (x, p.w - 44);
    c.rect(sx, y + 140, sw, 110, Palette::BG_SURFACE_ALT);
    c.hline(sx + 1, y + 140 + 106, sw - 2, 1, Palette::BORDER);
    let busy = vals.iter().any(|v| *v > 0.0);
    if vals.len() >= 2 && busy {
        let max = vals.iter().cloned().fold(1.0f32, f32::max) * 1.15;
        c.sparkline(sx + 8, y + 146, sw - 16, 98, &vals, max, Palette::INFO);
    } else {
        // 空闲：画一条中轴参考线 + 明确文案，避免"空图表"观感
        let mid = y + 146 + 49;
        c.hline(sx + 8, mid, sw - 16, 1, Palette::BORDER);
        let msg = if vals.is_empty() {
            "等待 WS 推送 stats.throughput（2Hz）…"
        } else {
            "空闲 · 当前无流式输出"
        };
        let (wtxt, _) = fit_text(c, msg, sw - 32, Weight::Regular);
        c.text(sx + 16, mid + 8, &wtxt, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    }
    let avg = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f32>() / vals.len() as f32 };
    let peak = vals.iter().cloned().fold(0.0f32, f32::max);
    c.text(
        x,
        y + 286,
        &format!("滑窗均值 {avg:.0} · 峰值 {peak:.0} tokens/s · 采样 {}/{}", vals.len(), TPS_POINTS),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );

    // 最近一次完成的调用（来自 request.completed 事件）
    c.hline(x, y + 316, p.w - 44, 1, Palette::BORDER);
    c.text(x, y + 352, "最近一次调用", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    let last = if f.live.last_req.is_empty() {
        "等待事件 …".to_string()
    } else {
        f.live.last_req.clone()
    };
    let last_txt = truncate(c, &last, p.w - 44, Type::LABEL, Weight::Regular);
    c.text(x, y + 380, &last_txt, Type::LABEL, Weight::Regular, Palette::FG_DEFAULT);
}

/// 模型排行（近 7 天）
fn card_models(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "模型排行（近 7 天）");
    if f.by_model.is_empty() {
        c.text(x, y + 80, "无数据", Type::BODY, Weight::Regular, Palette::FG_MUTED);
        return;
    }
    let total: i64 = f.by_model.iter().map(|m| m.requests).sum();
    c.text_right(
        p.x + p.w - 22,
        y + 38,
        &format!("共 {} 次 · {}", total, fmt_tokens(f.by_model.iter().map(|m| m.total_tokens).sum())),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );

    let bar_w = 190.max(p.w - 44 - 470);
    // 行高 58，按卡片可用高度决定能放几行（避免最后一行溢出卡片）
    let step = 58;
    let fit = (((p.h - 96 - 24) / step).clamp(1, 8)) as usize;
    let mut ry = y + 96;
    for (i, m) in f.by_model.iter().take(fit).enumerate() {
        let share = if total > 0 { m.requests as f64 / total as f64 * 100.0 } else { 0.0 };
        let color = [Palette::ACCENT, Palette::SUCCESS, Palette::INFO, Palette::ACCENT_2, Palette::WARNING, Palette::ERROR]
            [i % 6];
        let (name, ntype) = fit_text(c, &m.name, 268, Weight::Bold);
        c.text(x, ry, &name, ntype, Weight::Bold, Palette::FG_EMPHASIS);
        c.text(x + 284, ry, &format!("{}", m.requests), Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        c.text(x + 396, ry, &fmt_tokens(m.total_tokens), Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, ry, &fmt_money(m.cost_usd, f), Type::BODY, Weight::Bold, color);
        c.bar(x, ry + 14, bar_w, 10, share, color);
        let pct = format!("{share:.0}%");
        c.text(x + bar_w + 12, ry + 24, &pct, Type::TINY, Weight::Regular, Palette::FG_MUTED);
        ry += step;
    }
}

/// 上游渠道（近 7 天）+ 余额
fn card_channels(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "上游渠道与余额");
    let enabled = f.channels.iter().filter(|c| c.status == 1).count();
    c.text_right(
        p.x + p.w - 22,
        y + 38,
        &format!("按用量 Top 5（条=平均延迟）· 配置 {} · 启用 {}", f.channels.len(), enabled),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );
    if f.by_channel.is_empty() {
        c.text(x, y + 80, "无数据", Type::BODY, Weight::Regular, Palette::FG_MUTED);
        return;
    }
    let max_reqs = f.by_channel.iter().map(|c| c.requests).max().unwrap_or(1).max(1);
    let step = 56;
    let fit = (((p.h - 96 - 24) / step).clamp(1, 8)) as usize;
    let mut ry = y + 96;
    for ch in f.by_channel.iter().take(fit) {
        // 余额（同名渠道）
        let bal = f
            .balances
            .iter()
            .find(|b| b.channel_name == ch.name)
            .and_then(|b| b.balance.as_ref());
        let bal_txt = match bal {
            Some(b) if b.total > 0.0 => {
                format!("余额 {}{:.2}", if b.currency == "CNY" { "¥" } else { "$" }, b.total)
            }
            _ => "余额 —".to_string(),
        };
        // 接口未提供按渠道错误数 → 用平均延迟作为健康信号
        let avg_s = ch.avg_ms / 1000.0;
        let err_color = if avg_s < 3.0 {
            Palette::SUCCESS
        } else if avg_s < 8.0 {
            Palette::WARNING
        } else {
            Palette::ERROR
        };
        let name = truncate(c, &ch.name, 200, Type::BODY, Weight::Regular);
        c.text(x, ry, &name, Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        c.text(x + 220, ry, &format!("{}", ch.requests), Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        c.text(x + 300, ry, &format!("{avg_s:.1}s"), Type::LABEL, Weight::Regular, err_color);
        c.text_right(p.x + p.w - 22, ry, &fmt_money(ch.cost_usd, f), Type::BODY, Weight::Bold, Palette::SUCCESS);
        let bal_color = if bal.is_some() { Palette::INFO } else { Palette::FG_MUTED };
        c.text(x + 420, ry, &bal_txt, Type::LABEL, Weight::Regular, bal_color);
        c.bar(
            x,
            ry + 14,
            190.max(p.w - 44 - 470),
            10,
            ch.requests as f64 / max_reqs as f64 * 100.0,
            err_color,
        );
        ry += step;
    }
}

/// 质量指标（近 7 天）
fn card_quality(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "质量指标（近 7 天）");
    let s = &f.summary_7d;
    let err_pct = if s.total_requests > 0 {
        s.error_requests as f64 / s.total_requests as f64 * 100.0
    } else {
        0.0
    };
    let err_color = if err_pct < 3.0 {
        Palette::SUCCESS
    } else if err_pct < 10.0 {
        Palette::WARNING
    } else {
        Palette::ERROR
    };
    let cache_color = if s.cache_hit_rate >= 0.8 {
        Palette::SUCCESS
    } else if s.cache_hit_rate >= 0.5 {
        Palette::WARNING
    } else {
        Palette::ERROR
    };
    let lat_color = if s.avg_duration_ms < 3000.0 {
        Palette::SUCCESS
    } else if s.avg_duration_ms < 8000.0 {
        Palette::WARNING
    } else {
        Palette::ERROR
    };
    let cells: [(&str, String, Argb); 3] = [
        ("错误率", format!("{err_pct:.1}%"), err_color),
        ("平均延迟", format!("{:.1}s", s.avg_duration_ms / 1000.0), lat_color),
        ("缓存命中", format!("{:.0}%", s.cache_hit_rate * 100.0), cache_color),
    ];
    let cw = (p.w - 44) / 3;
    for (i, (k, v, vc)) in cells.iter().enumerate() {
        let kx = x + i as i32 * cw;
        c.text(kx, y + 92, k, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text(kx, y + 146, v, Type::VALUE_L, Weight::Bold, *vc);
    }
    c.bar(x, y + 172, p.w - 44, 12, err_pct.min(100.0), err_color);
    c.text(
        x,
        y + 212,
        &format!("{} 次请求 / {} 次错误 · 均延迟 {:.1}s", s.total_requests, s.error_requests, s.avg_duration_ms / 1000.0),
        Type::LABEL,
        Weight::Regular,
        Palette::FG_MUTED,
    );

    // 近 7 天请求趋势（来自 /stats/trend，补满卡片下部）
    c.text(x, y + 244, "近 7 天请求数", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    let reqs: Vec<f32> = f.trend.iter().map(|t| t.requests as f32).collect();
    if reqs.len() >= 2 {
        let max = reqs.iter().cloned().fold(1.0f32, f32::max) * 1.2;
        c.sparkline(x, y + 254, p.w - 44, 46, &reqs, max, Palette::ACCENT);
    } else {
        c.text(x, y + 268, "趋势数据不足", Type::TINY, Weight::Regular, Palette::FG_MUTED);
    }
    c.text(
        x,
        y + 320,
        &format!(
            "输入 {} · 输出 {} · 缓存命中 {}",
            fmt_tokens(s.prompt_tokens),
            fmt_tokens(s.completion_tokens),
            fmt_tokens(s.cached_tokens)
        ),
        Type::TINY,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}

/// 调用方（API Key，近 7 天）
fn card_keys(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "调用方（API Key，近 7 天）");
    if f.by_key.is_empty() {
        c.text(x, y + 80, "无数据", Type::BODY, Weight::Regular, Palette::FG_MUTED);
        return;
    }
    let total: i64 = f.by_key.iter().map(|k| k.requests).sum();
    let mut ry = y + 96;
    for k in f.by_key.iter().take(5) {
        let share = if total > 0 { k.requests as f64 / total as f64 * 100.0 } else { 0.0 };
        let name = truncate(c, &k.name, 240, Type::BODY, Weight::Regular);
        c.text(x, ry, &name, Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        c.text(x + 250, ry, &format!("{} 次", k.requests), Type::BODY, Weight::Regular, Palette::FG_DEFAULT);
        let share_txt = format!("{share:.0}%");
        c.text(x + 400, ry, &share_txt, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, ry, &fmt_money(k.cost_usd, f), Type::BODY, Weight::Bold, Palette::FG_EMPHASIS);
        ry += 34;
    }
}

/// 网关状态 / 累计（lifetime）
fn card_gateway(c: &mut Canvas, p: &Pane, f: &TokenFeed) {
    card(c, p);
    let (x, y) = (p.x + 22, p.y);
    title(c, p, "网关与累计");
    let (label, color) = if f.http_ok && f.live.ws_connected {
        ("正常", Palette::SUCCESS)
    } else if f.http_ok {
        ("WS 断开", Palette::WARNING)
    } else {
        ("REST 异常", Palette::ERROR)
    };
    c.pill(p.x + p.w - 160, y + 18, label, Type::TINY, color, Palette::BG_SURFACE_ALT);

    let l = &f.lifetime;
    let rows: [(&str, String, Argb); 3] = [
        ("累计 tokens", fmt_tokens(l.total_tokens), Palette::FG_EMPHASIS),
        ("峰值日", format!("{} ({})", fmt_tokens(l.peak_day_tokens), mmdd(&l.peak_day)), Palette::FG_DEFAULT),
        ("连续 / 最长", format!("{} / {} 天", l.current_streak, l.max_streak), Palette::FG_DEFAULT),
    ];
    let mut ry = y + 78;
    for (k, v, vc) in rows.iter() {
        c.text(x, ry, k, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
        c.text_right(p.x + p.w - 22, ry, v, Type::BODY, Weight::Bold, *vc);
        ry += 30;
    }
    if let Some(e) = &f.last_error {
        let e = truncate(c, e, p.w - 44, Type::TINY, Weight::Regular);
        c.text(x, y + 162, &e, Type::TINY, Weight::Regular, Palette::ERROR);
    }
    // 单价提示（按币种）
    let budget = if f.billing.monthly_budget_usd > 0.0 {
        format!("月预算 {}", fmt_money_precise(f.billing.monthly_budget_usd, f))
    } else {
        "未设月预算".to_string()
    };
    c.text(
        x,
        y + 210,
        &format!("币种 {} · 1$={:.1} · {budget}", f.currency(), f.rate()),
        Type::TINY,
        Weight::Regular,
        Palette::FG_MUTED,
    );
}
