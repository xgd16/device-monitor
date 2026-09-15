//! 音量键热键监听（用于切换物理屏页面）。
//!
//! 不硬编码 event 编号：通过 `/sys/class/input/eventN/device/capabilities/key`
//! 的 KEY 位图自动发现支持 KEY_VOLUMEUP(115) / KEY_VOLUMEDOWN(114) 的设备。
//! 用 `poll(2)` 等待事件，不做 `EVIOCGRAB` 独占 —— 音量控制仍然可用。

use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const EV_KEY: u16 = 1;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_VOLUMEUP: u16 = 115;

/// input_event 结构体（64 位 Linux）
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    ev_type: u16,
    ev_code: u16,
    ev_value: i32,
}

/// 页面总数（新增页面时只改这里）
pub const PAGE_COUNT: u8 = 3;

static PAGE: AtomicU8 = AtomicU8::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);

/// 屏幕横向朝向：`false` = `Rot90`，`true` = `Rot270`（两者互为 180°）。
///
/// 面板物理上是竖屏（1080×2340），仪表内容横着放，所以有且只有两个横向朝向。
/// 手机往哪边摆就该用哪个 —— 这是**摆位事实**，不是每次开机都要重选的东西，
/// 因此双击切换后会落盘（`rotation.txt`）。放在这里而不是 `screen` 模块：
/// 它和 `PAGE` 一样是「按键驱动的显示状态」，集中在一处才不会两边不同步。
static ROT270: AtomicBool = AtomicBool::new(false);

/// 双击音量上的判定窗口。窗口内出现第二下 = 双击（翻转朝向），
/// 否则窗口过期后按单击处理（下一页）。
///
/// 代价是单击会有这个窗口长度的延迟：不能按下就立刻翻页，
/// 否则双击会顺带多翻一页。300ms 是惯用值，体感上察觉不到。
const DOUBLE_CLICK_MS: u64 = 300;

/// 当前是否为翻转后的横向朝向。
pub fn rot270() -> bool {
    ROT270.load(Ordering::Relaxed)
}

/// 设置朝向（启动时按持久化值初始化，使屏上状态与这里一致）。
pub fn set_rot270(v: bool) {
    ROT270.store(v, Ordering::Relaxed);
}

/// 「音量上」轻触判定：单击翻页，双击翻转朝向，共用一个键。
///
/// 关键约束：**单击必须等双击窗口过期才能兑现**，否则双击的第一下会立刻翻页，
/// 用户看到的是一次双击「又翻页又旋转」。
///
/// 抽成独立状态机而不是写在监听循环里，是因为这类手势逻辑最容易悄悄坏掉
/// （少等一次窗口、窗口过期分支被 `continue` 跳过都会让单击永远不生效），
/// 而它在循环里没法单测。时间由调用方传入，测试可以随便造。
#[derive(Default)]
struct TapTracker {
    pending: Option<Instant>,
}

impl TapTracker {
    /// 记一次按下。返回 `true` 表示这是双击的第二下（调用方应翻转朝向）。
    fn tap(&mut self, now: Instant) -> bool {
        if self.pending.take().is_some() {
            true
        } else {
            self.pending = Some(now);
            false
        }
    }

    /// 双击窗口是否已过期；过期即清空并返回 `true`（调用方据此兑现单击翻页）。
    fn expired(&mut self, now: Instant) -> bool {
        match self.pending {
            Some(t) if now.saturating_duration_since(t) >= Duration::from_millis(DOUBLE_CLICK_MS) => {
                self.pending = None;
                true
            }
            _ => false,
        }
    }
}

/// 翻页方向随屏幕朝向翻转。
///
/// 面板换了 180° 等于手机在用户手里也转了 180°，物理音量键相对屏上内容
/// 换到了另一侧 —— 方向不跟着翻，用户转到另一个朝向后按键就是反的。
/// 所以这里不能让「音量上 = 下一页」写死。
fn page_delta(base: i32) -> i32 {
    if rot270() {
        -base
    } else {
        base
    }
}

/// 当前朝向下「音量上 / 音量下」各自对应的翻页方向文案。
///
/// 日志与屏上页脚都必须取这一处，别在两处各写一套判断：
/// 朝向一变两处不同步，屏上就会指着按键写错方向，比不写还糟。
pub fn up_down_labels() -> (&'static str, &'static str) {
    if rot270() {
        ("上一页", "下一页")
    } else {
        ("下一页", "上一页")
    }
}

/// 翻转横向朝向并落盘。
fn toggle_rotation() {
    let next = !ROT270.load(Ordering::Relaxed);
    ROT270.store(next, Ordering::Relaxed);
    let name = if next { "rot270" } else { "rot90" };
    match crate::store::settings::save_rotation(name) {
        Ok(()) => tracing::info!("hotkeys: KEY_VOLUMEUP 双击 → 屏幕朝向翻转为 {name}"),
        Err(e) => tracing::warn!("hotkeys: 屏幕朝向已翻转但保存失败: {e}"),
    }
}

