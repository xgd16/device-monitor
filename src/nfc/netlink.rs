//! Linux NFC generic netlink 客户端（AF_NETLINK / NETLINK_GENERIC）
//!
//! 只依赖 `libc`，不引入新 crate：设备端离线编译也能过。
//! 协议参考内核 `include/uapi/linux/nfc.h` 与 `net/nfc/netlink.c`。
//!
//! 注意：内核的 NFC netlink 是「一次一条命令」的同步接口，本模块同样按
//! 单连接同步使用（worker 线程串行调用），命令与事件在同一个 socket 上收发。

// 常量与类型主要由 Linux 实现使用；macOS 构建时 Linux 实现被 cfg 掉，
// 会误报 dead_code，这里按平台收敛。
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::io;
use std::time::Duration;

// ───────────────────────── 常量（uapi/linux/nfc.h） ─────────────────────────

/// genl 家族名
pub const NFC_GENL_NAME: &str = "nfc";
/// genl 版本
pub const NFC_GENL_VERSION: u8 = 1;
/// 事件多播组名（net/nfc/netlink.c: NFC_GENL_MCAST_EVENT_NAME）
pub const NFC_MCAST_EVENTS: &str = "events";

/// `enum nfc_commands`
pub mod cmd {
    pub const GET_DEVICE: u8 = 1;
    pub const DEV_UP: u8 = 2;
    pub const DEV_DOWN: u8 = 3;
    pub const DEP_LINK_UP: u8 = 4;
    pub const DEP_LINK_DOWN: u8 = 5;
    pub const START_POLL: u8 = 6;
    pub const STOP_POLL: u8 = 7;
    pub const GET_TARGET: u8 = 8;
    pub const GET_SE: u8 = 26;

    // 内核主动上报的事件
    pub const EV_TARGETS_FOUND: u8 = 9;
    pub const EV_DEVICE_ADDED: u8 = 10;
    pub const EV_DEVICE_REMOVED: u8 = 11;
    pub const EV_TARGET_LOST: u8 = 12;
    pub const EV_TM_ACTIVATED: u8 = 13;
    pub const EV_TM_DEACTIVATED: u8 = 14;
    pub const EV_SE_ADDED: u8 = 22;
    pub const EV_SE_REMOVED: u8 = 23;
    pub const EV_SE_CONNECTIVITY: u8 = 24;
    pub const EV_SE_TRANSACTION: u8 = 25;
}

/// `enum nfc_attrs`
pub mod attr {
    pub const DEVICE_INDEX: u16 = 1;
    pub const DEVICE_NAME: u16 = 2;
    pub const PROTOCOLS: u16 = 3;
    pub const TARGET_INDEX: u16 = 4;
    pub const TARGET_SENS_RES: u16 = 5;
    pub const TARGET_SEL_RES: u16 = 6;
    pub const TARGET_NFCID1: u16 = 7;
    pub const TARGET_SENSB_RES: u16 = 8;
    pub const TARGET_SENSF_RES: u16 = 9;
    pub const COMM_MODE: u16 = 10;
    pub const RF_MODE: u16 = 11;
    pub const DEVICE_POWERED: u16 = 12;
    pub const IM_PROTOCOLS: u16 = 13;
    pub const TM_PROTOCOLS: u16 = 14;
    pub const SE: u16 = 18;
    pub const SE_INDEX: u16 = 21;
    pub const SE_TYPE: u16 = 22;
    pub const TARGET_ISO15693_DSFID: u16 = 25;
    pub const TARGET_ISO15693_UID: u16 = 26;
    pub const TARGET_ATS: u16 = 31;
}

/// `enum nfc_protocol`（数字是协议号，掩码是 `1 << 协议号`）
pub mod proto {
    pub const JEWEL: u32 = 1;
    pub const MIFARE: u32 = 2;
    pub const FELICA: u32 = 3;
    pub const ISO14443A: u32 = 4;
    pub const NFC_DEP: u32 = 5;
    pub const ISO14443B: u32 = 6;
    pub const ISO15693: u32 = 7;

