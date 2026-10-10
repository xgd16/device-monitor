//! NFC 原始帧收发（`AF_NFC`）与常见标签（Type 2 / Type 4）读写
//!
//! 内核把「目标激活」放在 `connect(2)`：connect 成功后 `send/recv` 就是直接
//! 对卡片收发 ISO14443 帧（`net/nfc/rawsock.c`）。三个必须踩准的点：
//!
//! 1. socket 类型必须是 **`SOCK_SEQPACKET`**：内核 `rawsock_create()` 里
//!    `SOCK_RAW` 会拿到 `rawsock_raw_ops`（`.connect = sock_no_connect`、
//!    `.sendmsg = sock_no_sendmsg`），那是**被动监听所有 NFC 流量**的 socket，
//!    connect 会返回 `EOPNOTSUPP(95)`；`SOCK_SEQPACKET` 才拿到可收发的
//!    `rawsock_ops`。
//! 2. 只能用 `connect()`，不能 `bind()`（`rawsock_ops.bind = sock_no_bind`）。
//! 3. `sockaddr_nfc.nfc_protocol` 要填**协议号**（如 `NFC_PROTO_MIFARE = 2`），
//!    不是 `1 << 协议号` 掩码。

use std::io;
use std::time::Duration;

/// 一次原始帧往返
#[derive(Debug, Clone, serde::Serialize)]
pub struct Exchange {
    pub tx: String,
    pub rx: String,
}

/// 单个页/块
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageDump {
    pub page: u8,
    pub hex: String,
    pub ascii: String,
}

// ───────────────────────── Linux：AF_NFC raw socket ─────────────────────────

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::mem;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    const AF_NFC: i32 = 39;
    const NFC_SOCKPROTO_RAW: i32 = 0;
    /// 可收发原始帧的 socket 类型（`SOCK_RAW` 在本内核是只读监听）
    const NFC_SOCK_TRANSCEIVE: i32 = libc::SOCK_SEQPACKET;

    #[repr(C)]
    struct SockaddrNfc {
        sa_family: u16,
        dev_idx: u32,
        target_idx: u32,
        nfc_protocol: u32,
    }

    /// 与某张卡建立原始帧通道（connect 时内核完成目标激活）
    pub struct RawTag {
        fd: OwnedFd,
    }

    impl RawTag {
        pub fn connect(dev_idx: u32, target_idx: u32, protocol: u32) -> io::Result<Self> {
            let raw = unsafe { libc::socket(AF_NFC, NFC_SOCK_TRANSCEIVE, NFC_SOCKPROTO_RAW) };
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let sa = SockaddrNfc { sa_family: AF_NFC as u16, dev_idx, target_idx, nfc_protocol: protocol };
            let rc = unsafe {
                libc::connect(
                    raw,
                    &sa as *const SockaddrNfc as *const libc::sockaddr,
                    mem::size_of::<SockaddrNfc>() as u32,
                )
            };
            if rc < 0 {
                let e = io::Error::last_os_error();
                return Err(io::Error::new(
                    e.kind(),
                    format!("激活目标失败（target={target_idx} protocol={protocol}）: {e}"),
                ));
            }
            Ok(RawTag { fd })
        }

        /// 发一帧收一帧；超时则丢弃可能迟到的响应并报错
        pub fn transceive(&self, tx: &[u8], timeout: Duration) -> io::Result<Vec<u8>> {
            let n = unsafe { libc::send(self.fd.as_raw_fd(), tx.as_ptr() as *const c_void, tx.len(), 0) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
            let r = unsafe { libc::poll(&mut pfd, 1, ms) };
            if r < 0 {
                return Err(io::Error::last_os_error());
            }
            if r == 0 {
                // 清掉可能的迟到响应，避免污染下一次收发
                let mut junk = [0u8; 256];
                unsafe { libc::recv(self.fd.as_raw_fd(), junk.as_mut_ptr() as *mut c_void, junk.len(), 0) };
                return Err(io::Error::new(io::ErrorKind::TimedOut, "卡片无响应（超时）"));
            }
            let mut buf = [0u8; 1024];
            let n = unsafe { libc::recv(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut c_void, buf.len(), 0) };
            if n < 0 {
                return Err(io::Error::last_os_error());
            }
            // 内核在数据交换完成路径上会 rawsock_add_header() 塞 1 字节 NULL 头
            // （Documentation/networking/nfc.rst：「every data received has a NULL
            // header of 1 byte」），必须剥掉才是卡片真实响应。
            let got = &buf[..n as usize];
            Ok(if got.is_empty() { Vec::new() } else { got[1..].to_vec() })
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    /// 非 Linux 平台占位：无法访问 NFC 硬件
    pub struct RawTag;

    impl RawTag {
        pub fn connect(_dev_idx: u32, _target_idx: u32, _protocol: u32) -> io::Result<Self> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "NFC 原始帧收发仅在 Linux 上可用"))
        }
        pub fn transceive(&self, _tx: &[u8], _timeout: Duration) -> io::Result<Vec<u8>> {
            Err(io::Error::new(io::ErrorKind::Unsupported, "NFC 原始帧收发仅在 Linux 上可用"))
        }
    }
}