/// 当前页面索引（0 起）。
pub fn page() -> u8 {
    PAGE.load(Ordering::Relaxed)
}

/// 直接设置页面（API/调试用）。
pub fn set_page(p: u8) {
    PAGE.store(p % PAGE_COUNT, Ordering::Relaxed);
}

/// 翻页：`delta` 为 +1（下一页）/ -1（上一页），循环。
fn step(delta: i32) {
    let cur = PAGE.load(Ordering::Relaxed) as i32;
    let next = (cur + delta).rem_euclid(PAGE_COUNT as i32) as u8;
    PAGE.store(next, Ordering::Relaxed);
}

/// 仅原子改页（async-signal-safe，供信号处理器调用）。
fn step_atomic(delta: i32) {
    let cur = PAGE.load(Ordering::Relaxed) as i32;
    PAGE.store((cur + delta).rem_euclid(PAGE_COUNT as i32) as u8, Ordering::Relaxed);
}

extern "C" fn on_usr1(_: libc::c_int) {
    step_atomic(1);
}

extern "C" fn on_usr2(_: libc::c_int) {
    step_atomic(-1);
}

/// 安装调试信号：`SIGUSR1` 下一页、`SIGUSR2` 上一页（无物理键时验证与远程翻页用）。
pub fn install_debug_signals() {
    let h1: extern "C" fn(libc::c_int) = on_usr1;
    let h2: extern "C" fn(libc::c_int) = on_usr2;
    unsafe {
        libc::signal(libc::SIGUSR1, h1 as usize as libc::sighandler_t);
        libc::signal(libc::SIGUSR2, h2 as usize as libc::sighandler_t);
    }
}

/// 解析 `capabilities/key` 的十六进制位图，判断是否包含指定按键码。
///
/// 内核按「高位字在前」打印，故最后一个字是 bit 0-63。
fn supports(bitmap_hex: &str, code: u16) -> bool {
    let words: Vec<&str> = bitmap_hex.split_whitespace().collect();
    let idx = (code / 64) as usize;
    let bit = (code % 64) as u32;
    let Some(word) = words.get(words.len().wrapping_sub(1 + idx)) else {
        return false;
    };
    u64::from_str_radix(word, 16)
        .map(|v| v & (1u64 << bit) != 0)
        .unwrap_or(false)
}

struct KeyDev {
    path: String,
    name: String,
    up: bool,
    down: bool,
}

