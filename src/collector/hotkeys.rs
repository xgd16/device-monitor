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
use std::time::Duration;

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

            loop {
                let mut pfds: Vec<libc::pollfd> = fds
                    .iter()
                    .map(|(_, fd, _, _)| libc::pollfd { fd: *fd, events: libc::POLLIN, revents: 0 })
                    .collect();
                let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, 500) };
                if n <= 0 {
                    continue;
                }
                for (i, p) in pfds.iter().enumerate() {
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
                                    if ev.ev_code == KEY_VOLUMEUP && up {
                                        tracing::info!("hotkeys: KEY_VOLUMEUP → 上一页");
                                        step(-1);
                                    } else if ev.ev_code == KEY_VOLUMEDOWN && down {
                                        tracing::info!("hotkeys: KEY_VOLUMEDOWN → 下一页");
                                        step(1);
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
            }
        })
        .expect("无法启动 hotkeys 线程");
}
