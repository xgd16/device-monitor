//! SQLite 持久化实现
//!
//! 两张表：
//! - `metrics`：每次采集的完整 SystemOverview JSON 快照
//! - `alerts`：告警引擎产生的告警记录

use rusqlite::{Connection, params};
use std::sync::Mutex;
use serde_json::Value;

/// 线程安全的 SQLite 数据库封装（内部 Mutex 保护 Connection）。
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// 打开/创建数据库文件并初始化表结构。
    pub fn new(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_metrics_ts ON metrics(timestamp);

            CREATE TABLE IF NOT EXISTS alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                level TEXT NOT NULL,
                title TEXT NOT NULL,
                message TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_alerts_ts ON alerts(timestamp);
            "
        )?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    /// 存储一次系统指标快照（序列化为 JSON）。
    pub fn store_metrics(&self, data: &impl serde::Serialize) -> Result<(), rusqlite::Error> {
        let json = serde_json::to_string(data).unwrap_or_default();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO metrics (timestamp, data) VALUES (?1, ?2)",
            params![chrono::Utc::now().timestamp(), json],
        )?;
        Ok(())
    }

    /// 写入一条告警记录。
    pub fn store_alert(&self, level: &str, title: &str, message: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO alerts (timestamp, level, title, message) VALUES (?1, ?2, ?3, ?4)",
            params![chrono::Utc::now().timestamp(), level, title, message],
        )?;
        Ok(())
    }

    /// 查询最近 N 条告警，按 id 降序。
    pub fn get_alerts(&self, limit: usize) -> Result<Vec<Value>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, level, title, message FROM alerts ORDER BY id DESC LIMIT ?1"
        )?;

        let limit_i64 = limit as i64;
        let rows = stmt.query_map(params![limit_i64], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "timestamp": row.get::<_, i64>(1)?,
                "level": row.get::<_, String>(2)?,
                "title": row.get::<_, String>(3)?,
                "message": row.get::<_, String>(4)?,
            }))
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// 清理超过指定天数的旧数据，并执行 VACUUM 回收磁盘空间。
    ///
    /// 返回 `(删除的 metrics 条数, 删除的 alerts 条数)`。
    pub fn cleanup_old_data(&self, days: i64) -> Result<(usize, usize), rusqlite::Error> {
        let cutoff = chrono::Utc::now().timestamp() - (days * 24 * 3600);
        let conn = self.conn.lock().unwrap();
        
        let metrics_deleted = conn.execute(
            "DELETE FROM metrics WHERE timestamp < ?1",
            params![cutoff],
        )?;
        
        let alerts_deleted = conn.execute(
            "DELETE FROM alerts WHERE timestamp < ?1",
            params![cutoff],
        )?;
        
        conn.execute_batch("VACUUM;")?;
        
        Ok((metrics_deleted, alerts_deleted))
    }

    /// 获取数据库统计信息（记录数、时间范围等）。
    pub fn get_stats(&self) -> Result<Value, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        
        let metrics_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM metrics", [], |row| row.get(0)
        )?;
        
        let alerts_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM alerts", [], |row| row.get(0)
        )?;
        
        let oldest_metric: Option<i64> = conn.query_row(
            "SELECT MIN(timestamp) FROM metrics", [], |row| row.get(0)
        ).ok();
        
        let newest_metric: Option<i64> = conn.query_row(
            "SELECT MAX(timestamp) FROM metrics", [], |row| row.get(0)
        ).ok();
        
        Ok(serde_json::json!({
            "metrics_count": metrics_count,
            "alerts_count": alerts_count,
            "oldest_metric": oldest_metric,
            "newest_metric": newest_metric,
            "retention_days": 7
        }))
    }
}