fn find_volume_devices() -> Vec<KeyDev> {
    let mut out = Vec::new();
    for i in 0..32 {
        let sys = format!("/sys/class/input/event{i}");
        if !std::path::Path::new(&sys).exists() {
            continue;
        }
        let cap = std::fs::read_to_string(format!("{sys}/device/capabilities/key")).unwrap_or_default();
        if cap.trim().is_empty() {
            continue;
        }
        let up = supports(&cap, KEY_VOLUMEUP);
        let down = supports(&cap, KEY_VOLUMEDOWN);
        if !up && !down {
            continue;
        }
        let name = std::fs::read_to_string(format!("{sys}/device/name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        // 耳机孔/手柄等设备会把耳机线控按键映射成音量键，容易产生假事件 → 跳过
        let lower = name.to_lowercase();
        if lower.contains("headset") || lower.contains("jack") || lower.contains("hdmi") {
            tracing::debug!("hotkeys: 跳过伪音量键设备 {name} ({sys})");
            continue;
        }
        out.push(KeyDev { path: format!("/dev/input/event{i}"), name, up, down });
    }
    out
}

/// 启动音量键监听线程（重复调用安全）。
pub fn start_listener() {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    thread::Builder::new()
        .name("hotkeys".into())
        .spawn(move || {
            let mut devs = Vec::new();
            for _ in 0..30 {
                devs = find_volume_devices();
                if !devs.is_empty() {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
            if devs.is_empty() {
                tracing::warn!("hotkeys: 未发现音量键设备，页面切换不可用");
                return;
            }

            let mut fds: Vec<(File, i32, bool, bool)> = Vec::new();
            for d in &devs {
                match File::open(&d.path) {
                    Ok(f) => {
                        unsafe { libc::fcntl(f.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
                        tracing::info!(
                            "hotkeys: 监听 {} ({}) 音量上={} 音量下={}",
                            d.name,
                            d.path,
                            d.up,
                            d.down
                        );
                        let raw = f.as_raw_fd();
                        let (up, down) = (d.up, d.down);
                        fds.push((f, raw, up, down));
                    }
                    Err(e) => tracing::warn!("hotkeys: 无法打开 {}: {e}", d.path),
                }
            }
            if fds.is_empty() {
                return;
            }

            let mut taps = TapTracker::default();
            loop {
                let mut pfds: Vec<libc::pollfd> = fds
                    .iter()
                    .map(|(_, fd, _, _)| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 })
                    .collect();
                // 超时 50ms：既要在双击窗口（300ms）内及时收第二下，也要在窗口
                // 过期后尽快把待定的单击兑现。注意超时返回 0 时不能 `continue`，
                // 否则待定单击永远等不到兑现。
                let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 50) };
                for (i, p) in (0..if n > 0 { pfds.len() } else { 0 }).map(|i| (i, &pfds[i])) {
                    if p.revents & libc::POLLIN == 0 {
                        continue;
                    }
                    let (file, _, up, down) = &mut fds[i];
                    let (up, down) = (*up, *down);
                    loop {
                        let mut buf = [0u8; std::mem::size_of::<InputEvent>()];
                        match file.read_exact(&mut buf) {
                            Ok(()) => {
                                let ev: InputEvent = unsafe { std::mem::transmute(buf) };
                                if ev.ev_type == EV_KEY && ev.ev_value == 1 {
                                    // 音量上 = 往右（下一页）。方向必须和顶栏页签的
                                    // 左右顺序一致：页签是 系统监控 → Token用量 → 时钟
                                    // 从左往右排的，按键却让「上」往左走，用起来就是反的。
                                    if ev.ev_code == KEY_VOLUMEUP && up {
                                        if taps.tap(Instant::now()) {
                                            toggle_rotation();
                                        }
                                    } else if ev.ev_code == KEY_VOLUMEDOWN && down {
                                        let (_, label) = up_down_labels();
                                        tracing::info!("hotkeys: KEY_VOLUMEDOWN → {label}");
                                        step(page_delta(-1));
                                    }
                                }
                            }
                            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(e) => {
                                tracing::warn!("hotkeys: 读取失败: {e}");
                                break;
                            }
                        }
                    }
                }

                // 双击窗口过期 → 那一下就是单击：翻到下一页
                if taps.expired(Instant::now()) {
                    let (label, _) = up_down_labels();
                    tracing::info!("hotkeys: KEY_VOLUMEUP → {label}");
                    step(page_delta(1));
                }
            }
        })
        .expect("无法启动 hotkeys 线程");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    /// 单击：窗口内没有第二下 → 窗口过期后兑现，且只兑现一次。
    #[test]
    fn 单击在窗口过期后兑现一次() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        assert!(!tr.tap(t), "第一下是单击的候选，不是双击");
        assert!(!tr.expired(at(t, 100)), "窗口内不该兑现");
        assert!(tr.expired(at(t, DOUBLE_CLICK_MS)), "窗口到点应兑现");
        assert!(!tr.expired(at(t, 5000)), "兑现后不该重复兑现");
    }

    /// 双击：窗口内的第二下判为双击，且**不再兑现单击**（否则双击会顺带翻一页）。
    #[test]
    fn 双击不触发翻页() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        assert!(!tr.tap(t));
        assert!(tr.tap(at(t, 150)), "窗口内的第二下应为双击");
        assert!(!tr.expired(at(t, 1000)), "双击后不该再兑现单击");
    }

    /// 窗口外连按两下 = 两次单击（翻两页），不能误判成双击。
    #[test]
    fn 窗口外连按判为两次单击() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        assert!(!tr.tap(t));
        assert!(tr.expired(at(t, DOUBLE_CLICK_MS)), "第一下先兑现");
        assert!(!tr.tap(at(t, 400)), "第二下是新的单击候选");
        assert!(tr.expired(at(t, 400 + DOUBLE_CLICK_MS)));
    }

    /// 双击之后紧接着的单击仍是独立一次（不该被双击状态吃掉）。
    #[test]
    fn 双击后的单击仍然有效() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        tr.tap(t);
        assert!(tr.tap(at(t, 120)));
        assert!(!tr.tap(at(t, 300)), "双击后的第一下是新候选");
        assert!(tr.expired(at(t, 300 + DOUBLE_CLICK_MS)));
    }

    /// 方向必须随朝向翻转：屏幕转 180° 后，同一个键对应的翻页方向要对调，
    /// 否则用户转到另一个朝向按键就是反的。
    #[test]
    fn 翻页方向随朝向翻转() {
        set_rot270(false);
        assert_eq!(page_delta(1), 1, "rot90：音量上应是下一页");
        assert_eq!(page_delta(-1), -1);
        assert_eq!(up_down_labels(), ("下一页", "上一页"));

        set_rot270(true);
        assert_eq!(page_delta(1), -1, "rot270：音量上应对调为上一页");
        assert_eq!(page_delta(-1), 1);
        assert_eq!(up_down_labels(), ("上一页", "下一页"));

        set_rot270(false);
    }

    /// 朝向翻转状态：set/rot270 往返一致。
    #[test]
    fn 朝向状态往返一致() {
        set_rot270(false);
        assert!(!rot270());
        set_rot270(true);
        assert!(rot270());
        set_rot270(false);
    }
}
