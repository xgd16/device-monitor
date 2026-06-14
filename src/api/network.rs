//! 网络信息 API
//!
//! - `GET /api/network` — 网卡列表
//! - `GET /api/network/wifi` — WiFi 连接详情
//! - `GET /api/network/bluetooth` — 蓝牙适配器信息

use axum::Json;
use serde_json::Value;
use crate::collector;
use super::success;

pub async fn network_info() -> Json<Value> {
    let data = collector::network::collect_interfaces();
    success(data)
}

pub async fn wifi_info() -> Json<Value> {
    let data = collector::network::get_wifi_info();
    success(data)
}

pub async fn bluetooth_info() -> Json<Value> {
    let data = collector::network::get_bluetooth_info();
    success(data)
}
