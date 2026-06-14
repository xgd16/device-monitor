//! 磁盘信息 API
//!
//! `GET /api/disk` — 返回所有 `/dev/*` 分区的容量、inode、I/O 统计及设备类型。
//!
//! 数据来源：`df -B1 -T`、/proc/diskstats、/sys/block/*/device。

use axum::Json;
use serde_json::{json, Value};
use std::fs;
use super::success;

pub async fn disk_info() -> Json<Value> {
    let mut disks = Vec::new();

    // df -B1 -T：字节为单位 + 文件系统类型
    if let Ok(output) = std::process::Command::new("df").args(["-B1", "-T"]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 7 { continue; }
            let device = parts[0];
            let fstype = parts[1];
            let total: u64 = parts[2].parse().unwrap_or(0);
            let used: u64 = parts[3].parse().unwrap_or(0);
            let available: u64 = parts[4].parse().unwrap_or(0);
            let mount = parts[6];

            if !device.starts_with("/dev/") { continue; }

            let usage = if total > 0 { (used as f64 / total as f64) * 100.0 } else { 0.0 };

            let (inode_total, inode_used, inode_free, inode_pct) = get_inode_info(mount);
            let block_dev = extract_block_dev(device);
            let (read_sectors, write_sectors, io_ticks_ms) = get_io_stats(&block_dev);
            let disk_type = get_disk_type(&block_dev);

            disks.push(json!({
                "device": device,
                "mount": mount,
                "fstype": fstype,
                "total_mb": total / 1024 / 1024,
                "used_mb": used / 1024 / 1024,
                "available_mb": available / 1024 / 1024,
                "usage_percent": usage,
                "inode_total": inode_total,
                "inode_used": inode_used,
                "inode_free": inode_free,
                "inode_percent": inode_pct,
                "read_sectors": read_sectors,
                "write_sectors": write_sectors,
                "io_ticks_ms": io_ticks_ms,
                "disk_type": disk_type,
            }));
        }
    }

    success(disks)
}

/// 通过 `df -i` 获取指定挂载点的 inode 使用情况。
fn get_inode_info(mount: &str) -> (u64, u64, u64, f64) {
    let output = std::process::Command::new("df")
        .args(["-i", mount])
        .output();
    match output {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            for line in text.lines().skip(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 5 {
                    let total: u64 = parts[1].parse().unwrap_or(0);
                    let used: u64 = parts[2].parse().unwrap_or(0);
                    let free: u64 = parts[3].parse().unwrap_or(0);
                    let pct_str = parts[4].trim_end_matches('%');
                    let pct: f64 = pct_str.parse().unwrap_or(0.0);
                    return (total, used, free, pct);
                }
            }
            (0, 0, 0, 0.0)
        }
        _ => (0, 0, 0, 0.0),
    }
}

/// 从设备路径提取块设备名。
///
/// 例：`/dev/loop0p2` → `loop0`，`/dev/mmcblk0p1` → `mmcblk0`
fn extract_block_dev(device: &str) -> String {
    let name = device.trim_start_matches("/dev/");
    // 去掉 pN 分区后缀
    if let Some(pos) = name.rfind("p") {
        let (base, part) = name.split_at(pos);
        if part[1..].chars().all(|c| c.is_ascii_digit()) && !base.is_empty() {
            return base.to_string();
        }
    }
    // 去掉末尾数字分区号
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed.is_empty() { name.to_string() } else { trimmed.to_string() }
}

/// 从 `/proc/diskstats` 读取块设备的累计读写扇区数和 I/O 耗时。
fn get_io_stats(block_dev: &str) -> (u64, u64, u64) {
    let content = fs::read_to_string("/proc/diskstats").unwrap_or_default();
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 13 && parts.get(2) == Some(&block_dev) {
            let read_sectors: u64 = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(0);
            let write_sectors: u64 = parts.get(9).and_then(|s| s.parse().ok()).unwrap_or(0);
            let io_ticks: u64 = parts.get(12).and_then(|s| s.parse().ok()).unwrap_or(0);
            return (read_sectors, write_sectors, io_ticks);
        }
    }
    (0, 0, 0)
}

/// 判断磁盘类型：读取 vendor/model 或 rotational 属性。
fn get_disk_type(block_dev: &str) -> String {
    let vendor = fs::read_to_string(format!("/sys/block/{}/device/vendor", block_dev))
        .unwrap_or_default().trim().to_string();
    let model = fs::read_to_string(format!("/sys/block/{}/device/model", block_dev))
        .unwrap_or_default().trim().to_string();
    if !vendor.is_empty() || !model.is_empty() {
        return format!("{} {}", vendor, model).trim().to_string();
    }
    // loop 设备尝试查找底层物理设备
    if block_dev.starts_with("loop") {
        for sd in &["sda", "sdb", "mmcblk0"] {
            let v = fs::read_to_string(format!("/sys/block/{}/device/vendor", sd))
                .unwrap_or_default().trim().to_string();
            let m = fs::read_to_string(format!("/sys/block/{}/device/model", sd))
                .unwrap_or_default().trim().to_string();
            if !v.is_empty() || !m.is_empty() {
                return format!("{} {}", v, m).trim().to_string();
            }
        }
        return "Flash".to_string();
    }
    let rotational = fs::read_to_string(format!("/sys/block/{}/queue/rotational", block_dev))
        .unwrap_or_default().trim().to_string();
    match rotational.as_str() {
        "0" => "Flash".to_string(),
        "1" => "HDD".to_string(),
        _ => "未知".to_string(),
    }
}
