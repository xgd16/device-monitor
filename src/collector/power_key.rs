//! 电源键监听模块
//!
//! 后台线程读取 `/dev/input/event0`（pm8941_pwrkey），
//! 支持单击和双击检测，触发不同动作。

use std::fs::File;
use std::io::Read;
use std::mem;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

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

/// 双击时间窗口（毫秒）
const DOUBLE_CLICK_MS: u128 = 400;

/// 屏幕是否亮着（电源键切换用）
static SCREEN_ON: AtomicBool = AtomicBool::new(true);

/// 闪光灯是否亮着
static FLASHLIGHT_ON: AtomicBool = AtomicBool::new(false);

/// 电源键监听线程是否已启动
static LISTENER_STARTED: AtomicBool = AtomicBool::new(false);

/// 单击动作：切换屏幕
fn on_single_click() {
    let was_on = SCREEN_ON.load(Ordering::Relaxed);
    let new_state = !was_on;
    match hardware::set_screen_power(new_state) {
        Ok(()) => {
            SCREEN_ON.store(new_state, Ordering::Relaxed);
            println!("power-key: 单击 → 屏幕{}", if new_state { "亮" } else { "灭" });
        }
        Err(e) => eprintln!("power-key: 切换屏幕失败: {e}"),
    }
}

/// 双击动作：切换所有闪光灯
fn on_double_click() {
    let new_state = !FLASHLIGHT_ON.load(Ordering::Relaxed);
    match hardware::set_flashlight("white", new_state) {
        Ok(()) => {}
        Err(e) => eprintln!("power-key: 白色闪光灯失败: {e}"),
    }
    match hardware::set_flashlight("yellow", new_state) {
        Ok(()) => {}
        Err(e) => eprintln!("power-key: 黄色闪光灯失败: {e}"),
    }
    FLASHLIGHT_ON.store(new_state, Ordering::Relaxed);
    println!("power-key: 双击 → 闪光灯{}", if new_state { "开" } else { "关" });
}

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
            let bl_power: u32 = std::fs::read_to_string(
                "/sys/class/backlight/ae94000.dsi.0/bl_power",
            )
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
            SCREEN_ON.store(bl_power == 0, Ordering::Relaxed);

            // 双击检测状态
            let mut pending_single: Option<Instant> = None;

            println!("power-key: 监听已启动（支持单击/双击）");

            loop {
                // 检查是否有待处理的单击已超时
                if let Some(press_time) = pending_single {
                    if press_time.elapsed().as_millis() >= DOUBLE_CLICK_MS {
                        // 超时，确认为单击
                        pending_single = None;
                        on_single_click();
                    }
                }

                match file.read_exact(&mut buf) {
                    Ok(()) => {
                        let ev: InputEvent = unsafe { mem::transmute(buf) };
                        if ev.ev_type == EV_KEY
                            && ev.ev_code == KEY_POWER
                            && ev.ev_value == 1
                        {
                            // 电源键按下
                            let now = Instant::now();

                            if let Some(prev) = pending_single.take() {
                                let elapsed = now.duration_since(prev).as_millis();
                                if elapsed < DOUBLE_CLICK_MS {
                                    // 双击！
                                    on_double_click();
                                    continue;
                                }
                            }

                            // 记录按下时间，等下一次
                            pending_single = Some(now);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // 没有事件，短暂休眠避免空转
                        thread::sleep(std::time::Duration::from_millis(20));
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
