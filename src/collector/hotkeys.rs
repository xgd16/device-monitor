//! 音量键监听：双击音量上翻转物理屏横向朝向（落盘 `rotation.txt`，面板下次启动按新朝向渲染）。
//!
//! 不硬编码 event 编号：通过 `/sys/class/input/eventN/device/capabilities/key`
//! 的 KEY 位图自动发现支持 KEY_VOLUMEUP(115) / KEY_VOLUMEDOWN(114) 的设备。
//! 用 `poll(2)` 等待事件，不做 `EVIOCGRAB` 独占 —— 音量控制仍然可用。
//!
//! 历史：单击翻页 / 双击切主题曾服务旧 DRM 直绘屏（`src/screen/`），该渲染器
//! 2026-09-27 已删除，面板（`panel/`）的交互完全走触控，这里的翻页/主题动作
//! 随之移除；朝向翻转保留 —— `rotation.txt` 是启动器与面板的朝向来源。

use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
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

static STARTED: AtomicBool = AtomicBool::new(false);

/// 屏幕横向朝向：`false` = `Rot90`，`true` = `Rot270`（两者互为 180°）。
///
/// 面板物理上是竖屏（1080×2340），仪表内容横着放，所以有且只有两个横向朝向。
/// 手机往哪边摆就该用哪个 —— 这是**摆位事实**，不是每次开机都要重选的东西，
/// 因此双击切换后会落盘（`rotation.txt`），由启动器在拉起面板时读取。
static ROT270: AtomicBool = AtomicBool::new(false);

/// 双击音量上的判定窗口。窗口内出现第二下 = 双击（翻转朝向）。
/// 单击不绑定动作，但必须等窗口过期才能确认「不是双击」—— 状态机保留是为双击判定。
const DOUBLE_CLICK_MS: u64 = 300;

/// 当前是否为翻转后的横向朝向。
pub fn rot270() -> bool {
    ROT270.load(Ordering::Relaxed)
}

/// 设置朝向（启动时按持久化值初始化，使对外状态与落盘一致）。
pub fn set_rot270(v: bool) {
    ROT270.store(v, Ordering::Relaxed);
}

/// 翻转横向朝向并落盘。面板重启（或服务重启触发启动器）后按新朝向渲染。
fn toggle_rotation() {
    let next = !ROT270.load(Ordering::Relaxed);
    ROT270.store(next, Ordering::Relaxed);
    let name = if next { "rot270" } else { "rot90" };
    match crate::store::settings::save_rotation(name) {
        Ok(()) => tracing::info!("hotkeys: KEY_VOLUMEUP 双击 → 屏幕朝向翻转为 {name}（面板下次启动生效）"),
        Err(e) => tracing::warn!("hotkeys: 屏幕朝向已翻转但保存失败: {e}"),
    }
}

