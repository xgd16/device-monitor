//! 网络信息 API
//!
//! - `GET /api/network` — 网卡列表
//! - `GET /api/network/wifi` — WiFi 连接详情
//! - `GET /api/network/wifi/scan` — 主动扫描周边 WiFi（面板 WiFi 操作页按需调用）
//! - `POST /api/network/wifi/connect` — 连接 WiFi（密码留空时优先用已保存配置）
//! - `GET /api/network/bluetooth` — 蓝牙适配器信息

use super::{error, success};
use crate::collector;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

pub async fn network_info() -> Json<Value> {
    let data = collector::network::collect_interfaces();
    success(data)
}

pub async fn wifi_info() -> Json<Value> {
    let data = collector::network::get_wifi_info();
    success(data)
}

/// `GET /api/network/wifi/scan` — 主动扫描周边 WiFi。
///
/// 会触发一次真实射频扫描（实测约 3s），因此只在面板打开 WiFi 操作页时调用；
/// nmcli 是阻塞命令，放到 blocking 线程池，避免占住 tokio 工作线程。
pub async fn wifi_scan() -> Json<Value> {
    match tokio::task::spawn_blocking(collector::network::scan_wifi_networks).await {
        Ok(Ok(list)) => success(list),
        Ok(Err(e)) => error(&e),
        Err(e) => error(&format!("扫描任务异常: {e}")),
    }
}

#[derive(Deserialize)]
pub struct WifiConnectParams {
    pub ssid: String,
    /// WPA/WPA2/WPA3 密码；留空表示尝试已保存配置或开放网络
    #[serde(default)]
    pub password: String,
}

/// `POST /api/network/wifi/connect` — 连接指定 SSID。
///
/// `nmcli` 激活可能等待 DHCP 数十秒，同样放 blocking 线程；面板侧 HTTP 超时按 60s 设置。
pub async fn wifi_connect(Json(p): Json<WifiConnectParams>) -> Json<Value> {
    let ssid = p.ssid;
    let call_ssid = ssid.clone();
    let password = p.password;
    let result = tokio::task::spawn_blocking(move || collector::network::connect_wifi(&call_ssid, &password)).await;
    match result {
        Ok(Ok(msg)) => success(serde_json::json!({ "message": msg, "ssid": ssid })),
        Ok(Err(e)) => error(&e),
        Err(e) => error(&format!("连接任务异常: {e}")),
    }
}

pub async fn bluetooth_info() -> Json<Value> {
    let data = collector::network::get_bluetooth_info();
    success(data)
}
