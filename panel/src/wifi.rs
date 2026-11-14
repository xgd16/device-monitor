//! WiFi 操作页（设置页 → WiFi）的桥接层：扫描、连接、密码键盘状态。
//!
//! 线程模型：
//!   · UI 回调只翻转状态并启动工作线程，绝不在 UI 线程里跑 nmcli；
//!   · 扫描（约 3s）与连接（最长 ~45s DHCP 等待）各自独立线程，结果经
//!     `upgrade_in_event_loop` 回到 UI；
//!   · 扫描代次号（SCAN_GEN）保证「关页 / 重扫」后在途结果直接作废——这也
//!     是「只有打开 WiFi 页面才扫描」的兜底，不会出现关页后列表又被写回。
//!
//! 密码键盘同样只用 ASCII 可见字符（WPA 密码规范），由 Slint 侧按键回调把
//! 字符传进来，这里维护明文 / 掩码两套显示串。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};

use crate::{App, BASE, Wifi, WifiRow, apply_wifi, fetch_json_timeout, tracing_or_log};

/// 在途扫描代次：关页/重扫时自增，旧线程的结果直接丢弃。
static SCAN_GEN: AtomicU64 = AtomicU64::new(0);

const SCAN_TIMEOUT: Duration = Duration::from_secs(35);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(65);

/// 注册 WiFi 页面所有 UI 回调。
pub fn init(app: &App, weak: Weak<App>, demo: bool) {
    {
        let w = weak.clone();
        app.global::<Wifi>().on_open_wifi(move || open(&w, demo));
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_close_wifi(move || close(&w));
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_rescan(move || rescan(&w, demo));
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_pick(move |ssid, security, saved, in_use| {
            pick(&w, ssid, security, saved, in_use, demo);
        });
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_key(move |ch: SharedString| {
            let Some(ui) = w.upgrade() else { return };
            let g = ui.global::<Wifi>();
            if g.get_connecting() {
                return;
            }
            // 只接受 ASCII 可见字符（WPA 密码规范；也避免中文输入串进密码框）
            if ch.is_empty() || !ch.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                return;
            }
            let mut pass = g.get_password().to_string();
            if pass.chars().count() >= 63 {
                return;
            }
            pass.push_str(&ch);
            apply_password(&ui, &pass);
        });
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_backspace(move || {
            let Some(ui) = w.upgrade() else { return };
            let g = ui.global::<Wifi>();
            if g.get_connecting() {
                return;
            }
            let mut chars: Vec<char> = g.get_password().chars().collect();
            chars.pop();
            let pass: String = chars.into_iter().collect();
            apply_password(&ui, &pass);
        });
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_clear_pass(move || {
            let Some(ui) = w.upgrade() else { return };
            if ui.global::<Wifi>().get_connecting() {
                return;
            }
            apply_password(&ui, "");
        });
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_toggle_show(move || {
            let Some(ui) = w.upgrade() else { return };
            let g = ui.global::<Wifi>();
            let show = !g.get_pass_show();
            g.set_pass_show(show);
            let pass = g.get_password().to_string();
            g.set_pass_display(password_display(&pass, show).into());
        });
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_submit(move || submit(&w, demo));
    }
    {
        let w = weak.clone();
        app.global::<Wifi>().on_cancel_pass(move || {
            let Some(ui) = w.upgrade() else { return };
            let g = ui.global::<Wifi>();
            if g.get_connecting() {
                return;
            }
            g.set_pass_view(false);
            g.set_error("".into());
            apply_password(&ui, "");
        });
    }
}

/// 密码串 → 明文（显示）或等长圆点（掩码）。
fn password_display(pass: &str, show: bool) -> String {
    if show { pass.to_string() } else { "•".repeat(pass.chars().count()) }
}

fn apply_password(ui: &App, pass: &str) {
    let g = ui.global::<Wifi>();
    g.set_password(pass.into());
    let show = g.get_pass_show();
    g.set_pass_display(password_display(pass, show).into());
}

/// 打开页面：只在此时触发一次扫描（设备上 nmcli --rescan yes，约 3s）。
fn open(w: &Weak<App>, demo: bool) {
    let Some(ui) = w.upgrade() else { return };
    let g = ui.global::<Wifi>();
    g.set_visible(true);
    g.set_pass_view(false);
    g.set_error("".into());
    g.set_kb_layer(0);
    g.set_kb_shift(false);
    apply_password(&ui, "");

    if demo {
        g.set_scanning(false);
        g.set_aps(demo_rows());
        g.set_status("演示数据 · 6 个网络".into());
        g.set_current_connected(true);
        g.set_current_ssid("Home-5G".into());
        g.set_current_detail("5G · 信道44 · 866.7 Mbit/s".into());
        return;
    }

    g.set_scanning(true);
    g.set_status("正在扫描…".into());
    // 当前连接（/api/network/wifi 只是读 iw/nmcli 状态，不触发射频扫描）
    let w2 = w.clone();
    std::thread::Builder::new().name("wifi-status".into()).spawn(move || refresh_current(&w2)).ok();
    start_scan(w);
}

