//! 内存信息 API
//!
//! `GET /api/memory` — 返回物理内存与 Swap 使用情况。

use axum::Json;
use serde_json::Value;
use crate::collector;
use super::success;

pub async fn memory_info() -> Json<Value> {
    let data = collector::memory::collect();
    success(data)
}