pub use imp::RawTag;

// ───────────────────────── 通用小工具 ─────────────────────────

/// 字节 → 连续小写 hex
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 字节 → 冒号分隔 hex（UID 展示用）
pub fn hex_colon(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

/// 连续 hex 字符串 → 字节（允许空格/冒号/换行，忽略大小写）
pub fn unhex(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace() && *c != ':' && *c != '-').collect();
    if cleaned.len() % 2 != 0 {
        return Err(format!("十六进制长度必须是偶数（当前 {} 位）", cleaned.len()));
    }
    if !cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("包含非十六进制字符".to_string());
    }
    Ok((0..cleaned.len() / 2).map(|i| u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16).unwrap()).collect())
}

fn ascii4(b: &[u8]) -> String {
    b.iter().map(|c| if (0x20..0x7f).contains(c) { *c as char } else { '.' }).collect()
}

/// 页缓冲 → 页转储（每 4 字节一页）
pub fn page_dump(start_page: u8, data: &[u8]) -> Vec<PageDump> {
    data.chunks(4)
        .enumerate()
        .map(|(i, chunk)| PageDump { page: start_page.wrapping_add(i as u8), hex: hex(chunk), ascii: ascii4(chunk) })
        .collect()
}

// ───────────────────────── Type 2（NTAG / MIFARE Ultralight） ─────────────────────────

/// READ(0x30) 命令一次返回 4 页（16 字节）
pub fn type2_read(tag: &RawTag, page: u8) -> io::Result<Vec<u8>> {
    let rx = tag.transceive(&[0x30, page], Duration::from_millis(600))?;
    if rx.len() != 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("READ 页 {page} 返回 {} 字节（期望 16）：{}", rx.len(), hex(&rx)),
        ));
    }
    Ok(rx)
}

/// 带原始帧记录的 READ
pub fn type2_read_logged(tag: &RawTag, page: u8, log: &mut Vec<Exchange>) -> io::Result<Vec<u8>> {
    let rx = type2_read(tag, page)?;
    log.push(Exchange { tx: hex(&[0x30, page]), rx: hex(&rx) });
    Ok(rx)
}

/// WRITE(0xA2)：写入一页 4 字节，卡返回 4-bit ACK(0x0A)
pub fn type2_write_page(tag: &RawTag, page: u8, data: &[u8; 4]) -> io::Result<()> {
    let mut tx = vec![0xA2, page];
    tx.extend_from_slice(data);
    let rx = tag.transceive(&tx, Duration::from_millis(600))?;
    // 4-bit ACK 会被内核按位对齐成 0x0A；部分卡返回 0x00（NAK）
    if rx.first() != Some(&0x0A) {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("写页 {page} 被拒绝（响应 {}）", hex(&rx)),
        ));
    }
    Ok(())
}

/// Type 2 标签的容量信息（来自 CC 页）
#[derive(Debug, Clone, serde::Serialize)]
pub struct Type2Cc {
    pub magic: String,
    pub version: u8,
    pub size_bytes: usize,
    pub access: u8,
    pub writable: bool,
}

