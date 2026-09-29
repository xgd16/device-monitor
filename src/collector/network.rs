//! 网络指标采集
//!
//! - 网卡列表：遍历 `/sys/class/net`，读取 operstate 和 statistics
//! - WiFi：通过 `iw dev wlan0 link` 解析连接信息
//! - 蓝牙：bluetoothctl（BlueZ）读取适配器开关与设备列表，不可用时退回 hciconfig/sysfs

use super::{NetworkInterface, WifiInfo, BluetoothDevice, BluetoothInfo};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const IP_CACHE_TTL: Duration = Duration::from_secs(30);

static IP_CACHE: LazyLock<Mutex<HashMap<String, (Instant, Vec<String>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn command_path(candidates: &[&str]) -> String {
    candidates
        .iter()
        .find(|path| path.contains('/') && Path::new(path).exists())
        .or_else(|| candidates.last())
        .unwrap_or(&"")
        .to_string()
}

fn ip_cmd() -> String {
    command_path(&["/sbin/ip", "/usr/sbin/ip", "/bin/ip", "/usr/bin/ip", "ip"])
}

fn iw_cmd() -> String {
    command_path(&["/usr/sbin/iw", "/sbin/iw", "/usr/bin/iw", "/bin/iw", "iw"])
}

fn nmcli_cmd() -> String {
    command_path(&["/usr/bin/nmcli", "/bin/nmcli", "nmcli"])
}

/// 执行 bluetoothctl 子命令，返回 stdout（不可用/超时返回空串）。
/// bluetoothctl 依赖 D-Bus，BlueZ 异常时可能挂住，有 timeout 则包 3 秒上限。
fn bluetoothctl(args: &[&str]) -> String {
    let bt = command_path(&["/usr/bin/bluetoothctl", "/bin/bluetoothctl", "bluetoothctl"]);
    let timeout = command_path(&["/usr/bin/timeout", "/bin/timeout"]);
    let mut cmd = if timeout.is_empty() {
        let mut c = std::process::Command::new(&bt);
        c.args(args);
        c
    } else {
        let mut c = std::process::Command::new(timeout);
        c.arg("3").arg(&bt).args(args);
        c
    };
    cmd.output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// 解析 `bluetoothctl devices` 输出行："Device AA:BB:CC:DD:EE:FF 名称"。
fn parse_bluetooth_device(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("Device ")?;
    let mut parts = rest.splitn(2, ' ');
    let address = parts.next()?.to_string();
    let name = parts.next().unwrap_or("").trim().to_string();
    Some((address, name))
}

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

    let ip = ip_cmd();
    if let Ok(output) = std::process::Command::new(&ip)
        .args(["-f", "inet", "addr", "show", "dev", iface])
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

    if let Ok(output) = std::process::Command::new(&ip)
        .args(["-f", "inet6", "addr", "show", "dev", iface, "scope", "global"])
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
    let output = std::process::Command::new(iw_cmd())
        .args(["dev", "wlan0", "link"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let connected = output.contains("Connected to");
    if !connected {
        if let Some(info) = get_wifi_info_from_nmcli() {
            return info;
        }
    }
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

fn get_wifi_info_from_nmcli() -> Option<WifiInfo> {
    let output = std::process::Command::new(nmcli_cmd())
        .args(["-t", "-f", "ACTIVE,SSID,BSSID,CHAN,FREQ,RATE,SIGNAL", "dev", "wifi"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.first().copied() != Some("yes") {
            continue;
        }
        let ssid = fields.get(1).copied().unwrap_or("").to_string();
        let bssid = fields.get(2).copied().unwrap_or("").to_string();
        let channel = fields.get(3).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let freq = fields
            .get(4)
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let signal = fields.get(6).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
        let (_, band) = freq_to_channel_band(freq);
        return Some(WifiInfo {
            connected: true,
            ssid,
            signal_dbm: signal,
            frequency_mhz: freq,
            channel,
            band,
            bitrate: fields.get(5).copied().unwrap_or("").to_string(),
            bssid,
        });
    }
    None
}

/// 获取蓝牙适配器信息：优先 bluetoothctl（BlueZ 真实开关状态与设备列表），
/// BlueZ 不可用时退回 hciconfig/sysfs。
pub fn get_bluetooth_info() -> BluetoothInfo {
    let show = bluetoothctl(&["show"]);
    if let Some(controller) = show.lines().find(|l| l.trim_start().starts_with("Controller ")) {
        let powered = extract_field(&show, "Powered")
            .is_some_and(|v| v.eq_ignore_ascii_case("yes"));
        let name = extract_field(&show, "Name")
            .or_else(|| extract_field(&show, "Alias"))
            .unwrap_or_default();
        let address = controller.split_whitespace().nth(1).unwrap_or("").to_string();

        let mut devices: Vec<BluetoothDevice> = Vec::new();
        for line in bluetoothctl(&["devices", "Paired"]).lines() {
            if let Some((addr, dev_name)) = parse_bluetooth_device(line) {
                devices.push(BluetoothDevice {
                    address: addr,
                    name: dev_name,
                    paired: true,
                    connected: false,
                });
            }
        }
        for line in bluetoothctl(&["devices", "Connected"]).lines() {
            if let Some((addr, dev_name)) = parse_bluetooth_device(line) {
                match devices.iter_mut().find(|d| d.address == addr) {
                    Some(d) => d.connected = true,
                    None => devices.push(BluetoothDevice {
                        address: addr,
                        name: dev_name,
                        paired: false,
                        connected: true,
                    }),
                }
            }
        }

        return BluetoothInfo {
            powered,
            address,
            name,
            devices,
        };
    }
    get_bluetooth_info_fallback()
}

/// 兜底：bluetoothctl（BlueZ）不可用时，用 hciconfig 判断适配器是否 UP，
/// 地址从 hciconfig 或 sysfs 读取。
fn get_bluetooth_info_fallback() -> BluetoothInfo {
    let output = std::process::Command::new(command_path(&[
        "/usr/bin/hciconfig",
        "/bin/hciconfig",
        "hciconfig",
    ]))
    .arg("hci0")
    .output()
    .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    .unwrap_or_default();

    let powered = output
        .lines()
        .any(|l| l.split_whitespace().next() == Some("UP"));
    let address = extract_field(&output, "BD Address")
        .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
        .or_else(|| {
            fs::read_to_string("/sys/class/bluetooth/hci0/address")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default();

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
