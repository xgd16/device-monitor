//! 数据持久化模块
//!
//! 当前实现为 SQLite 后端，存储指标快照与告警记录。

pub mod sqlite;
pub use sqlite::Database;
