//! 过渡 splash：横竖屏切换的黑屏窗口期占据 DRM 显示灰色过渡画面（仅 Linux 设备）。
//!
//! 生命周期（由 panel 的 on_set_orientation 在退出前拉起）：
//!   1. 打开 card0（此刻旧面板仍是 master → 自动获主失败），`acquire_master_lock`
//!      每 20ms 轮询；旧面板 exit(0) 的瞬间获主成功。
//!   2. dumb buffer 铺灰色 → set_crtc 上屏：切换起步阶段屏幕显示灰色而非黑屏。
//!   3. 监视 panel.log 出现新面板打印的 "splash handoff wait" → `release_master_lock`
//!      （自己保持存活，灰色帧继续被扫描）→ 写 /tmp/panel-splash.gone。
//!   4. 新面板后端打开 card0 时无主 → 内核自动授予 → 初始化 → 首帧 set_crtc 接管
//!      屏幕；启动器健康检查通过后击杀本进程（灰色帧彼时已不被扫描，释放无闪烁）。
//! 兑底：3 秒拿不到 master、DropMaster 失败、或 30 秒未交接 → 退出释放资源，
//! 退化为普通黑屏路径，不卡死。
//!
//! 单实例：/tmp/panel-splash.pid 已有存活 pid 则直接退出。

const PIDF: &str = "/tmp/panel-splash.pid";
const GONEF: &str = "/tmp/panel-splash.gone";
/// main.rs 启动时打印的交接信号（stderr → panel.log）
const TOKEN: &str = "splash handoff wait";
/// 面板日志（cwd 与 panel 相同 = 部署目录，启动器每轮先截断）
const PANEL_LOG: &str = "panel.log";

#[cfg(target_os = "linux")]
fn main() {
    if let Err(e) = run() {
        eprintln!("[splash] 终止: {e}");
        let _ = std::fs::remove_file(PIDF);
        std::process::exit(1);
    }
    let _ = std::fs::remove_file(PIDF);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("[splash] 仅 Linux 设备可用");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), String> {
    use std::fs;
    use std::os::unix::io::AsFd;
    use std::path::Path;
    use std::time::{Duration, Instant};

    use drm::buffer::DrmFourcc;
    use drm::control::{connector, Device as ControlDevice};
    use drm::Device as DrmDevice;

    // ── 单实例：接管旧残留（计划内切换间隔极短时旧 splash 可能还活着；
    //    它已 DropMaster 且其帧未被扫描，或它正持有灰色帧——杀掉后本实例
    //    会重新上灰色帧，衔接无缝）──
    if let Ok(s) = fs::read_to_string(PIDF) {
        if let Ok(pid) = s.trim().parse::<u32>() {
            if pid != std::process::id() && Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::process::Command::new("kill")
                    .arg(pid.to_string())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
    }
    let _ = fs::remove_file(GONEF);
    fs::write(PIDF, format!("{}\n", std::process::id())).map_err(|e| e.to_string())?;

    struct Card(std::fs::File);
    impl AsFd for Card {
        fn as_fd(&self) -> std::os::unix::io::BorrowedFd<'_> {
            self.0.as_fd()
        }
    }
    impl DrmDevice for Card {}
    impl ControlDevice for Card {}

    let card = Card(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/dri/card0")
            .map_err(|e| format!("打开 card0: {e}"))?,
    );

    // ── 1) 等旧面板让出 master ──
    let t0 = Instant::now();
    let mut master = false;
    while t0.elapsed() < Duration::from_secs(3) {
        if card.acquire_master_lock().is_ok() {
            master = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !master {
        return Err("3s 内未获得 DRM master（旧面板未退出？）".into());
    }
    eprintln!("[splash] 获得 master");

    // ── 2) 灰色 dumb buffer 上屏 ──
    let res = card.resource_handles().map_err(|e| format!("resources: {e}"))?;
    let mut con_info = None;
    for &h in res.connectors() {
        if let Ok(ci) = card.get_connector(h, true) {
            if ci.state() == connector::State::Connected && !ci.modes().is_empty() {
                con_info = Some(ci);
                break;
            }
        }
    }
    let con = con_info.ok_or_else(|| "无已连接且带模式的 connector".to_string())?;
    let mode = *con.modes().first().ok_or_else(|| "connector 无可用模式".to_string())?;
    let (w, h) = mode.size();
    let mut db = card
        .create_dumb_buffer((u32::from(w), u32::from(h)), DrmFourcc::Xrgb8888, 32)
        .map_err(|e| format!("dumb buffer: {e}"))?;
    {
        let mut map = card
            .map_dumb_buffer(&mut db)
            .map_err(|e| format!("map dumb: {e}"))?;
        // XRGB8888 小端内存序 = [B,G,R,X]；灰色 0x2E（含 pitch 填充区，无害）
        for px in map.as_mut().chunks_mut(4) {
            if px.len() == 4 {
                px[0] = 0x2E;
                px[1] = 0x2E;
                px[2] = 0x2E;
                px[3] = 0xFF;
            }
        }
    }
    let fb = card
        .add_framebuffer(&db, 24, 32)
        .map_err(|e| format!("add_framebuffer: {e}"))?;
    let mut posted = false;
    for &ch in res.crtcs() {
        if card
            .set_crtc(ch, Some(fb), (0, 0), &[con.handle()], Some(mode))
            .is_ok()
        {
            posted = true;
            break;
        }
    }
    if !posted {
        return Err("set_crtc 全部失败".into());
    }
    eprintln!("[splash] 灰色过渡画面上屏 {w}x{h}");

    // ── 3) 监视交接信号 → DropMaster（灰色帧保持扫描）→ 写 gone ──
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut handed = false;
    while Instant::now() < deadline {
        if !handed {
            let content = std::fs::read_to_string(PANEL_LOG).unwrap_or_default();
            if content.contains(TOKEN) {
                match card.release_master_lock() {
                    Ok(()) => {
                        let _ = fs::write(GONEF, "1\n");
                        eprintln!("[splash] 已 DropMaster，灰色帧接管中，等新面板首帧");
                        handed = true;
                    }
                    Err(e) => {
                        // 交不出去就整体退出：内核随 fd 关闭释放 master，新面板
                        // 自动获主，最多黑 1~2 秒，好过卡在无主状态。
                        return Err(format!("DropMaster 失败: {e}"));
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    eprintln!("[splash] 30s 未等到交接，超时退出");
    Ok(())
}
