//! 用户态直驱 NXP PN80T 的 UID 卡模拟（T4T NDEF 卡）
//!
//! 内核 NFC 核心没有卡模拟接口（无 `NFC_CMD_TARGET_ADD`，也无法配置监听模式的
//! `LA_NFCID1`），因此卡模拟绕过内核驱动，直接与芯片对话：
//!
//! 1. 停轮询 + `DEV_DOWN`（由 API 层先做，避免 remove 与在途 I2C 撞车挂死总线）；
//! 2. `echo 3-0028 > /sys/bus/i2c/drivers/nxp-nci_i2c/unbind` 解绑内核驱动；
//! 3. GPIO v2 拉高 EN（tlmm gpiochip3 line 128，来自 DT `enable-gpios`）给芯片上电；
//! 4. `/dev/i2c-3` 直说 NCI（`ioctl(I2C_SLAVE, 0x28)` 后裸 NCI 帧，与内核
//!    `nxp-nci_i2c` 线路格式一致）：CORE_RESET → CORE_INIT → CORE_SET_CONFIG
//!    （`LA_NFCID1=0x33` 自定义 UID）→ RF_SET_LISTEN_MODE_ROUTING（ISO-DEP 路由
//!    到主机，**必发**，否则读卡器贴近毫无反应）→ RF_DISCOVER（Listen-A）；
//! 5. ISO-DEP 激活后应答 T4T APDU（状态机移植自 NXP
//!    `linux_libnfc-nci_examples/ndef-emulation_example`）；
//! 6. 停止时 RF_DEACTIVATE → EN 拉低 → 重新 bind 回内核驱动。
//!
//! NCI 协议坑（实测）：`CORE_SET_CONFIG_CMD` 负载是 **[参数个数][TLV…]**，
//! 少写字节会得到 `0x05 SYNTAX_ERROR`；芯片空闲时首个写会被 NACK，110ms 后重试。

// 纯逻辑（NCI 组包 / T4T 状态机）只被 Linux 运行时使用；macOS 本地构建会告警
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::OnceLock;

use serde::Serialize;

use super::{EventRecord, lock};

// ───────────────────────── 对外数据结构 ─────────────────────────

/// 卡模拟状态快照
#[derive(Debug, Clone, Serialize, Default)]
pub struct EmulStatus {
    /// 模拟线程是否在运行
    pub running: bool,
    /// 读卡器是否已激活（正在读）
    pub active: bool,
    /// 模拟的 UID（冒号分隔）
    pub uid: String,
    /// NDEF 记录 hex
    pub ndef_hex: String,
    pub started_at: Option<String>,
    pub last_error: Option<String>,
}

/// 模拟请求
#[derive(Debug, Clone, Default)]
pub struct EmulReq {
    /// UID：4/7/10 字节 hex，可带冒号
    pub uid: String,
    /// "text" | "uri" | "raw" | "message"（message = 整条 NDEF 消息 hex 逐字节克隆）
    pub kind: String,
    pub value: String,
    pub lang: String,
    pub raw_hex: String,
}

// ───────────────────────── 纯逻辑（跨平台可测） ─────────────────────────

/// 解析 UID hex（允许冒号/空格分隔），长度必须是 4/7/10 字节
pub fn parse_uid(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() % 2 != 0 {
        return Err(format!("UID hex 长度非法：{s}"));
    }
    let bytes: Vec<u8> = (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect::<Result<_, _>>()?;
    match bytes.len() {
        4 | 7 | 10 => Ok(bytes),
        n => Err(format!("UID 必须是 4/7/10 字节（当前 {n} 字节）")),
    }
}

/// 组 `CORE_SET_CONFIG_CMD`：负载 = [参数个数][ID Len Value]…
pub fn core_set_config_frame(tlvs: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(1 + tlvs.len() * 2);
    payload.push(tlvs.len() as u8);
    for (id, val) in tlvs {
        payload.push(*id);
        payload.push(val.len() as u8);
        payload.extend_from_slice(val);
    }
    let mut frame = vec![0x20, 0x02, payload.len() as u8];
    frame.extend_from_slice(&payload);
    frame
}

/// `RF_SET_LISTEN_MODE_ROUTING_CMD`：把 ISO-DEP 协议路由到主机（DH），
/// 负载 = [more][条目数][条目…]，条目 = Type | Len | Route | Power | Proto（NXP 实测布局）
pub fn listen_routing_frame() -> Vec<u8> {
    vec![
        0x21, 0x01, // RF_SET_LISTEN_MODE_ROUTING
        0x07, // plen
        0x00, // more = 最后一组
        0x01, // 1 条路由
        0x01, 0x03, 0x00, 0x01, 0x04, // PROTO | len=3 | DH | PWR_ON | ISO-DEP
    ]
}

/// 判断一帧是否为指定命令的 RSP。
/// NCI 响应首字节 = `0x40 | GID`：CORE 组是 `0x40`，RF 管理组是 `0x41`
/// （实测教训：只认 0x40 会把 RF_DISCOVER 的响应当无关帧丢掉，2 秒后误报超时）。
pub fn is_rsp(frame: &[u8], rsp_hdr: u8, rsp_oid: u8) -> bool {
    frame.len() >= 3 && frame[0] == rsp_hdr && frame[1] == rsp_oid
}

/// T4T 卡模拟状态机的阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum T4tState {
    Ready,
    App,
    Cc,
    Ndef,
}

