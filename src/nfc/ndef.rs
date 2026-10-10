//! NDEF（NFC Forum Data Exchange Format）报文编解码
//!
//! 只实现常用子集：TNF=1（NFC Forum well-known）的 Text(`T`) 与 URI(`U`) 记录，
//! 其余记录按原始字节透出，方便前端展示与调试。

use crate::nfc::tag::{hex, unhex};

/// 解析出的单条 NDEF 记录
#[derive(Debug, Clone, serde::Serialize)]
pub struct Record {
    /// Type Name Format（0=空 1=well-known 2=MIME 3=绝对 URI 4=外部 5=未知 6=未变更 7=保留）
    pub tnf: u8,
    /// 记录类型（"T" / "U" / MIME 串 / hex）——前端字段名为 `type`
    #[serde(rename = "type")]
    pub type_name: String,
    /// "text" | "uri" | "raw"
    pub kind: String,
    pub text: Option<String>,
    pub uri: Option<String>,
    pub payload_hex: String,
}

/// 解析后的整条 NDEF 消息
#[derive(Debug, Clone, serde::Serialize)]
pub struct Message {
    pub records: Vec<Record>,
    pub raw_hex: String,
    pub size: usize,
}

/// URI 记录的前缀缩写表（NFC Forum URI RTD）
const URI_PREFIXES: [&str; 36] = [
    "",
    "http://www.",
    "https://www.",
    "http://",
    "https://",
    "tel:",
    "mailto:",
    "ftp://anonymous:anonymous@",
    "ftp://ftp.",
    "ftps://",
    "sftp://",
    "smb://",
    "nfs://",
    "ftp://",
    "dav://",
    "news:",
    "telnet://",
    "imap:",
    "rtsp://",
    "urn:",
    "pop:",
    "sip:",
    "sips:",
    "tftp:",
    "btspp://",
    "btl2cap://",
    "btgoep://",
    "tcpobex://",
    "irdaobex://",
    "file://",
    "urn:epc:id:",
    "urn:epc:tag:",
    "urn:epc:pat:",
    "urn:epc:raw:",
    "urn:epc:",
    "urn:nfc:",
];

fn type_string(tnf: u8, type_: &[u8]) -> String {
    match std::str::from_utf8(type_) {
        Ok(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic()) => s.to_string(),
        _ => format!("tnf{tnf}:{}", hex(type_)),
    }
}

fn decode_text(payload: &[u8]) -> Option<String> {
    let status = *payload.first()?;
    let lang_len = (status & 0x3F) as usize;
    if payload.len() < 1 + lang_len {
        return None;
    }
    let body = &payload[1 + lang_len..];
    let utf16 = status & 0x80 != 0;
    if utf16 {
        let units: Vec<u16> = body.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&units))
    } else {
        Some(String::from_utf8_lossy(body).to_string())
    }
}

fn decode_uri(payload: &[u8]) -> Option<String> {
    let code = *payload.first()? as usize;
    let prefix = URI_PREFIXES.get(code).copied().unwrap_or("");
    Some(format!("{prefix}{}", String::from_utf8_lossy(&payload[1..])))
}

/// 解析一条完整的 NDEF 消息（可含多条记录）
pub fn parse_message(data: &[u8]) -> Result<Message, String> {
    let mut records = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let hdr = data[i];
        let tnf = hdr & 0x07;
        let short = hdr & 0x10 != 0;
        let has_id = hdr & 0x08 != 0;
        let mut p = i + 1;
        let type_len = *data.get(p).ok_or("记录头截断")? as usize;
        p += 1;
        let payload_len = if short {
            let v = *data.get(p).ok_or("记录长度截断")? as usize;
            p += 1;
            v
        } else {
            if p + 4 > data.len() {
                return Err("记录长度截断".to_string());
            }
            let v = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
            p += 4;
            v
        };
        let type_ = data.get(p..p + type_len).ok_or("记录类型截断")?.to_vec();
        p += type_len;
        if has_id {
            let id_len = *data.get(p).ok_or("ID 长度截断")? as usize;
            p += 1 + id_len;
        }
        let payload = data.get(p..p + payload_len).ok_or("记录负载截断")?.to_vec();
        p += payload_len;

        let type_name = type_string(tnf, &type_);
        let (kind, text, uri) = if tnf == 1 && type_name == "T" {
            ("text".to_string(), decode_text(&payload), None)
        } else if tnf == 1 && type_name == "U" {
            ("uri".to_string(), None, decode_uri(&payload))
        } else {
            ("raw".to_string(), None, None)
        };
        records.push(Record { tnf, type_name, kind, text, uri, payload_hex: hex(&payload) });
        i = p;
        if hdr & 0x40 != 0 {
            // ME：消息结束
            break;
        }
    }
    Ok(Message { records, raw_hex: hex(data), size: data.len() })
}