/// 解析 CC（能力容器）页：E1 10 <size/8> <access>
pub fn parse_type2_cc(cc: &[u8]) -> Option<Type2Cc> {
    if cc.len() < 4 || cc[0] != 0xE1 {
        return None;
    }
    let size_bytes = cc[2] as usize * 8;
    let access = cc[3];
    Some(Type2Cc {
        magic: hex(&cc[..2]),
        version: cc[1],
        size_bytes,
        access,
        writable: access == 0x00,
    })
}

/// 解析 Type 2 数据区里的 NDEF TLV，返回 (消息体, 结束位置)
pub fn parse_ndef_tlv(data: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0usize;
    while i < data.len() {
        match data[i] {
            0x00 => i += 1,      // NULL TLV
            0xFE => return None, // 终止符，没有 NDEF
            0x03 => {
                let (len, hdr) = if data.get(i + 1).copied()? == 0xFF {
                    let hi = *data.get(i + 2)? as usize;
                    let lo = *data.get(i + 3)? as usize;
                    ((hi << 8) | lo, 4)
                } else {
                    (*data.get(i + 1)? as usize, 2)
                };
                let start = i + hdr;
                let end = start + len;
                if end > data.len() {
                    return None;
                }
                return Some(data[start..end].to_vec());
            }
            _ => return None,
        }
    }
    None
}

/// 把 NDEF 消息封装成 Type 2 的 TLV 数据区（含 0x03 头与 0xFE 终止符，按页补齐）
pub fn build_type2_tlv(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x03);
    if message.len() < 0xFF {
        out.push(message.len() as u8);
    } else {
        out.push(0xFF);
        out.push((message.len() >> 8) as u8);
        out.push((message.len() & 0xFF) as u8);
    }
    out.extend_from_slice(message);
    out.push(0xFE);
    while out.len() % 4 != 0 {
        out.push(0x00);
    }
    out
}

/// 读取 Type 2 标签：页 0..3（含 CC）＋数据区直到 TLV 结束
///
/// 返回 (页缓冲, CC, NDEF 消息, 原始帧记录)
pub fn type2_dump(
    tag: &RawTag,
    max_pages: u8,
) -> io::Result<(Vec<u8>, Option<Type2Cc>, Option<Vec<u8>>, Vec<Exchange>)> {
    let mut log = Vec::new();
    let mut buf = type2_read_logged(tag, 0, &mut log)?;
    let cc = parse_type2_cc(&buf[12..16]);
    // 依据 CC 声明的容量决定还要读多少页；没有 CC 就按 max_pages 读
    let need_bytes =
        cc.as_ref().map(|c| (c.size_bytes + 16).min(max_pages as usize * 4)).unwrap_or(max_pages as usize * 4);
    let mut page = 4u8;
    while (buf.len() < need_bytes) && page < max_pages {
        match type2_read_logged(tag, page, &mut log) {
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break, // 读到容量边界，停止
        }
        page = page.wrapping_add(4);
    }
    let message = parse_ndef_tlv(&buf[16..]);
    Ok((buf, cc, message, log))
}

/// 写入 Type 2 标签的 NDEF 区（从页 4 开始），返回写入字节数
pub fn type2_write_ndef(tag: &RawTag, message: &[u8], capacity: usize) -> io::Result<usize> {
    let tlv = build_type2_tlv(message);
    if tlv.len() > capacity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("内容 {} 字节超出标签容量 {} 字节", tlv.len(), capacity),
        ));
    }
    let mut page = 4u8;
    for chunk in tlv.chunks(4) {
        let mut data = [0u8; 4];
        data[..chunk.len()].copy_from_slice(chunk);
        type2_write_page(tag, page, &data)?;
        page = page.wrapping_add(1);
    }
    Ok(tlv.len())
}

// ───────────────────────── Type 4（ISO14443-4 / ISO-DEP，APDU） ─────────────────────────