/// T4T NDEF 卡模拟状态机（NXP 官方示例移植）
pub struct T4tEmu {
    state: T4tState,
    /// NDEF 文件 = NLEN(2 大端) + 记录
    file: Vec<u8>,
}

/// Capability Container（15 字节）：CLEN=0x000F, v0x20, MLe/MLc=0x00FF,
/// NDEF 文件控制 TLV：文件 0xE104，最大 0x00FF，可读可写
const CC_FILE: [u8; 15] = [0x00, 0x0F, 0x20, 0x00, 0xFF, 0x00, 0xFF, 0x04, 0x06, 0xE1, 0x04, 0x00, 0xFF, 0x00, 0xFF];

const SW_OK: [u8; 2] = [0x90, 0x00];
const SW_NOT_FOUND: [u8; 2] = [0x6A, 0x82];
const SW_WRONG_P1P2: [u8; 2] = [0x6B, 0x00];
const SW_SECURITY: [u8; 2] = [0x69, 0x82];
const SW_INS_NOT_SUPPORTED: [u8; 2] = [0x6D, 0x00];

impl T4tEmu {
    pub fn new(ndef_record: &[u8]) -> Self {
        let mut file = Vec::with_capacity(2 + ndef_record.len());
        file.extend_from_slice(&((ndef_record.len() as u16).to_be_bytes()));
        file.extend_from_slice(ndef_record);
        Self { state: T4tState::Ready, file }
    }

    pub fn reset(&mut self) {
        self.state = T4tState::Ready;
    }

    /// 处理一条 APDU，返回响应（含状态字）
    pub fn next(&mut self, cmd: &[u8]) -> Vec<u8> {
        if cmd.len() < 4 {
            self.state = T4tState::Ready;
            return SW_NOT_FOUND.to_vec();
        }
        let (ins, p1, p2) = (cmd[1], cmd[2], cmd[3]);
        match ins {
            // SELECT
            0xA4 => {
                let lc = *cmd.get(4).unwrap_or(&0) as usize;
                let data = if cmd.len() >= 5 + lc { &cmd[5..5 + lc] } else { &[][..] };
                if p1 == 0x04 && matches!(data, [0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01]) {
                    self.state = T4tState::App;
                    return SW_OK.to_vec();
                }
                if p1 == 0x00 && matches!(data, [0xE1, 0x03]) {
                    self.state = T4tState::Cc;
                    return SW_OK.to_vec();
                }
                if p1 == 0x00 && matches!(data, [0xE1, 0x04]) {
                    self.state = T4tState::Ndef;
                    return SW_OK.to_vec();
                }
                self.state = T4tState::Ready;
                SW_NOT_FOUND.to_vec()
            }
            // READ BINARY
            0xB0 => {
                let offset = ((p1 as usize) << 8) | p2 as usize;
                let le = match cmd.get(4).copied().unwrap_or(0) {
                    0 => 256,
                    n => n as usize,
                }
                .min(250);
                let src: &[u8] = match self.state {
                    T4tState::Cc => &CC_FILE,
                    T4tState::Ndef => &self.file,
                    _ => {
                        self.state = T4tState::Ready;
                        return SW_NOT_FOUND.to_vec();
                    }
                };
                if offset >= src.len() {
                    return SW_WRONG_P1P2.to_vec();
                }
                let n = (src.len() - offset).min(le);
                let mut rsp = src[offset..offset + n].to_vec();
                rsp.extend_from_slice(&SW_OK);
                rsp
            }
            // UPDATE BINARY：只读模拟
            0xD6 => SW_SECURITY.to_vec(),
            _ => SW_INS_NOT_SUPPORTED.to_vec(),
        }
    }
}

