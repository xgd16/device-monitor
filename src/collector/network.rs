//! 网络指标采集
//!
//! - 网卡列表：遍历 `/sys/class/net`，读取 operstate 和 statistics
//! - WiFi：通过 `iw dev wlan0 link` 解析连接信息
//! - 蓝牙：bluetoothctl（BlueZ）读取适配器开关与设备列表，不可用时退回 hciconfig/sysfs

use super::{BluetoothDevice, BluetoothInfo, NetworkInterface, WifiInfo, WifiNetwork};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const IP_CACHE_TTL: Duration = Duration::from_secs(30);

/// IP 查询缓存：命令名 → (写入时刻, 地址列表)
type IpCache = HashMap<String, (Instant, Vec<String>)>;

static IP_CACHE: LazyLock<Mutex<IpCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

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

fn bluetoothctl_cmd() -> String {
    command_path(&["/usr/bin/bluetoothctl", "/bin/bluetoothctl", "bluetoothctl"])
}

fn hciconfig_cmd() -> String {
    command_path(&["/usr/bin/hciconfig", "/bin/hciconfig", "hciconfig"])
}

fn rfkill_cmd() -> String {
    command_path(&["/usr/sbin/rfkill", "/sbin/rfkill", "/usr/bin/rfkill", "rfkill"])
}

/// 执行 bluetoothctl 子命令，返回 stdout（不可用/超时返回空串）。
/// bluetoothctl 依赖 D-Bus，BlueZ 异常时可能挂住，有 timeout 则包 3 秒上限。
fn bluetoothctl(args: &[&str]) -> String {
    let bt = bluetoothctl_cmd();
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
    cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
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
            if name == "lo" {
                continue;
            }

            let base = format!("{}/{}", net_dir, name);

            let operstate = fs::read_to_string(format!("{}/operstate", base)).unwrap_or_default().trim().to_string();
            let is_up = operstate == "up";

            let rx_bytes = read_stat(&base, "rx_bytes");
            let tx_bytes = read_stat(&base, "tx_bytes");
            let rx_packets = read_stat(&base, "rx_packets");
            let tx_packets = read_stat(&base, "tx_packets");

            let ip_addresses = get_ips(&name);

            interfaces.push(NetworkInterface { name, is_up, ip_addresses, rx_bytes, tx_bytes, rx_packets, tx_packets });
        }
    }

    interfaces
}

/// 读取网卡 statistics 下的累计计数器。
fn read_stat(base: &str, field: &str) -> u64 {
    fs::read_to_string(format!("{}/statistics/{}", base, field)).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

/// 通过 `ip addr` 获取 IPv4 和 IPv6 地址。
fn get_ips(iface: &str) -> Vec<String> {
    if let Some(cached) = {
        let cache = IP_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.get(iface).and_then(|(ts, ips)| if ts.elapsed() <= IP_CACHE_TTL { Some(ips.clone()) } else { None })
    } {
        return cached;
    }

    let mut ips = Vec::new();

    let ip = ip_cmd();
    if let Ok(output) = std::process::Command::new(&ip).args(["-f", "inet", "addr", "show", "dev", iface]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("inet ")
                && let Some(ip) = line.split_whitespace().nth(1)
            {
                ips.push(ip.to_string());
            }
        }
    }

    if let Ok(output) =
        std::process::Command::new(&ip).args(["-f", "inet6", "addr", "show", "dev", iface, "scope", "global"]).output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if line.contains("inet6 ")
                && let Some(ip) = line.split_whitespace().nth(1)
            {
                ips.push(ip.to_string());
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
    if !connected && let Some(info) = get_wifi_info_from_nmcli() {
        return info;
    }
    let ssid = extract_field(&output, "SSID").unwrap_or_default();
    let signal =
        extract_field(&output, "signal").and_then(|s| s.split_whitespace().next()?.parse::<i32>().ok()).unwrap_or(0);
    let freq = extract_field(&output, "freq").and_then(|s| s.split('.').next()?.parse::<u64>().ok()).unwrap_or(0);

    let (channel, band) = freq_to_channel_band(freq);

    let bitrate =
        extract_field(&output, "rx bitrate").or_else(|| extract_field(&output, "tx bitrate")).unwrap_or_default();
    let bssid = output
        .lines()
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
        let freq =
            fields.get(4).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
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

// ────────────────────────── WiFi 扫描 / 连接（NetworkManager） ──────────────────────────
//
// 设备上 NetworkManager 常驻、nmcli 1.56 可用。扫描只在面板打开 WiFi 操作页时
// 由 `/api/network/wifi/scan` 触发：`--rescan yes` 会真正让射频扫描（实测约 3s），
// 常驻轮询既费电也会挤占当前连接的吞吐，所以绝不放进入 2s 采集心跳。

/// 按 nmcli `-t` 规则切分一行：列分隔符是未转义的冒号，值里的 `\:` / `\\` 需还原。
fn nmcli_split(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ':' {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    if escaped {
        cur.push('\\');
    }
    fields.push(cur);
    fields
}

/// 解析 `nmcli -t -f IN-USE,SSID,BSSID,SIGNAL,SECURITY,FREQ,CHAN dev wifi list` 的一行。
fn parse_scan_line(line: &str) -> Option<WifiNetwork> {
    let f = nmcli_split(line);
    if f.len() < 7 {
        return None;
    }
    let ssid = f[1].clone();
    // 隐藏网络（SSID 为空）无法在面板上选择，直接丢弃
    if ssid.is_empty() {
        return None;
    }
    let security = f[4].trim().to_string();
    let frequency_mhz = f[5].split_whitespace().next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let (calc_channel, band) = freq_to_channel_band(frequency_mhz);
    let channel = f[6].trim().parse::<u32>().unwrap_or(calc_channel);
    Some(WifiNetwork {
        ssid,
        bssid: f[2].clone(),
        signal: f[3].trim().parse::<i32>().unwrap_or(0),
        secure: !security.is_empty(),
        security,
        frequency_mhz,
        channel,
        band,
        in_use: f[0].trim() == "*",
        saved: false,
    })
}

/// 同一 SSID 可能有多个 BSSID：保留正在连接的那条，否则保留信号最强的。
fn dedup_by_ssid(rows: Vec<WifiNetwork>) -> Vec<WifiNetwork> {
    let mut map: HashMap<String, WifiNetwork> = HashMap::new();
    for row in rows {
        match map.get(&row.ssid) {
            Some(old) if old.in_use || (!row.in_use && old.signal >= row.signal) => {}
            _ => {
                map.insert(row.ssid.clone(), row);
            }
        }
    }
    let mut out: Vec<WifiNetwork> = map.into_values().collect();
    out.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)).then_with(|| a.ssid.cmp(&b.ssid)));
    out
}