/// 发送 APDU（ISO7816-4），返回数据部分（已剥离 SW）
pub fn apdu(tag: &RawTag, apdu: &[u8], log: &mut Vec<Exchange>) -> io::Result<Vec<u8>> {
    let rx = tag.transceive(apdu, Duration::from_millis(900))?;
    log.push(Exchange { tx: hex(apdu), rx: hex(&rx) });
    if rx.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("APDU 响应过短：{}", hex(&rx)),
        ));
    }
    let sw = u16::from_be_bytes([rx[rx.len() - 2], rx[rx.len() - 1]]);
    if sw != 0x9000 {
        return Err(io::Error::new(io::ErrorKind::Other, format!("APDU 返回状态字 {sw:04X}")));
    }
    Ok(rx[..rx.len() - 2].to_vec())
}

/// 选择 NDEF 应用 + CC 文件，返回 NDEF 文件 ID 与最大读取长度
fn type4_select_cc(tag: &RawTag, log: &mut Vec<Exchange>) -> io::Result<(u16, usize)> {
    // SELECT NDEF application: D2760000850101
    apdu(
        tag,
        &[0x00, 0xA4, 0x04, 0x00, 0x07, 0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01, 0x00],
        log,
    )?;
    // SELECT CC file E103
    apdu(tag, &[0x00, 0xA4, 0x00, 0x0C, 0x02, 0xE1, 0x03], log)?;
    // READ BINARY 前 2 字节 = CCLEN
    let head = apdu(tag, &[0x00, 0xB0, 0x00, 0x00, 0x02], log)?;
    if head.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "CC 长度读取失败"));
    }
    let cclen = u16::from_be_bytes([head[0], head[1]]) as usize;
    let cc = apdu(tag, &[0x00, 0xB0, 0x00, 0x00, cclen.min(255) as u8], log)?;
    // CC 里找 TLV 0x04：<len:2> <file id:2> <max ndef size:2> <read access:1> <write access:1>
    let mut i = 7usize; // 跳过 CCLEN(2) mapping version(1) MLe(2) MLc(2)
    let mut file_id = 0u16;
    let mut max_size = 0usize;
    while i + 2 <= cc.len() {
        let t = cc[i];
        let l = cc[i + 1] as usize;
        if t == 0x04 && i + 2 + l <= cc.len() && l >= 6 {
            file_id = u16::from_be_bytes([cc[i + 2], cc[i + 3]]);
            max_size = u16::from_be_bytes([cc[i + 4], cc[i + 5]]) as usize;
            break;
        }
        if t == 0x00 {
            i += 1;
            continue;
        }
        i += 2 + l;
    }
    if file_id == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "CC 中没有 NDEF 文件（TLV 0x04）"));
    }
    Ok((file_id, max_size))
}

/// 读取 Type 4 标签的 NDEF 消息
pub fn type4_read_ndef(tag: &RawTag) -> io::Result<(Vec<u8>, Vec<Exchange>)> {
    let mut log = Vec::new();
    let (file_id, max_size) = type4_select_cc(tag, &mut log)?;
    apdu(
        tag,
        &[0x00, 0xA4, 0x00, 0x0C, 0x02, (file_id >> 8) as u8, (file_id & 0xFF) as u8],
        &mut log,
    )?;
    let nlen_b = apdu(tag, &[0x00, 0xB0, 0x00, 0x00, 0x02], &mut log)?;
    if nlen_b.len() < 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "NLEN 读取失败"));
    }
    let nlen = u16::from_be_bytes([nlen_b[0], nlen_b[1]]) as usize;
    let nlen = nlen.min(max_size);
    if nlen == 0 {
        return Ok((Vec::new(), log));
    }
    let mut msg = Vec::new();
    let mut off = 2usize;
    while msg.len() < nlen {
        let want = (nlen - msg.len()).min(0xF0);
        let hi = (off >> 8) as u8;
        let lo = (off & 0xFF) as u8;
        let chunk = apdu(tag, &[0x00, 0xB0, hi, lo, want as u8], &mut log)?;
        if chunk.is_empty() {
            break;
        }
        msg.extend_from_slice(&chunk);
        off += chunk.len();
    }
    Ok((msg, log))
}