    pub const JEWEL_MASK: u32 = 1 << JEWEL;
    pub const MIFARE_MASK: u32 = 1 << MIFARE;
    pub const FELICA_MASK: u32 = 1 << FELICA;
    pub const ISO14443A_MASK: u32 = 1 << ISO14443A;
    pub const NFC_DEP_MASK: u32 = 1 << NFC_DEP;
    pub const ISO14443B_MASK: u32 = 1 << ISO14443B;
    pub const ISO15693_MASK: u32 = 1 << ISO15693;

    /// 前端使用的协议名 ↔ 协议号
    pub fn from_name(name: &str) -> Option<u32> {
        Some(match name {
            "jewel" => JEWEL,
            "mifare" => MIFARE,
            "felica" => FELICA,
            "iso14443a" => ISO14443A,
            "nfc_dep" => NFC_DEP,
            "iso14443b" => ISO14443B,
            "iso15693" => ISO15693,
            _ => return None,
        })
    }

    pub fn name(p: u32) -> &'static str {
        match p {
            JEWEL => "jewel",
            MIFARE => "mifare",
            FELICA => "felica",
            ISO14443A => "iso14443a",
            NFC_DEP => "nfc_dep",
            ISO14443B => "iso14443b",
            ISO15693 => "iso15693",
            _ => "unknown",
        }
    }

    /// 掩码 → 协议名列表
    pub fn names(mask: u32) -> Vec<String> {
        (0..8)
            .filter(|b| mask & (1 << b) != 0)
            .map(|b| name(b))
            .filter(|n| *n != "unknown")
            .map(str::to_string)
            .collect()
    }
}

// ───────────────────────── 数据结构 ─────────────────────────

/// NFC 设备（控制器）信息
#[derive(Debug, Clone, Default)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
    /// 控制器支持的协议掩码（`NFC_PROTO_*_MASK` 或起来）
    pub protocols: u32,
    pub powered: bool,
    /// NFC_ATTR_RF_MODE：0=无 1=发起方 2=目标方
    pub rf_mode: u8,
}

/// 对端目标（卡）信息
#[derive(Debug, Clone, Default)]
pub struct TargetInfo {
    pub index: u32,
    /// 该卡支持的协议掩码
    pub protocols: u32,
    /// ISO14443-A 的 ATQA
    pub sens_res: Option<u16>,
    /// ISO14443-A 的 SAK
    pub sel_res: Option<u8>,
    /// ISO14443-A 的 UID（NFCID1）
    pub nfcid1: Vec<u8>,
    pub sensb_res: Vec<u8>,
    pub sensf_res: Vec<u8>,
    /// ISO14443-4 的 ATS
    pub ats: Vec<u8>,
    pub iso15693_uid: Vec<u8>,
}

impl TargetInfo {
    /// 做原始帧收发时优先使用的协议号：MIFARE / ISO14443-A / JEWEL / FELICA / ISO14443-B
    pub fn best_protocol(&self) -> Option<u32> {
        [
            proto::MIFARE_MASK,
            proto::ISO14443A_MASK,
            proto::JEWEL_MASK,
            proto::FELICA_MASK,
            proto::ISO14443B_MASK,
            proto::NFC_DEP_MASK,
        ]
        .into_iter()
        .find(|m| self.protocols & m != 0)
        .map(|m| m.trailing_zeros())
    }
}

/// 安全单元（eSE / UICC）
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct SeInfo {
    pub index: u32,
    pub se_type: u8,
}

/// 内核上报的事件
#[derive(Debug, Clone)]
pub enum NfcEvent {
    TargetsFound,
    TargetLost,
    TmActivated { protocol: u32, comm_mode: u8 },
    TmDeactivated,
    DeviceAdded,
    DeviceRemoved,
    SeAdded,
    SeRemoved,
    SeTransaction,
    Other(u8),
}