/// 带超时执行子进程（优先用系统 `timeout`，缺失时退化为阻塞执行）。
/// 返回 stdout；失败时把 stderr（没有则 stdout）作为错误带回面板显示。
fn run_command(program: &str, args: &[&str], timeout_secs: u64) -> Result<String, String> {
    let timeout = command_path(&["/usr/bin/timeout", "/bin/timeout", "timeout"]);
    let mut cmd = if timeout.is_empty() {
        let mut c = std::process::Command::new(program);
        c.args(args);
        c
    } else {
        let mut c = std::process::Command::new(&timeout);
        c.arg(timeout_secs.to_string()).arg(program).args(args);
        c
    };
    let out = cmd.output().map_err(|e| format!("执行 {program} 失败: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        let msg = if stderr.is_empty() { stdout } else { stderr };
        let msg = if msg.is_empty() { format!("退出码 {:?}", out.status.code()) } else { msg };
        return Err(msg);
    }
    Ok(stdout)
}

/// 已保存的 WiFi 配置：(连接名, SSID)。
fn saved_wifi_profiles() -> Vec<(String, String)> {
    let nmcli = nmcli_cmd();
    let Ok(text) = run_command(&nmcli, &["-t", "-f", "NAME,TYPE", "connection", "show"], 5) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let f = nmcli_split(line);
        if f.len() < 2 || f[1] != "802-11-wireless" {
            continue;
        }
        let name = f[0].clone();
        let ssid = run_command(&nmcli, &["-g", "802-11-wireless.ssid", "connection", "show", &name], 5)
            .ok()
            .map(|s| nmcli_split(s.trim()).into_iter().next().unwrap_or_default())
            .unwrap_or_default();
        if !ssid.is_empty() {
            out.push((name, ssid));
        }
    }
    out
}