/// 关闭页面：代次自增让在途扫描作废；连接进行中不允许关闭（避免误以为已取消）。
fn close(w: &Weak<App>) {
    SCAN_GEN.fetch_add(1, Ordering::SeqCst);
    let Some(ui) = w.upgrade() else { return };
    let g = ui.global::<Wifi>();
    if g.get_connecting() {
        return;
    }
    g.set_visible(false);
    g.set_pass_view(false);
    g.set_scanning(false);
    g.set_error("".into());
    apply_password(&ui, "");
}

fn rescan(w: &Weak<App>, demo: bool) {
    let Some(ui) = w.upgrade() else { return };
    let g = ui.global::<Wifi>();
    if g.get_scanning() || g.get_connecting() {
        return;
    }
    g.set_error("".into());
    if demo {
        g.set_status("演示数据 · 6 个网络".into());
        return;
    }
    g.set_scanning(true);
    g.set_status("正在扫描…".into());
    start_scan(w);
}

/// 启动一次扫描；结果回 UI 前再校验代次，关页/重扫后的旧结果直接丢。
fn start_scan(w: &Weak<App>) {
    let scan_gen = SCAN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let w = w.clone();
    std::thread::Builder::new()
        .name("wifi-scan".into())
        .spawn(move || {
            let result = fetch_json_timeout(&format!("{BASE}/network/wifi/scan"), SCAN_TIMEOUT);
            if SCAN_GEN.load(Ordering::SeqCst) != scan_gen {
                return;
            }
            match result {
                Ok(v) => {
                    let rows = parse_rows(&v);
                    let n = rows.len();
                    w.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<Wifi>();
                        g.set_aps(ModelRc::new(VecModel::from(rows)));
                        g.set_scanning(false);
                        g.set_status(format!("找到 {n} 个网络").into());
                        g.set_error("".into());
                    })
                    .ok();
                }
                Err(e) => {
                    tracing_or_log(&format!("WiFi 扫描失败：{e}"));
                    w.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<Wifi>();
                        g.set_scanning(false);
                        g.set_error(e.into());
                    })
                    .ok();
                }
            }
        })
        .ok();
}

/// 解析 `[{ssid,...}]` → Slint 模型。
fn parse_rows(v: &Value) -> Vec<WifiRow> {
    let Some(arr) = v.as_array() else { return Vec::new() };
    arr.iter()
        .map(|n| WifiRow {
            ssid: str_of(n, "ssid").into(),
            security: str_of(n, "security").into(),
            secure: bool_of(n, "secure"),
            signal: int_of(n, "signal"),
            band: str_of(n, "band").into(),
            channel: int_of(n, "channel"),
            in_use: bool_of(n, "in_use"),
            saved: bool_of(n, "saved"),
        })
        .collect()
}

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_string()
}
fn int_of(v: &Value, k: &str) -> i32 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}
fn bool_of(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// 刷新当前连接摘要（页面头部 + 底栏 WiFi 状态）。
fn refresh_current(w: &Weak<App>) {
    let Ok(v) = fetch_json_timeout(&format!("{BASE}/network/wifi"), Duration::from_secs(6)) else {
        return;
    };
    let connected = bool_of(&v, "connected");
    let ssid = str_of(&v, "ssid");
    let band = str_of(&v, "band");
    let channel = int_of(&v, "channel");
    let bitrate = str_of(&v, "bitrate");
    let detail = if connected { format!("{band} · 信道{channel} · {bitrate}") } else { String::new() };
    apply_wifi(w, &v);
    w.upgrade_in_event_loop(move |ui| {
        let g = ui.global::<Wifi>();
        g.set_current_connected(connected);
        g.set_current_ssid(ssid.into());
        g.set_current_detail(detail.into());
    })
    .ok();
}

/// 点选一个网络：已连接仅提示；开放网络直接连；加密网络切到密码视图。
fn pick(w: &Weak<App>, ssid: SharedString, security: SharedString, saved: bool, in_use: bool, demo: bool) {
    let Some(ui) = w.upgrade() else { return };
    let g = ui.global::<Wifi>();
    if g.get_connecting() {
        return;
    }
    if in_use {
        g.set_status(format!("{ssid} 已连接").into());
        return;
    }
    g.set_error("".into());
    let secure = !security.trim().is_empty();
    if !secure {
        start_connect(w, ssid.to_string(), String::new(), demo);
        return;
    }
    g.set_pass_view(true);
    g.set_sel_ssid(ssid);
    g.set_sel_security(security);
    g.set_sel_saved(saved);
    g.set_pass_show(false);
    g.set_kb_layer(0);
    g.set_kb_shift(false);
    apply_password(&ui, "");
}

fn submit(w: &Weak<App>, demo: bool) {
    let Some(ui) = w.upgrade() else { return };
    let g = ui.global::<Wifi>();
    if g.get_connecting() {
        return;
    }
    let ssid = g.get_sel_ssid().to_string();
    if ssid.is_empty() {
        return;
    }
    let pass = g.get_password().to_string();
    if pass.is_empty() && !g.get_sel_saved() {
        g.set_error("请输入密码；已保存过密码的网络可留空直接连接".into());
        return;
    }
    g.set_error("".into());
    start_connect(w, ssid, pass, demo);
}

/// 连接线程：nmcli 最多等 45s（DHCP），HTTP 超时 65s。
fn start_connect(w: &Weak<App>, ssid: String, password: String, demo: bool) {
    let Some(ui) = w.upgrade() else { return };
    let g = ui.global::<Wifi>();
    g.set_connecting(true);
    g.set_error("".into());
    g.set_status(format!("正在连接 {ssid}…").into());

    if demo {
        let w = w.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            w.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<Wifi>();
                g.set_connecting(false);
                g.set_pass_view(false);
                g.set_status(format!("已连接 {ssid}（演示）").into());
                apply_password(&ui, "");
            })
            .ok();
        });
        return;
    }

    let w = w.clone();
    std::thread::Builder::new()
        .name("wifi-connect".into())
        .spawn(move || {
            let body = serde_json::json!({ "ssid": ssid, "password": password });
            match post_json(&format!("{BASE}/network/wifi/connect"), &body, CONNECT_TIMEOUT) {
                Ok(v) => {
                    let msg = str_of(&v, "message");
                    let msg = if msg.is_empty() { format!("已连接到 {ssid}") } else { msg };
                    tracing_or_log(&format!("WiFi 连接成功：{msg}"));
                    let done = msg.clone();
                    w.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<Wifi>();
                        g.set_connecting(false);
                        g.set_pass_view(false);
                        g.set_status(done.into());
                        apply_password(&ui, "");
                    })
                    .ok();
                    // 同步底栏状态 + 列表里的「已连接」标记（不再触发一次射频扫描）
                    refresh_current(&w);
                    mark_connected(&w, &ssid);
                }
                Err(e) => {
                    tracing_or_log(&format!("WiFi 连接失败：{e}"));
                    w.upgrade_in_event_loop(move |ui| {
                        let g = ui.global::<Wifi>();
                        g.set_connecting(false);
                        g.set_error(e.into());
                    })
                    .ok();
                }
            }
        })
        .ok();
}

