//! HTTP REST API 模块
//!
//! 各子模块对应一类资源的 GET/POST 处理器，统一返回 `{ code, data }` 或 `{ code, error }` 格式。

pub mod system;
pub mod cpu;
pub mod memory;
pub mod disk;
pub mod thermal;
pub mod battery;
pub mod network;
pub mod process;
pub mod logs;
pub mod alerts;
pub mod hardware;
pub mod database;
pub mod history;
pub mod files;
pub mod archive;

use axum::Json;
use serde_json::{json, Value};

/// 成功响应包装：`{ "code": 0, "data": ... }`
pub fn success<T: serde::Serialize>(data: T) -> Json<Value> {
    Json(json!({ "code": 0, "data": data }))
}

/// 错误响应包装：`{ "code": -1, "error": "..." }`
pub fn error(msg: &str) -> Json<Value> {
    Json(json!({ "code": -1, "error": msg }))
}