/// 构造文本记录（UTF-8，按语言码长度编码状态字节）
pub fn text_record(text: &str, lang: &str) -> Vec<u8> {
    let lang_bytes = lang.as_bytes();
    let mut payload = vec![(lang_bytes.len().min(0x3F)) as u8];
    payload.extend_from_slice(lang_bytes);
    payload.extend_from_slice(text.as_bytes());
    build_record(1, b"T", &payload)
}

/// 构造 URI 记录（自动挑选最长匹配的前缀缩写）
pub fn uri_record(uri: &str) -> Vec<u8> {
    let mut best_code = 0u8;
    let mut best_len = 0usize;
    for (code, prefix) in URI_PREFIXES.iter().enumerate() {
        if !prefix.is_empty() && uri.starts_with(prefix) && prefix.len() > best_len {
            best_code = code as u8;
            best_len = prefix.len();
        }
    }
    let mut payload = vec![best_code];
    payload.extend_from_slice(uri[best_len..].as_bytes());
    build_record(1, b"U", &payload)
}

/// 构造原始记录（供高级用法：自定义 TNF/类型/负载 hex）
pub fn raw_record(tnf: u8, type_name: &str, payload_hex: &str) -> Result<Vec<u8>, String> {
    let payload = unhex(payload_hex)?;
    Ok(build_record(tnf & 0x07, type_name.as_bytes(), &payload))
}

/// 组装一条记录（短记录、无 ID）
fn build_record(tnf: u8, type_: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + type_.len() + 4);
    let short = payload.len() < 0xFF;
    // MB | ME | SR | TNF
    out.push(0x80 | 0x40 | if short { 0x10 } else { 0x00 } | (tnf & 0x07));
    out.push(type_.len() as u8);
    if short {
        out.push(payload.len() as u8);
    } else {
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(type_);
    out.extend_from_slice(payload);
    out
}

/// 把一条记录当作整条消息返回（前端写入时用）
pub fn message_from_record(record: Vec<u8>) -> Vec<u8> {
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 文本记录可往返() {
        let rec = text_record("你好 NFC", "zh");
        let msg = parse_message(&message_from_record(rec)).unwrap();
        assert_eq!(msg.records.len(), 1);
        assert_eq!(msg.records[0].kind, "text");
        assert_eq!(msg.records[0].text.as_deref(), Some("你好 NFC"));
        assert_eq!(msg.records[0].type_name, "T");
        assert_eq!(msg.records[0].tnf, 1);
    }

    #[test]
    fn uri_记录使用前缀缩写() {
        let rec = uri_record("https://example.com/a");
        // 0x04 = "https://"
        assert_eq!(rec[rec.len() - "example.com/a".len() - 1], 0x04);
        let msg = parse_message(&message_from_record(rec)).unwrap();
        assert_eq!(msg.records[0].uri.as_deref(), Some("https://example.com/a"));
    }

    #[test]
    fn uri_无匹配前缀时原样存放() {
        let rec = uri_record("customscheme://x");
        let msg = parse_message(&message_from_record(rec)).unwrap();
        assert_eq!(msg.records[0].uri.as_deref(), Some("customscheme://x"));
    }

    #[test]
    fn 原始记录按_tnf_解析() {
        let rec = raw_record(4, "android.com:pkg", "0102").unwrap();
        let msg = parse_message(&message_from_record(rec)).unwrap();
        assert_eq!(msg.records[0].kind, "raw");
        assert_eq!(msg.records[0].tnf, 4);
        assert_eq!(msg.records[0].payload_hex, "0102");
    }

    #[test]
    fn 空负载记录不崩() {
        let rec = build_record(1, b"T", &[]);
        assert!(parse_message(&rec).is_ok());
    }

    /// 前后端字段名契约：`type` / `kind` / `raw_hex` / `size`，不要改成 Rust 习惯名
    #[test]
    fn json_字段名与前端契约一致() {
        let msg = parse_message(&text_record("hi", "en")).unwrap();
        let v = serde_json::to_value(&msg).unwrap();
        assert!(v["records"][0].get("type").is_some(), "缺 type 字段");
        assert!(v["records"][0].get("type_name").is_none(), "不应出现 type_name");
        assert_eq!(v["records"][0]["kind"], "text");
        assert_eq!(v["records"][0]["text"], "hi");
        assert!(v["raw_hex"].is_string());
        assert!(v["size"].is_u64());
    }
}