/// 主动扫描周边 WiFi（仅供面板 WiFi 操作页按需调用）。
pub fn scan_wifi_networks() -> Result<Vec<WifiNetwork>, String> {
    let nmcli = nmcli_cmd();
    let base = ["-t", "-f", "IN-USE,SSID,BSSID,SIGNAL,SECURITY,FREQ,CHAN", "dev", "wifi", "list"];
    let mut args: Vec<&str> = base.to_vec();
    args.extend(["--rescan", "yes"]);
    // NM 偶尔会拒绝「刚扫过」的重扫请求：退回缓存列表，页面至少还有数据
    let text = match run_command(&nmcli, &args, 25) {
        Ok(t) => t,
        Err(e) => run_command(&nmcli, &base, 8).map_err(|e2| format!("扫描失败：{e}；读取缓存失败：{e2}"))?,
    };
    let saved: Vec<String> = saved_wifi_profiles().into_iter().map(|(_, ssid)| ssid).collect();
    let mut rows: Vec<WifiNetwork> = text.lines().filter_map(parse_scan_line).collect();
    for row in &mut rows {
        row.saved = saved.iter().any(|s| s == &row.ssid);
    }
    Ok(dedup_by_ssid(rows))
}

/// 连接指定 SSID：密码为空时优先激活已保存配置，否则交给 `nmcli device wifi connect`。
/// 密码走 argv（本机 root 服务、无 shell 拼接）；nmcli 的 `--wait` 与外部 timeout 双重兜底。
pub fn connect_wifi(ssid: &str, password: &str) -> Result<String, String> {
    let ssid = ssid.trim();
    if ssid.is_empty() {
        return Err("SSID 不能为空".to_string());
    }
    if password.chars().count() > 63 {
        return Err("WiFi 密码最长 63 个字符".to_string());
    }
    let nmcli = nmcli_cmd();
    if password.is_empty()
        && let Some((name, _)) = saved_wifi_profiles().into_iter().find(|(_, s)| s == ssid)
    {
        run_command(&nmcli, &["--wait", "45", "connection", "up", "id", &name], 55)?;
        return Ok(format!("已连接到 {ssid}"));
    }
    let mut args: Vec<&str> = vec!["--wait", "45", "device", "wifi", "connect", ssid];
    if !password.is_empty() {
        args.push("password");
        args.push(password);
    }
    run_command(&nmcli, &args, 55)?;
    Ok(format!("已连接到 {ssid}"))
}

/// 获取蓝牙适配器信息：优先 bluetoothctl（BlueZ 真实开关状态与设备列表），
/// BlueZ 不可用时退回 hciconfig/sysfs。
///
/// rfkill 软锁时即使 hci0 还保持 UP（本机 BlueZ 未接管时就是这样），对外也报「关」，
/// 避免面板显示开机但实际射频已被禁用。
pub fn get_bluetooth_info() -> BluetoothInfo {
    let blocked = rfkill_soft_blocked("bluetooth");
    let mut info = read_bluetooth_info();
    if blocked {
        info.powered = false;
        info.devices.clear();
    }
    info
}

fn read_bluetooth_info() -> BluetoothInfo {
    let show = bluetoothctl(&["show"]);
    if let Some(controller) = show.lines().find(|l| l.trim_start().starts_with("Controller ")) {
        let powered = extract_field(&show, "Powered").is_some_and(|v| v.eq_ignore_ascii_case("yes"));
        let name = extract_field(&show, "Name").or_else(|| extract_field(&show, "Alias")).unwrap_or_default();
        let address = controller.split_whitespace().nth(1).unwrap_or("").to_string();

        let mut devices: Vec<BluetoothDevice> = Vec::new();
        for line in bluetoothctl(&["devices", "Paired"]).lines() {
            if let Some((addr, dev_name)) = parse_bluetooth_device(line) {
                devices.push(BluetoothDevice { address: addr, name: dev_name, paired: true, connected: false });
            }
        }
        for line in bluetoothctl(&["devices", "Connected"]).lines() {
            if let Some((addr, dev_name)) = parse_bluetooth_device(line) {
                match devices.iter_mut().find(|d| d.address == addr) {
                    Some(d) => d.connected = true,
                    None => {
                        devices.push(BluetoothDevice { address: addr, name: dev_name, paired: false, connected: true })
                    }
                }
            }
        }

        return BluetoothInfo { powered, address, name, devices };
    }
    get_bluetooth_info_fallback()
}

/// 兜底：bluetoothctl（BlueZ）不可用时，用 hciconfig 判断适配器是否 UP，
/// 地址从 hciconfig 或 sysfs 读取。
fn get_bluetooth_info_fallback() -> BluetoothInfo {
    let output = std::process::Command::new(hciconfig_cmd())
        .arg("hci0")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let powered = output.lines().any(|l| l.split_whitespace().next() == Some("UP"));
    let address = extract_field(&output, "BD Address")
        .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
        .or_else(|| fs::read_to_string("/sys/class/bluetooth/hci0/address").ok().map(|s| s.trim().to_string()))
        .unwrap_or_default();

    BluetoothInfo { powered, address, name: String::new(), devices: Vec::new() }
}