// ───────────────────────── 报文编解码（跨平台纯函数） ─────────────────────────

pub(crate) const NLMSG_HDRLEN: usize = 16;
pub(crate) const GENL_HDRLEN: usize = 4;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
pub(crate) const NLM_F_REQUEST: u16 = 1;
pub(crate) const NLM_F_ACK: u16 = 4;
pub(crate) const NLM_F_DUMP: u16 = 0x300;

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

pub(crate) fn rd_u16(b: &[u8]) -> u16 {
    u16::from_ne_bytes([b[0], b[1]])
}

pub(crate) fn rd_u32(b: &[u8]) -> u32 {
    u32::from_ne_bytes([b[0], b[1], b[2], b[3]])
}

/// 逐条遍历一个 netlink 报文缓冲
pub(crate) fn walk_nlmsgs(buf: &[u8], mut f: impl FnMut(u16, u16, u32, &[u8])) {
    let mut off = 0usize;
    while off + NLMSG_HDRLEN <= buf.len() {
        let len = rd_u32(&buf[off..]) as usize;
        let ty = rd_u16(&buf[off + 4..]);
        let flags = rd_u16(&buf[off + 6..]);
        let seq = rd_u32(&buf[off + 8..]);
        if len < NLMSG_HDRLEN || off + len > buf.len() {
            break;
        }
        f(ty, flags, seq, &buf[off + NLMSG_HDRLEN..off + len]);
        off += align4(len);
    }
}

/// 逐条遍历 genl 属性（type 去掉 NLA_F_NESTED 等高两位标记）
pub(crate) fn walk_attrs(mut b: &[u8], mut f: impl FnMut(u16, &[u8])) {
    while b.len() >= 4 {
        let len = rd_u16(b) as usize;
        let ty = rd_u16(&b[2..]) & 0x3fff;
        if len < 4 || len > b.len() {
            break;
        }
        f(ty, &b[4..len]);
        let adv = align4(len);
        if adv > b.len() {
            break;
        }
        b = &b[adv..];
    }
}

/// genl 报文 → (命令号, 属性)
fn genl_parts(payload: &[u8]) -> Option<(u8, &[u8])> {
    if payload.len() < GENL_HDRLEN {
        return None;
    }
    Some((payload[0], &payload[GENL_HDRLEN..]))
}

fn parse_device(attrs: &[(u16, Vec<u8>)]) -> DeviceInfo {
    let mut d = DeviceInfo::default();
    for (t, v) in attrs {
        let (t, v) = (*t, v.as_slice());
        match t {
            attr::DEVICE_INDEX if v.len() == 4 => d.index = rd_u32(v),
            attr::DEVICE_NAME => {
                let end = v.iter().position(|c| *c == 0).unwrap_or(v.len());
                d.name = String::from_utf8_lossy(&v[..end]).to_string();
            }
            attr::PROTOCOLS if v.len() == 4 => d.protocols = rd_u32(v),
            attr::DEVICE_POWERED if v.len() == 1 => d.powered = v[0] != 0,
            attr::RF_MODE if v.len() == 1 => d.rf_mode = v[0],
            _ => {}
        }
    }
    d
}

fn parse_target(attrs: &[(u16, Vec<u8>)]) -> TargetInfo {
    let mut t = TargetInfo::default();
    for (ty, v) in attrs {
        let (ty, v) = (*ty, v.as_slice());
        match ty {
            attr::TARGET_INDEX if v.len() == 4 => t.index = rd_u32(v),
            attr::PROTOCOLS if v.len() == 4 => t.protocols = rd_u32(v),
            attr::TARGET_SENS_RES if v.len() == 2 => t.sens_res = Some(rd_u16(v)),
            attr::TARGET_SEL_RES if v.len() == 1 => t.sel_res = Some(v[0]),
            attr::TARGET_NFCID1 => t.nfcid1 = v.to_vec(),
            attr::TARGET_SENSB_RES => t.sensb_res = v.to_vec(),
            attr::TARGET_SENSF_RES => t.sensf_res = v.to_vec(),
            attr::TARGET_ATS => t.ats = v.to_vec(),
            attr::TARGET_ISO15693_UID => t.iso15693_uid = v.to_vec(),
            _ => {}
        }
    }
    t
}