/// 写入 Type 4 标签的 NDEF 消息（NLEN 置 0 → UPDATE BINARY → 回写 NLEN）
pub fn type4_write_ndef(tag: &RawTag, message: &[u8], max_size: usize) -> io::Result<usize> {
    let mut log = Vec::new();
    if message.len() > max_size || message.len() > 0xFFFE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("内容 {} 字节超出 NDEF 文件容量 {} 字节", message.len(), max_size),
        ));
    }
    let (file_id, _) = type4_select_cc(tag, &mut log)?;
    apdu(
        tag,
        &[0x00, 0xA4, 0x00, 0x0C, 0x02, (file_id >> 8) as u8, (file_id & 0xFF) as u8],
        &mut log,
    )?;
    // NLEN = 0，声明写入中
    apdu(tag, &[0x00, 0xD6, 0x00, 0x00, 0x02, 0x00, 0x00], &mut log)?;
    // 分块 UPDATE BINARY，块大小受 NXP I2C 最大负载（32 字节）限制
    let mut off = 2usize;
    for chunk in message.chunks(16) {
        let mut apdu_v = vec![0x00, 0xD6, (off >> 8) as u8, (off & 0xFF) as u8, chunk.len() as u8];
        apdu_v.extend_from_slice(chunk);
        apdu(tag, &apdu_v, &mut log)?;
        off += chunk.len();
    }
    // 回写真实 NLEN
    let nlen = message.len() as u16;
    apdu(
        tag,
        &[0x00, 0xD6, 0x00, 0x00, 0x02, (nlen >> 8) as u8, (nlen & 0xFF) as u8],
        &mut log,
    )?;
    Ok(message.len())
}

/// 读取 Type 4 的 NDEF 文件容量（写前校验用）
pub fn type4_capacity(tag: &RawTag) -> io::Result<usize> {
    let mut log = Vec::new();
    let (_fid, max_size) = type4_select_cc(tag, &mut log)?;
    Ok(max_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_与_unhex_往返() {
        assert_eq!(hex(&[0x0a, 0xff]), "0aff");
        assert_eq!(unhex("0A:ff").unwrap(), vec![0x0a, 0xff]);
        assert_eq!(unhex(" 0a ff ").unwrap(), vec![0x0a, 0xff]);
        assert!(unhex("abc").is_err());
        assert!(unhex("zz").is_err());
    }

    #[test]
    fn 解析_type2_cc() {
        let cc = parse_type2_cc(&[0xE1, 0x10, 0x12, 0x00]).unwrap();
        assert_eq!(cc.size_bytes, 0x12 * 8);
        assert!(cc.writable);
        assert!(parse_type2_cc(&[0x00, 0x00, 0x00, 0x00]).is_none());
    }

    #[test]
    fn 解析_ndef_tlv_短长度() {
        let data = [0x03, 0x03, 0xD0, 0x00, 0x00, 0xFE];
        assert_eq!(parse_ndef_tlv(&data).unwrap(), vec![0xD0, 0x00, 0x00]);
    }

    #[test]
    fn 解析_ndef_tlv_三字节长度与_null_跳过() {
        let mut data = vec![0x00, 0x00];
        data.extend_from_slice(&[0x03, 0xFF, 0x00, 0x02, 0xAA, 0xBB]);
        assert_eq!(parse_ndef_tlv(&data).unwrap(), vec![0xAA, 0xBB]);
    }

    #[test]
    fn 构造_type2_tlv_按页补齐() {
        let tlv = build_type2_tlv(&[0xD1, 0x01, 0x02, 0x54, 0x00]);
        // 0x03 + len + 5 字节 + 0xFE = 8 字节，正好两页
        assert_eq!(tlv.len(), 8);
        assert_eq!(tlv[0], 0x03);
        assert_eq!(tlv[1], 5);
        assert_eq!(tlv[7], 0xFE);
    }

    #[test]
    fn 页转储可读() {
        let d = page_dump(4, &[0x41, 0x42, 0x43, 0x44, 0x00, 0x01, 0x02, 0x03]);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].page, 4);
        assert_eq!(d[0].ascii, "ABCD");
        assert_eq!(d[1].page, 5);
    }
}
