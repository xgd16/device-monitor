//! 电源键监听模块
//!
//! 后台线程读取 `/dev/input/event0`（pm8941_pwrkey），
//! 按下电源键时切换屏幕背光开关。

use std::fs::File;
use std::io::Read;
use std::mem;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use super::hardware;

/// 电源键 input_event 结构体
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    ev_type: u16,
    ev_code: u16,
    ev_value: i32,
}

const EV_KEY: u16 = 1;
const KEY_POWER: u16 = 116;

/// 屏幕是否亮着（电源键切换用）
static SCREEN_ON: AtomicBool = AtomicBool::new(true);

/// 电源键监听线程是否已启动
static LISTENER_STARTED: AtomicBool = AtomicBool::new(false);

/// 获取当前屏幕状态
pub fn is_screen_on() -> bool {
    SCREEN_ON.load(Ordering::Relaxed)
}

/// 更新屏幕状态（API 调用时同步）
pub fn update_screen_state(on: bool) {
    SCREEN_ON.store(on, Ordering::Relaxed);
}

/// 启动电源键监听后台线程。
/// 重复调用安全，只会启动一个线程。
pub fn start_listener() {
    if LISTENER_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }

    thread::Builder::new()
        .name("power-key".into())
        .spawn(move || {
            // 等待输入设备就绪
            for _ in 0..30 {
                if std::path::Path::new("/dev/input/event0").exists() {
                    break;
                }
                thread::sleep(std::time::Duration::from_secs(1));
            }

            let file = match File::open("/dev/input/event0") {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("power-key: 无法打开 /dev/input/event0: {e}");
                    return;
                }
            };

            // 设置非阻塞
            let fd = file.as_raw_fd();
            unsafe {
                libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            }

            let mut buf = [0u8; mem::size_of::<InputEvent>()];
            let mut file = file;

            // 同步初始屏幕状态
            let bl_power: u32 = std::fs::read_to_string("/sys/class/backlight/ae94000.dsi.0/bl_power")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            SCREEN_ON.store(bl_power == 0, Ordering::Relaxed);

            println!("power-key: 监听已启动");

            loop {
                match file.read_exact(&mut buf) {
                    Ok(()) => {
                        let ev: InputEvent = unsafe { mem::transmute(buf) };
                        if ev.ev_type == EV_KEY && ev.ev_code == KEY_POWER && ev.ev_value == 1 {
                            // 电源键按下：切换屏幕
                            let was_on = SCREEN_ON.load(Ordering::Relaxed);
                            let new_state = !was_on;
                            match hardware::set_screen_power(new_state) {
                                Ok(()) => {
                                    SCREEN_ON.store(new_state, Ordering::Relaxed);
                                    println!("power-key: 屏幕{}", if new_state { "亮" } else { "灭" });
                                }
                                Err(e) => eprintln!("power-key: 切换屏幕失败: {e}"),
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // 没有事件，短暂休眠避免空转
                        thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(e) => {
                        eprintln!("power-key: 读取错误: {e}");
                        thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        })
        .expect("无法启动 power-key 线程");
}
