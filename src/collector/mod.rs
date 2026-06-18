//! 系统指标采集模块
//!
//! 从 Linux `/proc`、`/sys` 等接口读取 CPU、内存、温度、电池、网络等数据，
//! 聚合为 `SystemOverview` 供 API、WebSocket、TUI 和告警引擎使用。

pub mod cpu;
pub mod memory;
pub mod thermal;
pub mod battery;
pub mod network;
pub mod process;
pub mod hardware;
pub mod disk;
pub mod mihomo;

use serde::{Deserialize, Serialize};

/// 系统概览快照，所有子系统的聚合数据结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemOverview {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub thermal: Vec<ThermalZone>,
    pub battery: BatteryInfo,
    pub network: Vec<NetworkInterface>,
    /// Mihomo / Clash Meta 代理状态。
    #[serde(default)]
    pub mihomo: MihomoInfo,
    /// 系统运行时间（秒），来自 `/proc/uptime`
    pub uptime: f64,
    /// 1/5/15 分钟平均负载，来自 `/proc/loadavg`
    pub load_avg: [f64; 3],
    /// 当前进程总数（`/proc` 下数字目录数）
    pub process_count: usize,
    /// 采集时刻 Unix 时间戳（秒）
    pub timestamp: i64,
}

/// Mihomo / Clash Meta 当前连接状态。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MihomoInfo {
    pub available: bool,
    pub controller: String,
    pub version: String,
    pub mode: String,
    pub tun_enabled: bool,
    pub active_group: String,
    pub active_proxy: String,
    pub proxy_chain: Vec<String>,
    pub connection_count: usize,
    pub upload_total: u64,
    pub download_total: u64,
    /// 上次订阅更新时间（Unix 秒），来自 `/etc/mihomo/.last_subscription_update`
    pub subscription_last_update: i64,
    pub error: String,
}

/// CPU 总体与各核心信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    /// 总体 CPU 使用率（%，两次采样差分计算）
    pub overall_usage: f32,
    pub cores: Vec<CpuCore>,
}

/// 单个 CPU 核心的状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuCore {
    pub id: usize,
    /// 该核心使用率（%）
    pub usage: f32,
    /// 当前频率（MHz）
    pub frequency_mhz: u64,
}

/// 内存与 Swap 使用情况。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_mb: u64,
    pub used_mb: u64,
    pub free_mb: u64,
    pub available_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
    pub usage_percent: f64,
}

/// 温度传感器区域（`/sys/class/thermal/thermal_zoneN`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalZone {
    pub id: usize,
    pub name: String,
    pub temp_celsius: f64,
}

/// 电池状态（高通平台 power_supply 节点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub capacity: u8,
    pub status: String,
    pub voltage_v: f64,
    pub current_ma: f64,
    /// 当前功率（W），电压 × |电流|；充电/放电方向由 status 区分
    #[serde(default)]
    pub power_w: f64,
    pub temp_celsius: f64,
    /// 放电: 剩余可用分钟(正数)；充电: 距充满分钟(负数)；未知: 0
    pub time_left_min: i64,
    /// 学习到的实际上限 SOC（老化电池可能低于 100）
    #[serde(default = "default_battery_hundred")]
    pub effective_max_pct: u8,
    /// 相对实际上限映射后的显示电量（0-100，用于进度条）
    #[serde(default = "default_battery_hundred")]
    pub display_capacity_pct: u8,
    /// 实际上限明显低于 100%
    #[serde(default)]
    pub is_degraded: bool,
    /// 已到达实际上限（接入电源且充电停滞）
    #[serde(default)]
    pub at_charge_limit: bool,
}

fn default_battery_hundred() -> u8 {
    100
}

/// 网络接口信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub is_up: bool,
    pub ip_addresses: Vec<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

/// 进程摘要信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: i32,
    pub name: String,
    pub status: String,
    pub cpu_usage: f32,
    pub memory_mb: u64,
    pub ppid: i32,
    pub threads: u64,
}

/// WiFi 连接信息（通过 `iw dev wlan0 link` 获取）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiInfo {
    pub connected: bool,
    pub ssid: String,
    pub signal_dbm: i32,
    pub frequency_mhz: u64,
    pub channel: u32,
    pub band: String,
    pub bitrate: String,
    pub bssid: String,
}

/// 蓝牙适配器信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothInfo {
    pub powered: bool,
    pub address: String,
    pub name: String,
    pub devices: Vec<BluetoothDevice>,
}

/// 已配对/连接的蓝牙设备。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

/// 采集完整系统概览，聚合各子模块采集结果。
pub fn collect_system_overview() -> SystemOverview {
    SystemOverview {
        cpu: cpu::collect(),
        memory: memory::collect(),
        thermal: thermal::collect(),
        battery: battery::collect(),
        network: network::collect_interfaces(),
        mihomo: mihomo::collect(),
        uptime: read_uptime(),
        load_avg: read_load_avg(),
        process_count: process::count_processes(),
        timestamp: chrono::Utc::now().timestamp(),
    }
}

/// 读取系统运行时间（秒）。
fn read_uptime() -> f64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// 读取 1/5/15 分钟负载均值。
fn read_load_avg() -> [f64; 3] {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .map(|s| {
            let parts: Vec<&str> = s.split_whitespace().collect();
            [
                parts.first().and_then(|v| v.parse().ok()).unwrap_or(0.0),
                parts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0.0),
                parts.get(2).and_then(|v| v.parse().ok()).unwrap_or(0.0),
            ]
        })
        .unwrap_or([0.0; 3])
}
