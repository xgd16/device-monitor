//! 磁盘信息 API
//!
//! `GET /api/disk` — 返回所有 `/dev/*` 分区的容量、inode、I/O 统计及设备类型。
//!
//! 数据来源：`df -B1 -T`、/proc/diskstats、/sys/block/*/device。

use axum::Json;
use serde_json::{json, Value};
use crate::collector::disk;
use super::success;

pub async fn disk_info() -> Json<Value> {
    let mut disks = Vec::new();

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

            let (inode_total, inode_used, inode_free, inode_pct) = disk::get_inode_info(mount);
            let block_dev = disk::extract_block_dev(device);
            let (read_sectors, write_sectors, io_ticks_ms) = disk::get_io_stats(&block_dev);
            let disk_type = disk::get_disk_type(&block_dev);

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
