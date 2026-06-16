//! 网络指标采集
//!
//! - 网卡列表：遍历 `/sys/class/net`，读取 operstate 和 statistics
//! - WiFi：通过 `iw dev wlan0 link` 解析连接信息
//! - 蓝牙：读取 `/sys/class/bluetooth/hci0` 基本信息

use super::{NetworkInterface, WifiInfo, BluetoothInfo};
use std::collections::HashMap;
use std::fs;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const IP_CACHE_TTL: Duration = Duration::from_secs(30);

static IP_CACHE: LazyLock<Mutex<HashMap<String, (Instant, Vec<String>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 采集所有非 loopback 网络接口的信息。
pub fn collect_interfaces() -> Vec<NetworkInterface> {
    let mut interfaces = Vec::new();

    let net_dir = "/sys/class/net";
    if let Ok(entries) = fs::read_dir(net_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "lo" { continue; }

            let base = format!("{}/{}", net_dir, name);

            let operstate = fs::read_to_string(format!("{}/operstate", base))
                .unwrap_or_default()
                .trim()
                .to_string();
            let is_up = operstate == "up";

            let rx_bytes = read_stat(&base, "rx_bytes");
            let tx_bytes = read_stat(&base, "tx_bytes");
            let rx_packets = read_stat(&base, "rx_packets");
            let tx_packets = read_stat(&base, "tx_packets");

            let ip_addresses = get_ips(&name);

            interfaces.push(NetworkInterface {
                name,
                is_up,
                ip_addresses,
                rx_bytes,
                tx_bytes,
                rx_packets,
                tx_packets,
            });
        }
    }

    interfaces
}

/// 读取网卡 statistics 下的累计计数器。
fn read_stat(base: &str, field: &str) -> u64 {
    fs::read_to_string(format!("{}/statistics/{}", base, field))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// 通过 `ip addr` 获取 IPv4 和 IPv6 地址。
fn get_ips(iface: &str) -> Vec<String> {
    if let Some(cached) = {
        let cache = IP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(iface).and_then(|(ts, ips)| {
            if ts.elapsed() <= IP_CACHE_TTL {
                Some(ips.clone())
            } else {
                None
            }
        })
    } {
        return cached;
    }

    let mut ips = Vec::new();

    if let Ok(output) = std::process::Command::new("ip")
        .args(["-4", "addr", "show", iface])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("inet ") {
                if let Some(ip) = line.split_whitespace().nth(1) {
                    ips.push(ip.to_string());
                }
            }
        }
    }

    if let Ok(output) = std::process::Command::new("ip")
        .args(["-6", "addr", "show", iface, "scope", "global"])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("inet6 ") {
                if let Some(ip) = line.split_whitespace().nth(1) {
                    ips.push(ip.to_string());
                }
            }
        }
    }

    let mut cache = IP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(iface.to_string(), (Instant::now(), ips.clone()));
    ips
}

/// 获取 wlan0 的 WiFi 连接详情。
pub fn get_wifi_info() -> WifiInfo {
    let output = std::process::Command::new("iw")
        .args(["dev", "wlan0", "link"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let connected = output.contains("Connected to");
    let ssid = extract_field(&output, "SSID").unwrap_or_default();
    let signal = extract_field(&output, "signal")
        .and_then(|s| s.split_whitespace().next()?.parse::<i32>().ok())
        .unwrap_or(0);
    let freq = extract_field(&output, "freq")
        .and_then(|s| s.split('.').next()?.parse::<u64>().ok())
        .unwrap_or(0);

    let (channel, band) = freq_to_channel_band(freq);

    let bitrate = extract_field(&output, "rx bitrate")
        .or_else(|| extract_field(&output, "tx bitrate"))
        .unwrap_or_default();
    let bssid = output.lines()
        .find(|l| l.contains("Connected to"))
        .and_then(|l| l.split_whitespace().nth(2))
        .unwrap_or("")
        .to_string();

    WifiInfo {
        connected,
        ssid,
        signal_dbm: signal,
        frequency_mhz: freq,
        channel,
        band,
        bitrate,
        bssid,
    }
}

/// 获取蓝牙适配器基本信息（设备列表暂未实现）。
pub fn get_bluetooth_info() -> BluetoothInfo {
    let powered = fs::read_to_string("/sys/class/bluetooth/hci0/power/runtime_status")
        .map(|s| s.trim() == "active")
        .unwrap_or(false);

    let address = fs::read_to_string("/sys/class/bluetooth/hci0/address")
        .unwrap_or_default()
        .trim()
        .to_string();

    BluetoothInfo {
        powered,
        address,
        name: String::new(),
        devices: Vec::new(),
    }
}

/// 从 iw 输出中按关键字提取冒号后的值。
fn extract_field<'a>(text: &'a str, key: &str) -> Option<String> {
    let key_lower = key.to_lowercase();
    for line in text.lines() {
        if line.to_lowercase().contains(&key_lower) {
            return Some(line.split(':').skip(1).collect::<Vec<_>>().join(":").trim().to_string());
        }
    }
    None
}

/// 根据频率 (MHz) 计算 WiFi 信道和频段（2.4G/5G/6G）。
fn freq_to_channel_band(freq_mhz: u64) -> (u32, String) {
    match freq_mhz {
        // 2.4 GHz: channels 1-13 (2412-2472 MHz, 5 MHz step, offset 2412)
        2412..=2472 => (((freq_mhz - 2412) / 5 + 1) as u32, "2.4G".to_string()),
        // 2.4 GHz channel 14 (2484 MHz)
        2484 => (14, "2.4G".to_string()),
        // 5 GHz: channels 7-165 (5035-5825 MHz)
        5035..=5825 => (((freq_mhz - 5000) / 5) as u32, "5G".to_string()),
        // 6 GHz: channels 1-233 (5955-7115 MHz)
        5955..=7115 => (((freq_mhz - 5950) / 5) as u32, "6G".to_string()),
        _ => (0, String::new()),
    }
}
