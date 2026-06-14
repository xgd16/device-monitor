//! 振动马达测试工具
//!
//! 独立二进制，通过 Linux input 子系统的 FF_RUMBLE ioctl 直接驱动振动马达。
//! 用法：`test_vibrate [duration_ms]`（默认 500ms）
//!
//! 依赖设备节点 `/dev/input/event3`，需 root 权限。

use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;

extern "C" {
    fn ioctl(fd: i32, request: u32, ...) -> i32;
}

fn main() {
    let dur: u16 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    println!("vibrating {}ms...", dur);

    let mut file = fs::OpenOptions::new()
        .write(true)
        .open("/dev/input/event3")
        .expect("open event3");
    let fd = file.as_raw_fd();
    println!("fd={}", fd);

    // 构造 FF_RUMBLE 效果结构体（48 字节）
    let mut buf = [0u8; 48];
    buf[0..2].copy_from_slice(&0x50u16.to_le_bytes());       // type = FF_RUMBLE
    buf[2..4].copy_from_slice(&(-1i16).to_le_bytes());       // id = -1（新建）
    buf[10..12].copy_from_slice(&dur.to_le_bytes());           // u.rumble.strong_magnitude duration
    buf[14..16].copy_from_slice(&0x7FFFu16.to_le_bytes());    // strong_magnitude
    buf[16..18].copy_from_slice(&0x7FFFu16.to_le_bytes());    // weak_magnitude

    // EVIOCSFF = _IOW('E', 0x80, sizeof(struct ff_effect))
    let evio_csff: u32 = (1 << 30) | (48 << 16) | (0x45 << 8) | 0x80;
    println!("EVIOCSFF=0x{:08X}", evio_csff);

    let ret = unsafe { ioctl(fd, evio_csff, buf.as_mut_ptr()) };
    println!("ioctl ret={}", ret);
    if ret < 0 {
        eprintln!("ioctl failed: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }

    let eid = i16::from_le_bytes([buf[2], buf[3]]);
    println!("effect_id={}", eid);

    // 构造 EV_FF 事件触发振动
    let mut event = [0u8; 24];
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    event[0..8].copy_from_slice(&(ts.as_secs() as i64).to_le_bytes());       // timeval.tv_sec
    event[8..16].copy_from_slice(&(ts.subsec_micros() as i64).to_le_bytes()); // timeval.tv_usec
    event[16..18].copy_from_slice(&0x15u16.to_le_bytes());                    // EV_FF
    event[18..20].copy_from_slice(&eid.to_le_bytes());                        // effect id
    event[20..24].copy_from_slice(&1i32.to_le_bytes());                       // value = 1 (play)

    println!("writing event...");
    file.write_all(&event).expect("write event");
    std::thread::sleep(std::time::Duration::from_millis(dur as u64 + 100));
    println!("done - phone should have vibrated");
}