/// 连接成功后只改模型里的 in-use 标记，避免为了刷新状态再扫一次。
fn mark_connected(w: &Weak<App>, ssid: &str) {
    let ssid = ssid.to_string();
    w.upgrade_in_event_loop(move |ui| {
        let g = ui.global::<Wifi>();
        let rows: Vec<WifiRow> = g
            .get_aps()
            .iter()
            .map(|mut row| {
                row.in_use = row.ssid.as_str() == ssid.as_str();
                row
            })
            .collect();
        g.set_aps(ModelRc::new(VecModel::from(rows)));
    })
    .ok();
}

/// 与主服务一致的 `{code,data}` 信封；错误直接把服务端文本带回 UI。
fn post_json(url: &str, body: &Value, timeout: Duration) -> Result<Value, String> {
    let resp = ureq::post(url)
        .timeout(timeout)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let text = resp.into_string().map_err(|e| format!("读取 {url} 失败: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("解析 {url} 失败: {e}"))?;
    if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
        let msg = v.get("error").and_then(|e| e.as_str()).unwrap_or("未知错误");
        return Err(msg.to_string());
    }
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

fn demo_rows() -> ModelRc<WifiRow> {
    let rows = vec![
        WifiRow {
            ssid: "Home-5G".into(),
            security: "WPA2 WPA3".into(),
            secure: true,
            signal: 100,
            band: "5G".into(),
            channel: 44,
            in_use: true,
            saved: true,
        },
        WifiRow {
            ssid: "XM_MX".into(),
            security: "WPA2".into(),
            secure: true,
            signal: 72,
            band: "5G".into(),
            channel: 44,
            in_use: false,
            saved: false,
        },
        WifiRow {
            ssid: "Xiaomi_1343".into(),
            security: "WPA2".into(),
            secure: true,
            signal: 54,
            band: "2.4G".into(),
            channel: 6,
            in_use: false,
            saved: false,
        },
        WifiRow {
            ssid: "TP-LINK_C147".into(),
            security: "WPA1 WPA2".into(),
            secure: true,
            signal: 38,
            band: "2.4G".into(),
            channel: 11,
            in_use: false,
            saved: false,
        },
        WifiRow {
            ssid: "QHY".into(),
            security: "".into(),
            secure: false,
            signal: 30,
            band: "2.4G".into(),
            channel: 6,
            in_use: false,
            saved: false,
        },
        WifiRow {
            ssid: "guest".into(),
            security: "WPA2".into(),
            secure: true,
            signal: 18,
            band: "2.4G".into(),
            channel: 1,
            in_use: false,
            saved: true,
        },
    ];
    ModelRc::new(VecModel::from(rows))
}
