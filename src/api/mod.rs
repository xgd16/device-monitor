//! HTTP REST API 模块
//!
//! 各子模块对应一类资源的 GET/POST 处理器，统一返回 `{ code, data }` 或 `{ code, error }` 格式。

pub mod alerts;
pub mod archive;
pub mod battery;
pub mod cpu;
pub mod database;
pub mod disk;
pub mod files;
pub mod hardware;
pub mod history;
pub mod logs;
pub mod memory;
pub mod mihomo;
pub mod network;
pub mod nfc;
pub mod process;
pub mod system;
pub mod thermal;

use axum::Json;
use serde_json::{Value, json};

/// 成功响应包装：`{ "code": 0, "data": ... }`
pub fn success<T: serde::Serialize>(data: T) -> Json<Value> {
    Json(json!({ "code": 0, "data": data }))
}

/// 错误响应包装：`{ "code": -1, "error": "..." }`
pub fn error(msg: &str) -> Json<Value> {
    Json(json!({ "code": -1, "error": msg }))
}