/// rfkill 软锁状态（kind = "bluetooth"）。
fn rfkill_soft_blocked(kind: &str) -> bool {
    let Ok(text) = run_command(&rfkill_cmd(), &["list", kind], 5) else { return false };
    text.to_ascii_lowercase().contains("soft blocked: yes")
}

/// BlueZ 是否已接管至少一个控制器。本机当前是反例：hci0 由 hci_uart 挂上但
/// 没进 mgmt（`bluetoothctl list` / `btmgmt info` 都是 0 个），只能走 HCI + rfkill。
fn bluetooth_controller_available() -> bool {
    run_command(&bluetoothctl_cmd(), &["list"], 6).map(|t| t.contains("Controller ")).unwrap_or(false)
}

/// 开关蓝牙：优先 BlueZ（`bluetoothctl power`），没有控制器时退回 `hciconfig` + `rfkill`。
///
/// 关闭顺序：先下电/断开连接，再 rfkill 软锁（保证重启后仍保持关闭）。
/// 打开顺序：先解 rfkill 再上电，否则适配器无法启动。
pub fn set_bluetooth_power(enabled: bool) -> Result<(), String> {
    if bluetooth_controller_available() {
        if enabled {
            let _ = run_command(&rfkill_cmd(), &["unblock", "bluetooth"], 5);
            run_command(&bluetoothctl_cmd(), &["power", "on"], 10)?;
        } else {
            run_command(&bluetoothctl_cmd(), &["power", "off"], 10)?;
            let _ = run_command(&rfkill_cmd(), &["block", "bluetooth"], 5);
        }
        return Ok(());
    }

    // 无 BlueZ 控制器：hci_uart 直接把 hci0 暴露给 HCI，rfkill 负责射频软锁
    if enabled {
        run_command(&rfkill_cmd(), &["unblock", "bluetooth"], 5)?;
        run_command(&hciconfig_cmd(), &["hci0", "up"], 8)?;
    } else {
        run_command(&hciconfig_cmd(), &["hci0", "down"], 8)?;
        let _ = run_command(&rfkill_cmd(), &["block", "bluetooth"], 5);
    }
    Ok(())
}

/// 从 iw 输出中按关键字提取冒号后的值。
fn extract_field(text: &str, key: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmcli_split_handles_escaped_colons() {
        let f = nmcli_split(r"*:net_5g:D0\:5B\:CB\:C5\:67\:A2:100:WPA2 WPA3:5180 MHz:36");
        assert_eq!(f.len(), 7);
        assert_eq!(f[0], "*");
        assert_eq!(f[2], "D0:5B:CB:C5:67:A2");
        assert_eq!(f[4], "WPA2 WPA3");
    }

    #[test]
    fn parse_scan_line_marks_open_and_secure() {
        let open = parse_scan_line(r" :OpenNet:AA\:BB\:CC\:DD\:EE\:FF:42::2437 MHz:6").unwrap();
        assert!(!open.secure);
        assert_eq!(open.band, "2.4G");
        assert_eq!(open.channel, 6);
        assert!(!open.in_use);

        let saved = parse_scan_line(r"*:net_5g:D0\:5B\:CB\:C5\:67\:A2:100:WPA2 WPA3:5180 MHz:36").unwrap();
        assert!(saved.secure);
        assert!(saved.in_use);
        assert_eq!(saved.band, "5G");
        assert_eq!(saved.channel, 36);
    }

    #[test]
    fn hidden_ssid_is_skipped() {
        assert!(parse_scan_line(r" ::3A\:70\:4F\:3B\:D1\:E6:52:WPA2:5220 MHz:44").is_none());
    }

    #[test]
    fn dedup_keeps_strongest_and_connected() {
        let rows = vec![
            parse_scan_line(r" :XM_MX:AA\:BB\:CC\:DD\:EE\:01:40:WPA2:2432 MHz:5").unwrap(),
            parse_scan_line(r" :XM_MX:AA\:BB\:CC\:DD\:EE\:02:70:WPA2:5220 MHz:44").unwrap(),
            parse_scan_line(r"*:net_5g:AA\:BB\:CC\:DD\:EE\:03:30:WPA2:5180 MHz:36").unwrap(),
            parse_scan_line(r" :net_5g:AA\:BB\:CC\:DD\:EE\:04:90:WPA2:5180 MHz:36").unwrap(),
        ];
        let out = dedup_by_ssid(rows);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].ssid, "net_5g");
        assert!(out[0].in_use);
        assert_eq!(out[1].ssid, "XM_MX");
        assert_eq!(out[1].signal, 70);
    }
}
