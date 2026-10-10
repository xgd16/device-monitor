//! NFC 操作 API
//!
//! 通过 `crate::nfc::service()` 的常驻 worker 访问 NFC 硬件：
//! 轮询发现卡片、读取/写入 NDEF、原始帧收发、NFC-DEP 监听；
//! 卡模拟（自定义 UID + T4T NDEF 卡）走 `crate::nfc::emulate` 用户态直驱芯片。
//!
//! 互斥：卡模拟期间内核 `nfc0` 下线，读写/轮询等接口返回明确错误；
//! 启动模拟前会先停轮询并 `DEV_DOWN`，停止后自动重新绑定驱动并上电。

use axum::Json;
use axum::extract::Query;
use serde::Deserialize;
use serde_json::Value;

use crate::nfc::{self, RawReq, ReadReq, WriteReq, emulate};

use super::{error, success};

/// 卡模拟运行中时，内核侧操作的统一拒绝
fn emulate_running_error() -> Json<Value> {
    error("卡模拟进行中：内核 NFC 已临时下线，请先停止模拟")
}

/// `GET /api/nfc/status` — NFC 状态与能力
pub async fn status() -> Json<Value> {
    success(nfc::service().status())
}

/// `GET /api/nfc/tags` — 已发现卡片列表
pub async fn tags() -> Json<Value> {
    success(nfc::service().tags())
}

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

/// `GET /api/nfc/events?limit=50` — 最近事件
pub async fn events(Query(q): Query<EventsQuery>) -> Json<Value> {
    success(nfc::service().events(q.limit.clamp(1, 200)))
}

#[derive(Deserialize)]
pub struct PowerParams {
    pub up: bool,
}

