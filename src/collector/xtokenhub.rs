//! XTokenHub 服务状态检测。
//!
//! 通过 `systemctl is-active xtokenhub` 检测服务是否运行。

use super::XTokenHubInfo;

pub fn collect() -> XTokenHubInfo {
    let status = std::process::Command::new("systemctl")
        .args(["is-active", "xtokenhub"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let running = status == "active";

    XTokenHubInfo {
        available: running,
        status,
        error: if running {
            String::new()
        } else {
            "xtokenhub service not running".to_string()
        },
    }
}
