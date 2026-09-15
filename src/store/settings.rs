//! 运行时可调的小型设置项。
//!
//! 与 `battery_*.txt` 同风格：纯文本文件放在运行目录，重启保留；
//! 文件缺失、解析失败或值不在允许列表时回落默认，不致命。

use std::fs;

/// 允许的界面刷新间隔（秒）。DRM 屏渲染线程 1Hz 轮询，1 秒已是最快档。
pub const REFRESH_CHOICES: &[u64] = &[1, 3, 5, 10];
pub const DEFAULT_REFRESH_SECS: u64 = 5;
const REFRESH_FILE: &str = "refresh_secs.txt";

fn is_valid(secs: u64) -> bool {
    REFRESH_CHOICES.contains(&secs)
}

/// 读取界面刷新间隔。
pub fn load_refresh_secs() -> u64 {
    fs::read_to_string(REFRESH_FILE)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&v| is_valid(v))
        .unwrap_or(DEFAULT_REFRESH_SECS)
}

/// 保存界面刷新间隔。调用方需先校验 `secs` 在允许列表内。
pub fn save_refresh_secs(secs: u64) -> std::io::Result<()> {
    if !is_valid(secs) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("刷新间隔仅支持 {REFRESH_CHOICES:?}"),
        ));
    }
    fs::write(REFRESH_FILE, format!("{secs}\n"))
}

// ── 屏幕朝向 ──

/// 面板物理上是竖屏（如 1080×2340），仪表内容横着放，所以有两个横向朝向：
/// `Rot90` 和 `Rot270`（互为 180°）。手机怎么摆就选哪个 —— 这是**物理摆位**决定的，
/// 不是每次开机都该重选的东西，所以落盘持久化。
const ROTATION_FILE: &str = "rotation.txt";

/// 朝向标识。存文件用字符串，避免以后加反射/新朝向时数字含义漂移。
pub const ROTATION_CHOICES: &[&str] = &["rot90", "rot270"];

/// 读取屏幕朝向；缺失或非法返回 `None`（由调用方用命令行默认值兜底）。
pub fn load_rotation() -> Option<String> {
    fs::read_to_string(ROTATION_FILE)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| ROTATION_CHOICES.contains(&s.as_str()))
}

/// 保存屏幕朝向。
pub fn save_rotation(rot: &str) -> std::io::Result<()> {
    if !ROTATION_CHOICES.contains(&rot) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("朝向仅支持 {ROTATION_CHOICES:?}"),
        ));
    }
    fs::write(ROTATION_FILE, format!("{rot}\n"))
}