fn parse_se(attrs: &[(u16, Vec<u8>)]) -> SeInfo {
    let mut s = SeInfo::default();
    for (t, v) in attrs {
        let (t, v) = (*t, v.as_slice());
        match t {
            attr::SE_INDEX if v.len() == 4 => s.index = rd_u32(v),
            attr::SE_TYPE if v.len() == 1 => s.se_type = v[0],
            _ => {}
        }
    }
    s
}

/// 把一条 genl 报文的属性区解析成 (属性号, 负载) 列表
pub(crate) fn collect_attrs(attrs: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut out = Vec::new();
    walk_attrs(attrs, |t, v| out.push((t, v.to_vec())));
    out
}

/// 组装一条 genl 命令（跨平台，供 Linux 实现发送与单测使用）
#[derive(Debug)]
pub(crate) struct Msg {
    buf: Vec<u8>,
}

impl Msg {
    pub(crate) fn new(type_: u16, flags: u16, seq: u32, c: u8) -> Self {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&0u32.to_ne_bytes()); // nlmsg_len，最后回填
        buf.extend_from_slice(&type_.to_ne_bytes());
        buf.extend_from_slice(&flags.to_ne_bytes());
        buf.extend_from_slice(&seq.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes()); // nlmsg_pid
        buf.push(c);
        buf.push(NFC_GENL_VERSION);
        buf.extend_from_slice(&0u16.to_ne_bytes());
        Msg { buf }
    }

    pub(crate) fn attr(mut self, t: u16, data: &[u8]) -> Self {
        let len = (4 + data.len()) as u16;
        self.buf.extend_from_slice(&len.to_ne_bytes());
        self.buf.extend_from_slice(&t.to_ne_bytes());
        self.buf.extend_from_slice(data);
        while self.buf.len() % 4 != 0 {
            self.buf.push(0);
        }
        self
    }

    pub(crate) fn attr_u32(self, t: u16, v: u32) -> Self {
        self.attr(t, &v.to_ne_bytes())
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        let len = self.buf.len() as u32;
        self.buf[0..4].copy_from_slice(&len.to_ne_bytes());
        self.buf
    }
}

