//! 系统日志 API
//!
//! `GET /api/logs?lines=&keyword=&level=` — 读取系统日志并支持过滤。

use axum::Json;
use axum::extract::Query;
use serde::Deserialize;
use serde_json::Value;
use std::fs;

/// 日志查询参数。
#[derive(Deserialize)]
pub struct LogQuery {
    /// 返回的最大行数（默认 100，取尾部）
    pub lines: Option<usize>,
    /// 关键字过滤（包含匹配）
    pub keyword: Option<String>,
    /// 日志级别过滤：error/warn/info/debug
    pub level: Option<String>,
}

pub async fn get_logs(Query(q): Query<LogQuery>) -> Json<Value> {
    let max_lines = q.lines.unwrap_or(100);

    // 优先级：/var/log/messages → /var/log/syslog → dmesg
    let output = fs::read_to_string("/var/log/messages")
        .or_else(|_| fs::read_to_string("/var/log/syslog"))
        .unwrap_or_else(|_| {
            std::process::Command::new("dmesg")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default()
        });

    let mut logs: Vec<&str> = output.lines().collect();

    if let Some(ref kw) = q.keyword {
        logs.retain(|l| l.contains(kw.as_str()));
    }

    if let Some(ref level) = q.level {
        let pattern = match level.as_str() {
            "error" | "err" => "error",
            "warn" | "warning" => "warn",
            "info" => "info",
            "debug" => "debug",
            _ => "",
        };
        if !pattern.is_empty() {
            logs.retain(|l| l.to_lowercase().contains(pattern));
        }
    }

    let total = logs.len();
    let start = if total > max_lines { total - max_lines } else { 0 };
    let logs: Vec<&str> = logs[start..].to_vec();

    Json(serde_json::json!({
        "code": 0,
        "data": {
            "lines": logs,
            "total": total,
            "showing": logs.len(),
        }
    }))
}