/// 按请求构造 NDEF 记录字节
pub fn build_ndef(req: &EmulReq) -> Result<Vec<u8>, String> {
    let record = match req.kind.as_str() {
        "text" => {
            let lang = if req.lang.is_empty() { "en" } else { req.lang.as_str() };
            super::ndef::text_record(&req.value, lang)
        }
        "uri" => super::ndef::uri_record(&req.value),
        "raw" => {
            let tnf = 0x01u8;
            super::ndef::raw_record(tnf, "T", &req.raw_hex)?
        }
        // 整条 NDEF 消息逐字节克隆（模拟“发现过的卡片”时把读到的内容原样搬过来）
        "message" => {
            if req.raw_hex.trim().is_empty() {
                return Err("克隆内容为空：请填入 NDEF 消息 hex（可用读取结果的「用此内容模拟」带入）".into());
            }
            let data = super::tag::unhex(&req.raw_hex)?;
            super::ndef::parse_message(&data).map_err(|e| format!("NDEF 消息无法解析：{e}"))?;
            data
        }
        other => return Err(format!("未知 kind：{other}（应为 text/uri/raw/message）")),
    };
    // CC 声明 NDEF 文件最大 0x00FF 字节（含 2 字节 NLEN）
    if record.len() + 2 > 0xFF {
        return Err(format!("NDEF 记录过长：{} 字节（上限 253）", record.len()));
    }
    Ok(record)
}