/// `attribute` 的入参形式：属性号 + 原始负载
pub(crate) type AttrSlice<'a> = &'a [(u16, &'a [u8])];

/// 打包命令报文（Linux/非 Linux 共用，便于测试）
pub(crate) fn build_cmd(family: u16, flags: u16, seq: u32, c: u8, attrs: AttrSlice<'_>) -> Vec<u8> {
    let mut m = Msg::new(family, flags, seq, c);
    for (t, v) in attrs {
        m = m.attr(*t, v);
    }
    m.finish()
}

// ───────────────────────── Linux 实现 ─────────────────────────

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::mem;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    const NETLINK_GENERIC: i32 = 16;
    const GENL_ID_CTRL: u16 = 16;
    const CTRL_CMD_GETFAMILY: u8 = 3;
    const CTRL_ATTR_FAMILY_ID: u16 = 1;
    const CTRL_ATTR_FAMILY_NAME: u16 = 2;
    const CTRL_ATTR_MCAST_GROUPS: u16 = 7;
    const CTRL_ATTR_MCAST_GRP_NAME: u16 = 1;
    const CTRL_ATTR_MCAST_GRP_ID: u16 = 2;
    const SOL_NETLINK: i32 = 270;
    const NETLINK_ADD_MEMBERSHIP: i32 = 1;

    /// NFC generic netlink 连接
    pub struct Netlink {
        fd: OwnedFd,
        family: u16,
        group: Option<u32>,
        seq: u32,
    }

    impl Netlink {
        /// 打开 netlink 连接、解析家族号并订阅 `events` 多播组
        pub fn open() -> io::Result<Self> {
            let raw = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_GENERIC) };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            // nl_pad 在 libc 里是私有的 Padding<u16>，只能用零值构造
            let mut sa: libc::sockaddr_nl = unsafe { mem::zeroed() };
            sa.nl_family = libc::AF_NETLINK as u16;
            let rc = unsafe {
                libc::bind(
                    raw,
                    &sa as *const libc::sockaddr_nl as *const libc::sockaddr,
                    mem::size_of::<libc::sockaddr_nl>() as u32,
                )
            };
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut nl = Netlink { fd, family: 0, group: None, seq: 0 };
            let (family, group) = nl.resolve_family()?;
            nl.family = family;
            nl.group = group;
            if let Some(g) = group {
                let rc = unsafe {
                    libc::setsockopt(
                        nl.fd.as_raw_fd(),
                        SOL_NETLINK,
                        NETLINK_ADD_MEMBERSHIP,
                        &g as *const u32 as *const c_void,
                        mem::size_of::<u32>() as u32,
                    )
                };
                if rc < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(nl)
        }

        /// 家族号（`nfc`）
        pub fn family(&self) -> u16 {
            self.family
        }

        fn next_seq(&mut self) -> u32 {
            self.seq = self.seq.wrapping_add(1);
            self.seq
        }

        fn send(&self, buf: &[u8]) -> io::Result<()> {
            // nl_pad 在 libc 里是私有的 Padding<u16>，只能用零值构造
            let mut sa: libc::sockaddr_nl = unsafe { mem::zeroed() };
            sa.nl_family = libc::AF_NETLINK as u16;
            let n = unsafe {
                libc::sendto(
                    self.fd.as_raw_fd(),
                    buf.as_ptr() as *const c_void,
                    buf.len(),
                    0,
                    &sa as *const libc::sockaddr_nl as *const libc::sockaddr,
                    mem::size_of::<libc::sockaddr_nl>() as u32,
                )
            };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }

        fn recv_timeout(&self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
            let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
            let r = unsafe { libc::poll(&mut pfd, 1, ms) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    return Ok(None);
                }
                return Err(e);
            }
            if r == 0 {
                return Ok(None);
            }
            let mut buf = vec![0u8; 64 * 1024];
            let n = unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut c_void, buf.len(), 0) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            buf.truncate(n as usize);
            Ok(Some(buf))
        }

        /// 解析 `nfc` 家族：返回 (家族号, events 多播组号)
        fn resolve_family(&mut self) -> io::Result<(u16, Option<u32>)> {
            let seq = self.next_seq();
            // 家族名需要以 NUL 结尾
            let mut payload = NFC_GENL_NAME.as_bytes().to_vec();
            payload.push(0);
            let msg = build_cmd(
                GENL_ID_CTRL,
                NLM_F_REQUEST,
                seq,
                CTRL_CMD_GETFAMILY,
                &[(CTRL_ATTR_FAMILY_NAME, &payload)],
            );
            self.send(&msg)?;

            let buf = loop {
                match self.recv_timeout(Duration::from_secs(3))? {
                    Some(b) => break b,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            "内核未提供 nfc netlink 家族（NFC 子系统未加载？）",
                        ));
                    }
                }
            };
            let mut family = 0u16;
            let mut group = None;
            walk_nlmsgs(&buf, |ty, _f, _s, pl| {
                if ty == NLMSG_ERROR || ty == NLMSG_DONE {
                    return;
                }
                let Some((_c, attrs)) = genl_parts(pl) else { return };
                walk_attrs(attrs, |t, v| match t {
                    CTRL_ATTR_FAMILY_ID if v.len() >= 2 => family = rd_u16(v),
                    CTRL_ATTR_MCAST_GROUPS => {
                        walk_attrs(v, |_g, entry| {
                            let mut name = String::new();
                            let mut id = None;
                            walk_attrs(entry, |t2, v2| match t2 {
                                CTRL_ATTR_MCAST_GRP_NAME => {
                                    let end = v2.iter().position(|c| *c == 0).unwrap_or(v2.len());
                                    name = String::from_utf8_lossy(&v2[..end]).to_string();
                                }
                                CTRL_ATTR_MCAST_GRP_ID if v2.len() >= 4 => id = Some(rd_u32(v2)),
                                _ => {}
                            });
                            if name == NFC_MCAST_EVENTS {
                                group = id;
                            }
                        });
                    }
                    _ => {}
                });
            });
            if family == 0 {
                return Err(io::Error::new(io::ErrorKind::NotFound, "nfc 家族号解析失败"));
            }
            Ok((family, group))
        }

        /// 等待命令 ACK（单连接同步使用，一次只有一条命令在飞）
        fn wait_ack(&mut self, timeout: Duration) -> io::Result<()> {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                let remain = deadline.saturating_duration_since(std::time::Instant::now());
                if remain.is_zero() {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "等待内核响应超时"));
                }
                let Some(buf) = self.recv_timeout(remain)? else {
                    continue;
                };
                let mut result = None;
                walk_nlmsgs(&buf, |ty, _f, _s, pl| {
                    if ty == NLMSG_ERROR && pl.len() >= 4 {
                        let err = i32::from_ne_bytes([pl[0], pl[1], pl[2], pl[3]]);
                        result = Some(err);
                    }
                });
                if let Some(err) = result {
                    return if err == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(-err)) };
                }
                // 事件等其它报文：忽略，继续等 ACK
            }
        }

        fn simple(&mut self, c: u8, idx: u32) -> io::Result<()> {
            let seq = self.next_seq();
            let msg = build_cmd(
                self.family,
                NLM_F_REQUEST | NLM_F_ACK,
                seq,
                c,
                &[(attr::DEVICE_INDEX, &idx.to_ne_bytes())],
            );
            self.send(&msg)?;
            self.wait_ack(Duration::from_secs(4))
        }

        /// `NFC_CMD_DEV_UP`
        pub fn dev_up(&mut self, idx: u32) -> io::Result<()> {
            self.simple(cmd::DEV_UP, idx)
        }

        /// `NFC_CMD_DEV_DOWN`
        pub fn dev_down(&mut self, idx: u32) -> io::Result<()> {
            self.simple(cmd::DEV_DOWN, idx)
        }

        /// `NFC_CMD_STOP_POLL`
        pub fn stop_poll(&mut self, idx: u32) -> io::Result<()> {
            self.simple(cmd::STOP_POLL, idx)
        }

        /// `NFC_CMD_START_POLL`：im=发起方轮询协议掩码，tm=目标方监听协议掩码
        pub fn start_poll(&mut self, idx: u32, im: u32, tm: u32) -> io::Result<()> {
            let seq = self.next_seq();
            let idx_b = idx.to_ne_bytes();
            let im_b = im.to_ne_bytes();
            let tm_b = tm.to_ne_bytes();
            let mut attrs: Vec<(u16, &[u8])> = vec![(attr::DEVICE_INDEX, &idx_b)];
            if im != 0 {
                attrs.push((attr::IM_PROTOCOLS, &im_b));
            }
            if tm != 0 {
                attrs.push((attr::TM_PROTOCOLS, &tm_b));
            }
            let msg = build_cmd(self.family, NLM_F_REQUEST | NLM_F_ACK, seq, cmd::START_POLL, &attrs);
            self.send(&msg)?;
            self.wait_ack(Duration::from_secs(6))
        }

        /// 通用 dump：返回每条报文解析出的属性列表
        fn dump(&mut self, c: u8, idx: u32, timeout: Duration) -> io::Result<Vec<Vec<(u16, Vec<u8>)>>> {
            let seq = self.next_seq();
            let msg = build_cmd(
                self.family,
                NLM_F_REQUEST | NLM_F_DUMP,
                seq,
                c,
                &[(attr::DEVICE_INDEX, &idx.to_ne_bytes())],
            );
            self.send(&msg)?;
            let deadline = std::time::Instant::now() + timeout;
            let mut out = Vec::new();
            loop {
                let remain = deadline.saturating_duration_since(std::time::Instant::now());
                if remain.is_zero() {
                    break;
                }
                let Some(buf) = self.recv_timeout(remain)? else {
                    continue;
                };
                let mut done = false;
                walk_nlmsgs(&buf, |ty, _f, _s, pl| {
                    if ty == NLMSG_DONE {
                        done = true;
                        return;
                    }
                    if ty == NLMSG_ERROR {
                        return;
                    }
                    let Some((_c2, attrs)) = genl_parts(pl) else { return };
                    let mut one = Vec::new();
                    walk_attrs(attrs, |t, v| one.push((t, v.to_vec())));
                    out.push(one);
                });
                if done {
                    break;
                }
            }
            Ok(out)
        }

        /// `NFC_CMD_GET_DEVICE` dump
        pub fn get_devices(&mut self) -> io::Result<Vec<DeviceInfo>> {
            Ok(self.dump(cmd::GET_DEVICE, 0, Duration::from_secs(2))?.iter().map(|a| parse_device(a)).collect())
        }

        /// `NFC_CMD_GET_TARGET` dump（需先 `START_POLL`）
        pub fn get_targets(&mut self, idx: u32) -> io::Result<Vec<TargetInfo>> {
            Ok(self.dump(cmd::GET_TARGET, idx, Duration::from_secs(2))?.iter().map(|a| parse_target(a)).collect())
        }

        /// `NFC_CMD_GET_SE` dump（安全单元）
        pub fn get_ses(&mut self, idx: u32) -> io::Result<Vec<SeInfo>> {
            Ok(self.dump(cmd::GET_SE, idx, Duration::from_millis(1500))?.iter().map(|a| parse_se(a)).collect())
        }

        /// 读取一条事件（超时返回 None）
        pub fn next_event(&mut self, timeout: Duration) -> io::Result<Option<NfcEvent>> {
            let Some(buf) = self.recv_timeout(timeout)? else {
                return Ok(None);
            };
            let mut ev = None;
            walk_nlmsgs(&buf, |ty, _f, _s, pl| {
                if ty != self.family {
                    return;
                }
                let Some((c, attrs)) = genl_parts(pl) else { return };
                let mut protocol = 0u32;
                let mut comm_mode = 0u8;
                if c == cmd::EV_TM_ACTIVATED {
                    walk_attrs(attrs, |t, v| match t {
                        attr::PROTOCOLS if v.len() == 4 => protocol = rd_u32(v),
                        attr::COMM_MODE if v.len() == 1 => comm_mode = v[0],
                        _ => {}
                    });
                }
                ev = Some(match c {
                    cmd::EV_TARGETS_FOUND => NfcEvent::TargetsFound,
                    cmd::EV_TARGET_LOST => NfcEvent::TargetLost,
                    cmd::EV_TM_ACTIVATED => NfcEvent::TmActivated { protocol, comm_mode },
                    cmd::EV_TM_DEACTIVATED => NfcEvent::TmDeactivated,
                    cmd::EV_DEVICE_ADDED => NfcEvent::DeviceAdded,
                    cmd::EV_DEVICE_REMOVED => NfcEvent::DeviceRemoved,
                    cmd::EV_SE_ADDED => NfcEvent::SeAdded,
                    cmd::EV_SE_REMOVED => NfcEvent::SeRemoved,
                    cmd::EV_SE_TRANSACTION => NfcEvent::SeTransaction,
                    other => NfcEvent::Other(other),
                });
            });
            Ok(ev)
        }
    }
}