/// 「音量上」轻触判定：只区分单击与双击。
///
/// 关键约束：**单击必须等双击窗口过期才能确认**，否则双击的第一下会被当成单击
/// （历史上单击=翻页，误判会让双击顺带多翻一页；现在单击虽无动作，状态机语义不变）。
///
/// 抽成独立状态机而不是写在监听循环里，是因为这类手势逻辑最容易悄悄坏掉
/// （少等一次窗口、窗口过期分支被 `continue` 跳过都会让单击/双击判定错位），
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

    /// 双击窗口是否已过期；过期即清空并返回 `true`（单击无动作，调用方只需清状态）。
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
    u64::from_str_radix(word, 16).map(|v| v & (1u64 << bit) != 0).unwrap_or(false)
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
        let name = std::fs::read_to_string(format!("{sys}/device/name")).unwrap_or_default().trim().to_string();
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
                tracing::warn!("hotkeys: 未发现音量键设备，朝向切换不可用");
                return;
            }

            let mut fds: Vec<(File, i32, bool, bool)> = Vec::new();
            for d in &devs {
                match File::open(&d.path) {
                    Ok(f) => {
                        unsafe { libc::fcntl(f.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
                        tracing::info!("hotkeys: 监听 {} ({}) 音量上={} 音量下={}", d.name, d.path, d.up, d.down);
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

            let mut taps_up = TapTracker::default();
            loop {
                let mut pfds: Vec<libc::pollfd> = fds
                    .iter()
                    .map(|(_, fd, _, _)| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 })
                    .collect();
                // 超时 50ms：既要在双击窗口（300ms）内及时收第二下，也要在窗口
                // 过期后尽快确认单击。注意超时返回 0 时不能 `continue`，
                // 否则待定单击永远等不到窗口过期判定。
                let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 50) };
                let mut dead: Vec<usize> = Vec::new();
                if n > 0 {
                    for i in 0..pfds.len() {
                        let revents = pfds[i].revents;
                        // 设备消失（uinput 虚拟设备被销毁、USB 拔出等）时 poll 会
                        // 立即返回且带 POLLERR/POLLHUP —— 不移除就变成永久忙轮询
                        if revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                            dead.push(i);
                            continue;
                        }
                        if revents & libc::POLLIN == 0 {
                            continue;
                        }
                        let (file, _, up, _down) = &mut fds[i];
                        let up = *up;
                        let mut fatal = false;
                        loop {
                            let mut buf = [0u8; std::mem::size_of::<InputEvent>()];
                            match file.read_exact(&mut buf) {
                                Ok(()) => {
                                    let ev: InputEvent = unsafe { std::mem::transmute(buf) };
                                    // 双击音量上 = 翻转朝向（落盘，面板下次启动生效）；
                                    // 音量下与单击均无绑定动作（历史：翻页/主题切换，
                                    // 随旧 DRM 渲染器 2026-09-27 移除）。
                                    if ev.ev_type == EV_KEY
                                        && ev.ev_value == 1
                                        && ev.ev_code == KEY_VOLUMEUP
                                        && up
                                        && taps_up.tap(Instant::now())
                                    {
                                        toggle_rotation();
                                    }
                                }
                                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                                Err(e) => {
                                    tracing::warn!("hotkeys: 读取失败: {e}");
                                    fatal = true;
                                    break;
                                }
                            }
                        }
                        if fatal {
                            dead.push(i);
                        }
                    }
                }
                // 失效设备移出监听（倒序 remove 防下标漂移）；一个都不剩就收工
                let had_dead = !dead.is_empty();
                for i in dead.into_iter().rev() {
                    let fd = fds[i].1;
                    tracing::warn!("hotkeys: 按键设备 fd={fd} 失效，移出监听");
                    fds.remove(i);
                }
                if had_dead && fds.is_empty() {
                    tracing::warn!("hotkeys: 所有按键设备已失效，监听线程退出");
                    return;
                }

                // 双击窗口过期 → 那一下就是单击：仅清空待定状态（无绑定动作）
                if taps_up.expired(Instant::now()) {
                    tracing::debug!("hotkeys: KEY_VOLUMEUP 单击（无绑定动作）");
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

    /// 单击：窗口内没有第二下 → 窗口过期后清空，且只清一次。
    #[test]
    fn 单击在窗口过期后清空() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        assert!(!tr.tap(t), "第一下是单击的候选，不是双击");
        assert!(!tr.expired(at(t, 100)), "窗口内不该清空");
        assert!(tr.expired(at(t, DOUBLE_CLICK_MS)), "窗口到点应清空");
        assert!(!tr.expired(at(t, 5000)), "清空后不该重复清空");
    }

    /// 双击：窗口内的第二下判为双击，且**不再按单击清空**（否则双击会被拆成单击+双击）。
    #[test]
    fn 双击后不再按单击清空() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        assert!(!tr.tap(t));
        assert!(tr.tap(at(t, 150)), "窗口内的第二下应为双击");
        assert!(!tr.expired(at(t, 1000)), "双击后不该再清空单击状态");
    }

    /// 窗口外连按两下 = 两次单击，不能误判成双击。
    #[test]
    fn 窗口外连按判为两次单击() {
        let t = Instant::now();
        let mut tr = TapTracker::default();
        assert!(!tr.tap(t));
        assert!(tr.expired(at(t, DOUBLE_CLICK_MS)), "第一下先清空");
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