// ───────────────────────── 运行时 ─────────────────────────

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::fs::OpenOptions;
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::io::AsRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const I2C_PATH: &str = "/dev/i2c-3";
    const I2C_CLIENT: &str = "3-0028";
    const I2C_ADDR: u64 = 0x28;
    const I2C_SLAVE: libc::c_int = 0x0703;

    const UNBIND: &str = "/sys/bus/i2c/drivers/nxp-nci_i2c/unbind";
    const BIND: &str = "/sys/bus/i2c/drivers/nxp-nci_i2c/bind";

    /// NFC 芯片电源 EN：tlmm（gpiochip3）line 128，来自 DT `enable-gpios <0x49 0x80 0>`
    const GPIO_CHIP: &str = "/dev/gpiochip3";
    const EN_LINE: u32 = 128;

    // GPIO v2 uAPI（linux/gpio.h，无 v1 时的唯一 ABI）
    const GPIO_V2_LINE_FLAG_OUTPUT: u64 = 1 << 3;
    const GPIO_V2_LINE_ATTR_ID_OUTPUT_VALUES: u32 = 2;

    /// Linux ioctl 号：`_IOWR(type=0xB4, nr, T)`；该平台 `libc::ioctl` 的 request 是 `c_int`
    const fn iowr<T>(nr: u32) -> libc::c_int {
        ((3u32 << 30) | ((std::mem::size_of::<T>() as u32) << 16) | (0xB4 << 8) | nr) as libc::c_int
    }

    #[repr(C)]
    #[derive(Default)]
    struct LineAttribute {
        id: u32,
        padding: u32,
        values: u64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct LineConfigAttribute {
        attr: LineAttribute,
        mask: u64,
    }

    #[repr(C)]
    struct LineConfig {
        flags: u64,
        num_attrs: u32,
        padding: [u32; 5],
        attrs: [LineConfigAttribute; 10],
    }

    impl Default for LineConfig {
        fn default() -> Self {
            Self { flags: 0, num_attrs: 0, padding: [0; 5], attrs: Default::default() }
        }
    }

    #[repr(C)]
    struct LineRequest {
        offsets: [u32; 64],
        consumer: [u8; 32],
        config: LineConfig,
        num_lines: u32,
        event_buffer_size: u32,
        padding: [u32; 5],
        fd: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct LineValues {
        bits: u64,
        mask: u64,
    }

    const GPIO_V2_GET_LINE_IOCTL: libc::c_int = iowr::<LineRequest>(0x07);
    const GPIO_V2_LINE_SET_VALUES_IOCTL: libc::c_int = iowr::<LineValues>(0x0F);

    /// EN 电源线（持有期间保持输出电平）
    struct EnLine {
        fd: i32,
    }

    impl EnLine {
        fn acquire(value: u8) -> Result<Self, String> {
            let chip = std::fs::File::open(GPIO_CHIP).map_err(|e| format!("打开 {GPIO_CHIP} 失败：{e}"))?;
            let mut req: LineRequest = unsafe { std::mem::zeroed() };
            req.offsets[0] = EN_LINE;
            req.num_lines = 1;
            let name = b"nfc-emulate";
            req.consumer[..name.len()].copy_from_slice(name);
            req.config.flags = GPIO_V2_LINE_FLAG_OUTPUT;
            req.config.num_attrs = 1;
            req.config.attrs[0].attr.id = GPIO_V2_LINE_ATTR_ID_OUTPUT_VALUES;
            req.config.attrs[0].attr.values = value as u64;
            req.config.attrs[0].mask = 1;
            let rc = unsafe { libc::ioctl(chip.as_raw_fd(), GPIO_V2_GET_LINE_IOCTL, &mut req) };
            if rc < 0 {
                return Err(format!(
                    "请求 GPIO EN（line {EN_LINE}）失败：{}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { fd: req.fd })
        }

        fn set(&self, value: u8) -> Result<(), String> {
            let mut vals = LineValues { bits: value as u64, mask: 1 };
            let rc = unsafe { libc::ioctl(self.fd, GPIO_V2_LINE_SET_VALUES_IOCTL, &mut vals) };
            if rc < 0 {
                return Err(format!("设置 GPIO EN 失败：{}", std::io::Error::last_os_error()));
            }
            Ok(())
        }
    }

    impl Drop for EnLine {
        fn drop(&mut self) {
            unsafe { libc::close(self.fd) };
        }
    }

    // ── NCI 线路收发 ──

    /// 写一帧；芯片空闲（standby）时首个写会被 NACK，按内核驱动的做法 110ms 后重试
    fn nci_write(dev: &mut std::fs::File, frame: &[u8]) -> Result<(), String> {
        let mut last = String::new();
        for _ in 0..3 {
            match dev.write_all(frame) {
                Ok(()) => {
                    eprintln!("[nfc-emulate] TX {}", super::hex(frame));
                    return Ok(());
                }
                Err(e) => {
                    last = e.to_string();
                    std::thread::sleep(Duration::from_millis(110));
                }
            }
        }
        Err(format!("NCI 写失败：{last}"))
    }

    /// 读一帧（3 字节头 + 负载）；无数据返回 None
    fn nci_read(dev: &mut std::fs::File) -> Option<Vec<u8>> {
        let mut hdr = [0u8; 3];
        match dev.read(&mut hdr) {
            Ok(3) => {}
            _ => return None, // NACK / EAGAIN：芯片暂无数据
        }
        let plen = hdr[2] as usize;
        let mut payload = vec![0u8; plen];
        let mut got = 0;
        while got < plen {
            match dev.read(&mut payload[got..]) {
                Ok(0) => return None,
                Ok(n) => got += n,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        let mut frame = hdr.to_vec();
        frame.extend_from_slice(&payload);
        eprintln!("[nfc-emulate] RX {}", super::hex(&frame));
        Some(frame)
    }

    /// 发命令并等到对应 RSP；期间的 NTF 转给 on_ntf
    /// 发命令等 RSP；`rsp_hdr` 是响应首字节（`0x40 | GID`：CORE=0x40，RF=0x41）
    fn nci_cmd(
        dev: &mut std::fs::File,
        frame: &[u8],
        rsp_hdr: u8,
        rsp_oid: u8,
        timeout: Duration,
        on_ntf: &mut dyn FnMut(&[u8]),
    ) -> Result<Vec<u8>, String> {
        nci_write(dev, frame)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match nci_read(dev) {
                Some(f) if is_rsp(&f, rsp_hdr, rsp_oid) => {
                    return Ok(f);
                }
                Some(f) if f.len() >= 3 && (f[0] & 0xE0) == 0x60 => on_ntf(&f),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        Err("NCI 命令超时未响应".into())
    }

    // ── 服务 ──

    struct Inner {
        running: bool,
        active: bool,
        uid: String,
        ndef_hex: String,
        started_at: Option<String>,
        last_error: Option<String>,
        stop: Option<Arc<AtomicBool>>,
    }

    pub struct EmulateService {
        inner: Mutex<Inner>,
        events: Mutex<Vec<EventRecord>>,
    }

    impl EmulateService {
        pub(super) fn new() -> Self {
            Self {
                inner: Mutex::new(Inner {
                    running: false,
                    active: false,
                    uid: String::new(),
                    ndef_hex: String::new(),
                    started_at: None,
                    last_error: None,
                    stop: None,
                }),
                events: Mutex::new(Vec::new()),
            }
        }

        fn event(&'static self, kind: &str, text: impl Into<String>, detail: serde_json::Value) {
            let mut ev = lock(&self.events);
            ev.push(EventRecord {
                at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                kind: kind.to_string(),
                text: text.into(),
                detail,
            });
            if ev.len() > 200 {
                let drop_n = ev.len() - 200;
                ev.drain(0..drop_n);
            }
        }

        pub fn events(&self, limit: usize) -> Vec<EventRecord> {
            let ev = lock(&self.events);
            ev.iter().rev().take(limit).cloned().collect()
        }

        pub fn status(&self) -> EmulStatus {
            let g = lock(&self.inner);
            EmulStatus {
                running: g.running,
                active: g.active,
                uid: g.uid.clone(),
                ndef_hex: g.ndef_hex.clone(),
                started_at: g.started_at.clone(),
                last_error: g.last_error.clone(),
            }
        }

        /// 开始模拟：调用方（API 层）须先停轮询并 `DEV_DOWN`
        pub fn start(&'static self, req: &EmulReq) -> Result<EmulStatus, String> {
            let uid = parse_uid(&req.uid)?;
            let record = build_ndef(req)?;
            {
                let mut g = lock(&self.inner);
                if g.running {
                    return Err("卡模拟已在运行，请先停止".into());
                }
                let stop = Arc::new(AtomicBool::new(false));
                g.running = true;
                g.active = false;
                g.uid = uid.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");
                g.ndef_hex = record.iter().map(|b| format!("{b:02x}")).collect();
                g.started_at = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
                g.last_error = None;
                g.stop = Some(stop.clone());
                drop(g);
                std::thread::Builder::new()
                    .name("nfc-emulate".into())
                    .spawn(move || run(self, uid, record, stop))
                    .map_err(|e| format!("启动模拟线程失败：{e}"))?;
            }
            Ok(self.status())
        }

        /// 请求停止；线程在 ~200ms 内收尾并 bind 回内核驱动
        pub fn stop(&'static self) -> Result<EmulStatus, String> {
            let g = lock(&self.inner);
            if !g.running {
                return Err("卡模拟未在运行".into());
            }
            if let Some(stop) = &g.stop {
                stop.store(true, Ordering::SeqCst);
            }
            drop(g);
            Ok(self.status())
        }
    }

    fn sysfs_write(path: &str, val: &str) -> std::io::Result<()> {
        std::fs::OpenOptions::new().write(true).open(path)?.write_all(val.as_bytes())
    }

    /// 模拟线程主流程：解绑 → 直驱 NCI → 监听 → 统一收尾交还驱动
    fn run(svc: &'static EmulateService, uid: Vec<u8>, record: Vec<u8>, stop: Arc<AtomicBool>) {
        // 解绑内核驱动（调用方已停轮询并断电，remove 不会与在途 I2C 撞车）
        match sysfs_write(UNBIND, I2C_CLIENT) {
            Ok(()) => svc.event("info", "已解绑内核驱动 nxp-nci_i2c", serde_json::Value::Null),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound || e.raw_os_error() == Some(19) => {
                svc.event("info", "内核驱动未绑定，直接接管芯片", serde_json::Value::Null)
            }
            Err(e) => {
                fail(svc, format!("解绑内核驱动失败：{e}"));
                finish(svc);
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(200));

        // 芯片直驱（en / dev 是本函数局部资源，返回前自动释放）
        let outcome = drive_chip(svc, &uid, &record, &stop);

        // 统一收尾：资源已释放，内核驱动可以安全 bind 回来（probe 要用 EN GPIO）
        let _ = sysfs_write(BIND, I2C_CLIENT);
        match outcome {
            Ok(()) => svc.event("info", "已停止模拟并交还内核驱动", serde_json::Value::Null),
            Err(e) => {
                let mut g = lock(&svc.inner);
                g.last_error = Some(e.clone());
                drop(g);
                svc.event("error", e, serde_json::Value::Null);
            }
        }
        finish(svc);
    }

    /// 直驱芯片：EN 上电 → NCI 初始化 → Listen-A 监听 → APDU 应答 → RF_DEACTIVATE
    fn drive_chip(
        svc: &'static EmulateService,
        uid: &[u8],
        record: &[u8],
        stop: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let _en = EnLine::acquire(1)?;
        std::thread::sleep(Duration::from_millis(150));

        let mut dev = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(I2C_PATH)
            .map_err(|e| format!("打开 {I2C_PATH} 失败：{e}"))?;
        if unsafe { libc::ioctl(dev.as_raw_fd(), I2C_SLAVE, I2C_ADDR as libc::c_ulong) } < 0 {
            return Err(format!("I2C 设从地址失败：{}", std::io::Error::last_os_error()));
        }

        // NCI 初始化 + 监听
        let mut log_ntf = |f: &[u8]| {
            svc.event("nci", format!("NTF {}", super::hex(f)), serde_json::Value::Null);
        };
        let rsp = nci_cmd(
            &mut dev,
            &[0x20, 0x00, 0x01, 0x01],
            0x40,
            0x00,
            Duration::from_secs(2),
            &mut log_ntf,
        )?;
        check_status("CORE_RESET", &rsp)?;
        let rsp = nci_cmd(&mut dev, &[0x20, 0x01, 0x00], 0x40, 0x01, Duration::from_secs(2), &mut log_ntf)?;
        check_status("CORE_INIT", &rsp)?;
        let tlvs = vec![
            (0x30, vec![0x04]),   // LA_BIT_FRAME_SDD
            (0x31, vec![0x00]),   // LA_PLATFORM_CONFIG（T4T）
            (0x32, vec![0x20]),   // LA_SEL_INFO = ISO-DEP
            (0x33, uid.to_vec()), // LA_NFCID1
        ];
        let rsp = nci_cmd(
            &mut dev,
            &core_set_config_frame(&tlvs),
            0x40,
            0x02,
            Duration::from_secs(2),
            &mut log_ntf,
        )?;
        check_status("CORE_SET_CONFIG(LA_NFCID1)", &rsp)?;
        // RF_SET_LISTEN_MODE_ROUTING：ISO-DEP 监听路由到主机（NXP HCE 路径必发，
        // 否则监听激活可能不送 DH）。条目布局：Type=PROTO | Len=3 | Route=DH | Power=ON | Proto=ISO-DEP
        match nci_cmd(
            &mut dev,
            &listen_routing_frame(),
            0x41,
            0x01,
            Duration::from_secs(2),
            &mut log_ntf,
        ) {
            Ok(rsp) => match check_status("RF_SET_LISTEN_MODE_ROUTING", &rsp) {
                Ok(()) => svc.event("nci", "ISO-DEP 监听已路由到主机", serde_json::Value::Null),
                Err(e) => svc.event("nci", format!("{e}（继续尝试监听）"), serde_json::Value::Null),
            },
            Err(e) => svc.event("nci", format!("路由命令{e}（继续尝试监听）"), serde_json::Value::Null),
        }
        let rsp = nci_cmd(
            &mut dev,
            &[0x21, 0x03, 0x03, 0x01, 0x80, 0x01],
            0x41,
            0x03,
            Duration::from_secs(2),
            &mut log_ntf,
        )?;
        check_status("RF_DISCOVER(Listen-A)", &rsp)?;
        svc.event("info", "正在模拟 T4T NDEF 卡（Listen-A）", serde_json::Value::Null);

        // 事件循环：读卡器 APDU ↔ T4T 应答
        let mut t4t = T4tEmu::new(record);
        while !stop.load(Ordering::SeqCst) {
            let frame = match nci_read(&mut dev) {
                Some(f) => f,
                None => {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
            };
            if frame.len() < 3 {
                continue;
            }
            match (frame[0], frame[1]) {
                (0x61, 0x05) => {
                    t4t.reset();
                    set_active(svc, true);
                    svc.event("emulate", "读卡器已激活", serde_json::json!({ "raw": super::hex(&frame) }));
                }
                (0x61, 0x06) => {
                    set_active(svc, false);
                    svc.event("emulate", "读卡器离开，继续监听", serde_json::Value::Null);
                }
                (0x61, _) => {
                    svc.event("nci", format!("NTF {}", super::hex(&frame)), serde_json::Value::Null);
                }
                (h, _) if (h & 0xE0) == 0x00 => {
                    let apdu = &frame[3..];
                    let rsp = t4t.next(apdu);
                    svc.event(
                        "apdu",
                        format!("→ {}\n← {}", super::hex(apdu), super::hex(&rsp)),
                        serde_json::Value::Null,
                    );
                    let mut out = vec![0x00, 0x00, rsp.len() as u8];
                    out.extend_from_slice(&rsp);
                    nci_write(&mut dev, &out)?;
                }
                _ => {}
            }
        }

        // 正常停止：RF_DEACTIVATE（失败不阻塞收尾）
        let _ = nci_write(&mut dev, &[0x21, 0x06, 0x01, 0x00]);
        Ok(())
    }

    fn fail(svc: &'static EmulateService, msg: String) {
        let mut g = lock(&svc.inner);
        g.last_error = Some(msg.clone());
        drop(g);
        svc.event("error", msg, serde_json::Value::Null);
    }

    fn set_active(svc: &'static EmulateService, active: bool) {
        lock(&svc.inner).active = active;
    }

    fn finish(svc: &'static EmulateService) {
        let mut g = lock(&svc.inner);
        g.running = false;
        g.active = false;
        g.stop = None;
    }

    fn check_status(what: &str, rsp: &[u8]) -> Result<(), String> {
        match rsp.get(3).copied() {
            Some(0x00) => Ok(()),
            Some(s) => Err(format!("{what} 失败：NCI 状态 0x{s:02x}")),
            None => Err(format!("{what} 失败：响应过短")),
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;
    use std::sync::Mutex;

    pub struct EmulateService {
        events: Mutex<Vec<EventRecord>>,
    }

    impl EmulateService {
        pub(super) fn new() -> Self {
            Self { events: Mutex::new(Vec::new()) }
        }
        pub fn events(&self, limit: usize) -> Vec<EventRecord> {
            lock(&self.events).iter().rev().take(limit).cloned().collect()
        }
        pub fn status(&self) -> EmulStatus {
            EmulStatus::default()
        }
        pub fn start(&'static self, _req: &EmulReq) -> Result<EmulStatus, String> {
            Err("卡模拟仅支持 Linux（需直驱 NFC 芯片）".into())
        }
        pub fn stop(&'static self) -> Result<EmulStatus, String> {
            Err("卡模拟未在运行".into())
        }
    }
}

pub use imp::EmulateService;

static SERVICE: OnceLock<EmulateService> = OnceLock::new();

/// 取得进程内唯一实例
pub fn service() -> &'static EmulateService {
    SERVICE.get_or_init(EmulateService::new)
}

/// hex 展示（小写、无分隔）
pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 监听路由帧_组包格式() {
        // 锁定 NXP 布局：Type=PROTO(0x01) Len=3 Route=DH(0x00) Power=ON(0x01) Proto=ISO-DEP(0x04)
        assert_eq!(
            listen_routing_frame(),
            vec![0x21, 0x01, 0x07, 0x00, 0x01, 0x01, 0x03, 0x00, 0x01, 0x04]
        );
    }

    #[test]
    fn 响应匹配区分_core_与_rf_组() {
        // RF_DISCOVER_RSP = 41 03（RF 组 GID=1），不能按 0x40 匹配
        assert!(is_rsp(&[0x41, 0x03, 0x01, 0x00], 0x41, 0x03));
        assert!(!is_rsp(&[0x41, 0x03, 0x01, 0x00], 0x40, 0x03));
        // CORE_RESET_RSP = 40 00
        assert!(is_rsp(&[0x40, 0x00, 0x03, 0x00], 0x40, 0x00));
        assert!(!is_rsp(&[0x40, 0x00], 0x40, 0x00));
    }

    #[test]
    fn uid_解析长度校验() {
        assert_eq!(parse_uid("db4c1b5a").unwrap(), vec![0xdb, 0x4c, 0x1b, 0x5a]);
        assert_eq!(parse_uid("DB:4C:1B:5A").unwrap(), vec![0xdb, 0x4c, 0x1b, 0x5a]);
        assert_eq!(parse_uid("01020304050607").unwrap().len(), 7);
        assert_eq!(parse_uid("0102030405060708090a").unwrap().len(), 10);
        assert!(parse_uid("aabbcc").is_err());
        assert!(parse_uid("aabbccd").is_err());
    }

    #[test]
    fn set_config_组包带参数个数() {
        // NCI 坑位：负载是 [参数个数][TLV…]，少写会得到 SYNTAX_ERROR
        let frame = core_set_config_frame(&[(0x33, vec![0xdb, 0x4c, 0x1b, 0x5a])]);
        assert_eq!(frame, vec![0x20, 0x02, 0x07, 0x01, 0x33, 0x04, 0xdb, 0x4c, 0x1b, 0x5a]);
        let frame = core_set_config_frame(&[]);
        assert_eq!(frame, vec![0x20, 0x02, 0x01, 0x00]);
    }

    #[test]
    fn t4t_选择与读取_cc() {
        let mut t4t = T4tEmu::new(&[0xd1, 0x01, 0x00]);
        // SELECT NDEF 应用
        assert_eq!(
            t4t.next(&[0x00, 0xA4, 0x04, 0x00, 0x07, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01]),
            vec![0x90, 0x00]
        );
        // SELECT CC 文件 E103
        assert_eq!(t4t.next(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x03]), vec![0x90, 0x00]);
        // READ BINARY offset=0 Le=15 → 整个 CC
        let rsp = t4t.next(&[0x00, 0xB0, 0x00, 0x00, 15]);
        assert_eq!(&rsp[..15], &CC_FILE);
        assert_eq!(&rsp[15..], &[0x90, 0x00]);
    }

    #[test]
    fn t4t_ndef_文件读取含长度前缀() {
        let record = vec![0xd1, 0x01, 0x02, 0xaa, 0xbb];
        let mut t4t = T4tEmu::new(&record);
        assert_eq!(
            t4t.next(&[0x00, 0xA4, 0x04, 0x00, 0x07, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01]),
            vec![0x90, 0x00]
        );
        assert_eq!(t4t.next(&[0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x04]), vec![0x90, 0x00]);
        // offset=0：NLEN 高位 + 记录
        let rsp = t4t.next(&[0x00, 0xB0, 0x00, 0x00, 0x03]);
        assert_eq!(&rsp[..3], &[0x00, 0x05, 0xd1]);
        // offset=2 起是记录本体
        let rsp = t4t.next(&[0x00, 0xB0, 0x00, 0x02, 0x05]);
        assert_eq!(&rsp[..5], &record[..]);
        assert_eq!(&rsp[5..], &[0x90, 0x00]);
    }

    #[test]
    fn t4t_未知命令与未选择文件() {
        let mut t4t = T4tEmu::new(&[0xd1, 0x01, 0x00]);
        // 未选择任何文件就读
        assert_eq!(t4t.next(&[0x00, 0xB0, 0x00, 0x00, 0x05]), vec![0x6a, 0x82]);
        // 写入拒绝（只读模拟）
        assert_eq!(t4t.next(&[0x00, 0xD6, 0x00, 0x00, 0x02, 0x00, 0x00]), vec![0x69, 0x82]);
        // 不支持的 INS
        assert_eq!(t4t.next(&[0x00, 0xCA, 0x00, 0x00]), vec![0x6d, 0x00]);
    }

    #[test]
    fn 克隆模式逐字节复制整条消息() {
        // kind=message：读到什么就模拟什么（多条记录也不重组），用于“模拟发现过的卡片”
        let msg = "91010c5402656e48656c6c6f204e464351010154aa";
        let req = EmulReq { kind: "message".into(), raw_hex: msg.into(), ..Default::default() };
        assert_eq!(build_ndef(&req).unwrap(), super::super::tag::unhex(msg).unwrap());
    }

    #[test]
    fn 克隆模式拒绝非法内容() {
        // 空内容：不能静默模拟出一张空 NDEF 卡（NLEN=0）
        let req = EmulReq { kind: "message".into(), raw_hex: "  ".into(), ..Default::default() };
        assert!(build_ndef(&req).is_err());
        // hex 非法
        let req = EmulReq { kind: "message".into(), raw_hex: "zz".into(), ..Default::default() };
        assert!(build_ndef(&req).is_err());
        // 记录负载截断（不是合法 NDEF 消息）
        let req = EmulReq { kind: "message".into(), raw_hex: "d10105aa".into(), ..Default::default() };
        assert!(build_ndef(&req).is_err());
        // 超过 NDEF 文件上限（253 字节）：合法消息但装不进 T4T NDEF 文件
        let req = EmulReq {
            kind: "message".into(),
            raw_hex: "9101fd54".to_string() + &"ab".repeat(253),
            ..Default::default()
        };
        assert!(build_ndef(&req).is_err());
    }
}
