//! 历史指标 API
//!
//! `GET /api/history/metrics` — 查询 SQLite 中持久化的指标时间序列，供报表图表使用。

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use crate::collector::SystemOverview;
use crate::AppState;
use super::{success, error};

#[derive(Deserialize)]
pub struct HistoryQuery {
    /// 时间范围：1h / 6h / 24h / 7d
    pub range: Option<String>,
    /// 最大返回点数（默认 500，上限 2000）
    pub max_points: Option<usize>,
}

fn range_to_secs(range: &str) -> i64 {
    match range {
        "6h" => 6 * 3600,
        "24h" => 24 * 3600,
        "7d" => 7 * 24 * 3600,
        _ => 3600, // 默认 1h
    }
}

pub async fn metrics_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Json<serde_json::Value> {
    let range = q.range.as_deref().unwrap_or("1h");
    let max_points = q.max_points.unwrap_or(500).clamp(50, 2000);
    let now = chrono::Utc::now().timestamp();
    let from_ts = now - range_to_secs(range);

    let rows = match state.db.get_metrics_in_range(from_ts, now, max_points) {
        Ok(r) => r,
        Err(e) => return error(&format!("查询历史数据失败: {}", e)),
    };

    let mut timestamps: Vec<i64> = Vec::with_capacity(rows.len());
    let mut cpu_usage: Vec<f64> = Vec::with_capacity(rows.len());
    let mut memory_percent: Vec<f64> = Vec::with_capacity(rows.len());
    let mut memory_used_mb: Vec<u64> = Vec::with_capacity(rows.len());
    let mut load_1: Vec<f64> = Vec::with_capacity(rows.len());
    let mut load_5: Vec<f64> = Vec::with_capacity(rows.len());
    let mut load_15: Vec<f64> = Vec::with_capacity(rows.len());
    let mut battery_capacity: Vec<u8> = Vec::with_capacity(rows.len());
    let mut battery_power_w: Vec<f64> = Vec::with_capacity(rows.len());
    let mut thermal_max: Vec<f64> = Vec::with_capacity(rows.len());
    let mut process_count: Vec<usize> = Vec::with_capacity(rows.len());
    let mut network_rx_kbps: Vec<f64> = Vec::with_capacity(rows.len());
    let mut network_tx_kbps: Vec<f64> = Vec::with_capacity(rows.len());

    let mut prev_rx: Option<u64> = None;
    let mut prev_tx: Option<u64> = None;
    let mut prev_ts: Option<i64> = None;

    for (ts, json) in &rows {
        let overview: SystemOverview = match serde_json::from_str(json) {
            Ok(o) => o,
            Err(_) => continue,
        };

        let total_rx: u64 = overview.network.iter().map(|n| n.rx_bytes).sum();
        let total_tx: u64 = overview.network.iter().map(|n| n.tx_bytes).sum();
        let (rx_kbps, tx_kbps) = if let (Some(pr), Some(pt), Some(pts)) = (prev_rx, prev_tx, prev_ts) {
            let dt = (ts - pts) as f64;
            if dt > 0.0 {
                (
                    (total_rx.saturating_sub(pr) as f64 / dt / 1024.0).max(0.0),
                    (total_tx.saturating_sub(pt) as f64 / dt / 1024.0).max(0.0),
                )
            } else {
                (0.0, 0.0)
            }
        } else {
            (0.0, 0.0)
        };
        prev_rx = Some(total_rx);
        prev_tx = Some(total_tx);
        prev_ts = Some(*ts);

        let max_temp = overview
            .thermal
            .iter()
            .map(|t| t.temp_celsius)
            .fold(0.0_f64, f64::max);

        timestamps.push(*ts);
        cpu_usage.push(overview.cpu.overall_usage as f64);
        memory_percent.push(overview.memory.usage_percent);
        memory_used_mb.push(overview.memory.used_mb);
        load_1.push(overview.load_avg[0]);
        load_5.push(overview.load_avg[1]);
        load_15.push(overview.load_avg[2]);
        battery_capacity.push(overview.battery.capacity);
        battery_power_w.push(if overview.battery.power_w > 0.0 {
            overview.battery.power_w
        } else {
            overview.battery.voltage_v * overview.battery.current_ma.abs() / 1000.0
        });
        thermal_max.push(max_temp);
        process_count.push(overview.process_count);
        network_rx_kbps.push(rx_kbps);
        network_tx_kbps.push(tx_kbps);
    }

    let count = timestamps.len();
    let from = timestamps.first().copied().unwrap_or(from_ts);
    let to = timestamps.last().copied().unwrap_or(now);

    success(json!({
        "range": range,
        "from": from,
        "to": to,
        "count": count,
        "timestamps": timestamps,
        "cpu_usage": cpu_usage,
        "memory_percent": memory_percent,
        "memory_used_mb": memory_used_mb,
        "load_1": load_1,
        "load_5": load_5,
        "load_15": load_15,
        "battery_capacity": battery_capacity,
        "battery_power_w": battery_power_w,
        "thermal_max": thermal_max,
        "process_count": process_count,
        "network_rx_kbps": network_rx_kbps,
        "network_tx_kbps": network_tx_kbps,
    }))
}