// ───────────────────────── 非 Linux 占位实现 ─────────────────────────

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    /// 非 Linux 平台：NFC 子系统不可用，接口保留以便上层编译与优雅降级
    pub struct Netlink;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "NFC netlink 仅在 Linux 上可用（当前平台非 Linux）")
    }

    impl Netlink {
        pub fn open() -> io::Result<Self> {
            Err(unsupported())
        }
        pub fn family(&self) -> u16 {
            0
        }
        pub fn dev_up(&mut self, _idx: u32) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn dev_down(&mut self, _idx: u32) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn start_poll(&mut self, _idx: u32, _im: u32, _tm: u32) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn stop_poll(&mut self, _idx: u32) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn get_devices(&mut self) -> io::Result<Vec<DeviceInfo>> {
            Err(unsupported())
        }
        pub fn get_targets(&mut self, _idx: u32) -> io::Result<Vec<TargetInfo>> {
            Err(unsupported())
        }
        pub fn get_ses(&mut self, _idx: u32) -> io::Result<Vec<SeInfo>> {
            Err(unsupported())
        }
        pub fn next_event(&mut self, _timeout: Duration) -> io::Result<Option<NfcEvent>> {
            Err(unsupported())
        }
    }
}

pub use imp::Netlink;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 协议名与协议号互转() {
        assert_eq!(proto::from_name("iso14443a"), Some(proto::ISO14443A));
        assert_eq!(proto::from_name("mifare"), Some(proto::MIFARE));
        assert_eq!(proto::name(proto::ISO14443B), "iso14443b");
        assert_eq!(proto::from_name("nope"), None);
    }

    #[test]
    fn 掩码展开协议名() {
        let mask = proto::ISO14443A_MASK | proto::MIFARE_MASK;
        assert_eq!(proto::names(mask), vec!["mifare", "iso14443a"]);
    }

    #[test]
    fn 取优先协议() {
        let t = TargetInfo { protocols: proto::MIFARE_MASK, ..Default::default() };
        assert_eq!(t.best_protocol(), Some(proto::MIFARE));
        let t2 = TargetInfo { protocols: proto::ISO14443A_MASK, ..Default::default() };
        assert_eq!(t2.best_protocol(), Some(proto::ISO14443A));
        let t3 = TargetInfo::default();
        assert_eq!(t3.best_protocol(), None);
    }

    #[test]
    fn 报文长度与属性对齐() {
        let m = build_cmd(
            30,
            NLM_F_REQUEST | NLM_F_ACK,
            7,
            cmd::DEV_UP,
            &[(attr::DEVICE_INDEX, &0u32.to_ne_bytes())],
        );
        assert_eq!(m.len(), NLMSG_HDRLEN + GENL_HDRLEN + 8);
        assert_eq!(rd_u32(&m[0..]), m.len() as u32);
        assert_eq!(rd_u16(&m[4..]), 30);
        assert_eq!(m[NLMSG_HDRLEN], cmd::DEV_UP);
    }

    #[test]
    fn 解析属性可跳过未知项() {
        let mut attrs = Vec::new();
        attrs.extend_from_slice(&8u16.to_ne_bytes()); // 未知属性，4 字节负载
        attrs.extend_from_slice(&99u16.to_ne_bytes());
        attrs.extend_from_slice(&0xdeadbeefu32.to_ne_bytes());
        attrs.extend_from_slice(&8u16.to_ne_bytes()); // TARGET_INDEX = 5（头 4 + 负载 4）
        attrs.extend_from_slice(&attr::TARGET_INDEX.to_ne_bytes());
        attrs.extend_from_slice(&5u32.to_ne_bytes());
        let t = parse_target(&collect_attrs(&attrs));
        assert_eq!(t.index, 5);
    }
}