/// `POST /api/nfc/power` — 控制器上电/下电
pub async fn power(Json(p): Json<PowerParams>) -> Json<Value> {
    if emulate::service().status().running {
        return emulate_running_error();
    }
    match nfc::service().set_power(p.up).await {
        Ok(()) => success(serde_json::json!({ "powered": p.up })),
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct PollParams {
    /// "start" | "stop"
    pub action: String,
    #[serde(default)]
    pub protocols: Vec<String>,
}

/// `POST /api/nfc/poll` — 开始/停止轮询（读卡）
pub async fn poll(Json(p): Json<PollParams>) -> Json<Value> {
    if emulate::service().status().running {
        return emulate_running_error();
    }
    let svc = nfc::service();
    match p.action.as_str() {
        "start" => match svc.start_poll(p.protocols).await {
            Ok(list) => success(serde_json::json!({ "polling": true, "protocols": list })),
            Err(e) => error(&e),
        },
        "stop" => match svc.stop_poll().await {
            Ok(()) => success(serde_json::json!({ "polling": false, "protocols": [] })),
            Err(e) => error(&e),
        },
        other => error(&format!("未知 action：{other}（应为 start/stop）")),
    }
}

#[derive(Deserialize)]
pub struct ListenParams {
    /// "start" | "stop"
    pub action: String,
}

/// `POST /api/nfc/listen` — 开启/关闭 NFC-DEP 监听（点对点目标模式）
pub async fn listen(Json(p): Json<ListenParams>) -> Json<Value> {
    if emulate::service().status().running {
        return emulate_running_error();
    }
    let svc = nfc::service();
    match p.action.as_str() {
        "start" => match svc.start_listen().await {
            Ok(()) => success(serde_json::json!({ "listening": true })),
            Err(e) => error(&e),
        },
        "stop" => match svc.stop_listen().await {
            Ok(()) => success(serde_json::json!({ "listening": false })),
            Err(e) => error(&e),
        },
        other => error(&format!("未知 action：{other}（应为 start/stop）")),
    }
}

#[derive(Deserialize)]
pub struct ReadParams {
    pub target_index: u32,
    #[serde(default)]
    pub protocol: String,
    /// "ndef" | "pages"
    #[serde(default = "default_mode")]
    pub mode: String,
    pub start_page: Option<u8>,
    pub page_count: Option<u8>,
}

fn default_mode() -> String {
    "ndef".to_string()
}

/// `POST /api/nfc/read` — 读取卡片（NDEF 或原始页）
pub async fn read(Json(p): Json<ReadParams>) -> Json<Value> {
    if emulate::service().status().running {
        return emulate_running_error();
    }
    let req = ReadReq {
        target_index: p.target_index,
        protocol: p.protocol,
        mode: p.mode,
        start_page: p.start_page,
        page_count: p.page_count,
    };
    match nfc::service().read(req).await {
        Ok(r) => success(r),
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct WriteParams {
    pub target_index: u32,
    #[serde(default)]
    pub protocol: String,
    /// "text" | "uri" | "raw"
    pub kind: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub raw_hex: String,
}

/// `POST /api/nfc/write` — 写入卡片 NDEF
pub async fn write(Json(p): Json<WriteParams>) -> Json<Value> {
    if emulate::service().status().running {
        return emulate_running_error();
    }
    let req = WriteReq {
        target_index: p.target_index,
        protocol: p.protocol,
        kind: p.kind,
        value: p.value,
        lang: p.lang,
        raw_hex: p.raw_hex,
    };
    match nfc::service().write(req).await {
        Ok(r) => success(r),
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct RawParams {
    pub target_index: u32,
    #[serde(default)]
    pub protocol: String,
    pub tx_hex: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    800
}

/// `POST /api/nfc/raw` — 原始帧收发（高级调试）
pub async fn raw(Json(p): Json<RawParams>) -> Json<Value> {
    if emulate::service().status().running {
        return emulate_running_error();
    }
    let req = RawReq {
        target_index: p.target_index,
        protocol: p.protocol,
        tx_hex: p.tx_hex,
        timeout_ms: p.timeout_ms,
    };
    match nfc::service().raw(req).await {
        Ok(r) => success(r),
        // 参数类错误（hex 非法等）也走统一错误结构
        Err(e) => error(&e),
    }
}

#[derive(Deserialize)]
pub struct EmulateParams {
    /// "start" | "stop"
    pub action: String,
    /// UID：4/7/10 字节 hex，可带冒号
    #[serde(default)]
    pub uid: String,
    /// "text" | "uri" | "raw" | "message"（message = 整条 NDEF 消息 hex 逐字节克隆）
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub raw_hex: String,
}

/// `GET /api/nfc/emulate` — 卡模拟状态与事件
pub async fn emulate_status() -> Json<Value> {
    let svc = emulate::service();
    let mut v = serde_json::to_value(svc.status()).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut v {
        map.insert("events".into(), serde_json::to_value(svc.events(20)).unwrap_or(Value::Null));
    }
    success(v)
}

/// `POST /api/nfc/emulate` — 开始/停止 UID 卡模拟（T4T NDEF 卡）
pub async fn emulate(Json(p): Json<EmulateParams>) -> Json<Value> {
    let svc = emulate::service();
    match p.action.as_str() {
        "start" => {
            if svc.status().running {
                return error("卡模拟已在运行，请先停止");
            }
            // 解绑内核驱动前先停轮询并断电，避免 remove 与在途 I2C 撞车
            let nfc = nfc::service();
            let _ = nfc.stop_poll().await;
            let _ = nfc.set_power(false).await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let req = emulate::EmulReq {
                uid: p.uid,
                kind: if p.kind.is_empty() { "text".into() } else { p.kind },
                value: p.value,
                lang: p.lang,
                raw_hex: p.raw_hex,
            };
            match svc.start(&req) {
                Ok(st) => success(st),
                Err(e) => error(&e),
            }
        }
        "stop" => {
            if let Err(e) = svc.stop() {
                return error(&e);
            }
            // 等模拟线程收尾（重新 bind 内核驱动）后恢复上电
            for _ in 0..50 {
                if !svc.status().running {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let stopped = !svc.status().running;
            let mut out = serde_json::to_value(svc.status()).unwrap_or(Value::Null);
            if stopped {
                // 重新 bind 后 nfc0 注册需要一点时间，恢复上电做几次重试
                for i in 0..5 {
                    if nfc::service().set_power(true).await.is_ok() {
                        break;
                    }
                    if i < 4 {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                }
            } else if let Value::Object(map) = &mut out {
                map.insert("note".into(), Value::String("已请求停止，后台仍在清理（稍后刷新）".into()));
            }
            success(out)
        }
        other => error(&format!("未知 action：{other}（应为 start/stop）")),
    }
}
