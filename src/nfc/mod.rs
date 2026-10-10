//! NFC 子系统：netlink 控制、原始帧收发、NDEF 编解码与常驻服务
//!
//! 架构：`NfcService` 起一个常驻 worker 线程，独占 netlink 连接（命令 + 事件
//! 同一 socket），共享状态用快照锁暴露给 HTTP 层；HTTP 侧通过 oneshot 等待
//! 操作结果，不阻塞 tokio 运行时。
//!
//! 硬件实测（Mi Mix 3 / PN80T）：控制器 `nfc0`，内核只支持发起方轮询与原始帧
//! 收发；卡模拟中 NFC-DEP 监听走内核，UID 卡模拟走 [`emulate`]（用户态直驱芯片，
//! 见其模块文档）。

pub mod emulate;
pub mod ndef;
pub mod netlink;
pub mod tag;

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::oneshot;

use netlink::{Netlink, NfcEvent, TargetInfo, proto};

// ───────────────────────── 对外数据结构 ─────────────────────────

/// 控制器视图
#[derive(Debug, Clone, Serialize, Default)]
pub struct DeviceView {
    pub index: u32,
    pub name: String,
    pub protocols_mask: u32,
    pub protocols: Vec<String>,
    pub powered: bool,
    /// 内核 `NFC_ATTR_RF_MODE`：0=空闲 1=发起方 2=目标方
    pub rf_mode: u8,
    pub rfkill_soft_blocked: bool,
    pub rfkill_hard_blocked: bool,
}

/// 已发现卡片视图
#[derive(Debug, Clone, Serialize)]
pub struct TagView {
    pub index: u32,
    /// 展示用主协议名
    pub protocol: String,
    pub protocols: Vec<String>,
    /// UID（冒号分隔小写 hex）
    pub uid: String,
    /// ISO14443-A 的 ATQA
    pub atqa: String,
    pub sak: Option<u8>,
    pub ats: Option<String>,
    /// 技术类型（人类可读）
    pub technology: String,
    /// 做原始帧收发时用的协议名
    pub best_protocol: String,
    /// 是否支持读卡器模式收发（非 NFC-DEP）
    pub readable: bool,
    pub seen_at: String,
    pub lost: bool,
}

/// 单项能力
#[derive(Debug, Clone, Serialize)]
pub struct Capability {
    pub ok: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Capabilities {
    pub discover: Capability,
    pub read: Capability,
    pub write: Capability,
    pub emulate: Capability,
    pub emulate_uid: Capability,
}

/// 状态快照
#[derive(Debug, Clone, Serialize)]
pub struct NfcStatus {
    pub available: bool,
    pub platform: String,
    pub message: String,
    pub device: Option<DeviceView>,
    pub polling: bool,
    pub listening: bool,
    pub listen_protocols: Vec<String>,
    pub target_count: usize,
    pub secure_elements: Vec<netlink::SeInfo>,
    pub capabilities: Capabilities,
    pub last_error: Option<String>,
    pub last_op: Option<String>,
}

/// 事件记录
#[derive(Debug, Clone, Serialize)]
pub struct EventRecord {
    pub at: String,
    pub kind: String,
    pub text: String,
    pub detail: serde_json::Value,
}

/// `POST /api/nfc/read` 结果
#[derive(Debug, Clone, Serialize, Default)]
pub struct ReadResult {
    pub uid: String,
    pub technology: String,
    pub ndef: Option<ndef::Message>,
    pub pages: Option<Vec<tag::PageDump>>,
    pub raw_exchanges: Vec<tag::Exchange>,
    pub note: String,
}

/// `POST /api/nfc/write` 结果
#[derive(Debug, Clone, Serialize, Default)]
pub struct WriteResult {
    pub wrote_bytes: usize,
    pub template: String,
    pub verify: Option<VerifyResult>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyResult {
    pub raw_hex: String,
    pub records: usize,
}

/// `POST /api/nfc/raw` 结果
#[derive(Debug, Clone, Serialize, Default)]
pub struct RawResult {
    pub tx: String,
    pub rx: String,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

/// 读请求
#[derive(Debug, Clone)]
pub struct ReadReq {
    pub target_index: u32,
    pub protocol: String,
    /// "ndef" | "pages"
    pub mode: String,
    pub start_page: Option<u8>,
    pub page_count: Option<u8>,
}

/// 写请求
#[derive(Debug, Clone)]
pub struct WriteReq {
    pub target_index: u32,
    pub protocol: String,
    /// "text" | "uri" | "raw"
    pub kind: String,
    pub value: String,
    pub lang: String,
    pub raw_hex: String,
}

/// 原始帧请求
#[derive(Debug, Clone)]
pub struct RawReq {
    pub target_index: u32,
    pub protocol: String,
    pub tx_hex: String,
    pub timeout_ms: u64,
}

// ───────────────────────── 内部命令 ─────────────────────────

type Reply<T> = oneshot::Sender<Result<T, String>>;

enum Cmd {
    SetPower { up: bool, reply: Reply<()> },
    StartPoll { protocols: Vec<String>, reply: Reply<Vec<String>> },
    StopPoll(Reply<()>),
    StartListen(Reply<()>),
    StopListen(Reply<()>),
    Read { req: ReadReq, reply: Reply<ReadResult> },
    Write { req: WriteReq, reply: Reply<WriteResult> },
    Raw { req: RawReq, reply: Reply<RawResult> },
}

struct Shared {
    status: Mutex<NfcStatus>,
    tags: Mutex<Vec<TagView>>,
    events: Mutex<Vec<EventRecord>>,
}

impl Shared {
    fn snapshot_status(&self) -> NfcStatus {
        lock(&self.status).clone()
    }
    fn tags(&self) -> Vec<TagView> {
        lock(&self.tags).clone()
    }
    fn event(&self, kind: &str, text: impl Into<String>, detail: serde_json::Value) {
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
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

// ───────────────────────── 服务 ─────────────────────────

/// NFC 常驻服务（进程内单例）
pub struct NfcService {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
}

static SERVICE: OnceLock<NfcService> = OnceLock::new();

/// 取得进程内唯一实例（首次调用时启动 worker 线程）
pub fn service() -> &'static NfcService {
    SERVICE.get_or_init(NfcService::start)
}

impl NfcService {
    fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared {
            status: Mutex::new(initial_status()),
            tags: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        });
        let worker_shared = shared.clone();
        // 等 worker 完成首轮初始化（读设备 / 订阅事件），避免首次 status 请求拿到空状态
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("nfc-worker".to_string())
            .spawn(move || worker(rx, worker_shared, ready_tx))
            .expect("启动 NFC worker 线程失败");
        let _ = ready_rx.recv_timeout(Duration::from_secs(8));
        NfcService { tx, shared }
    }

    /// 状态快照
    pub fn status(&self) -> NfcStatus {
        let mut s = self.shared.snapshot_status();
        s.target_count = self.shared.tags().len();
        s
    }

    /// 已发现卡片
    pub fn tags(&self) -> Vec<TagView> {
        self.shared.tags()
    }

    /// 最近事件（最多 limit 条，倒序由前端处理）
    pub fn events(&self, limit: usize) -> Vec<EventRecord> {
        let ev = lock(&self.shared.events);
        let start = ev.len().saturating_sub(limit);
        ev[start..].to_vec()
    }

    async fn call<T>(&self, make: impl FnOnce(Reply<T>) -> Cmd, timeout: Duration) -> Result<T, String> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(make(tx)).map_err(|_| "NFC 服务线程已退出".to_string())?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("NFC 服务线程未返回结果".to_string()),
            Err(_) => Err("NFC 操作超时".to_string()),
        }
    }

    /// 控制器上电 / 下电
    pub async fn set_power(&self, up: bool) -> Result<(), String> {
        self.call(|reply| Cmd::SetPower { up, reply }, Duration::from_secs(20)).await
    }

    /// 开始轮询（读卡）；`protocols` 为空表示默认全选
    pub async fn start_poll(&self, protocols: Vec<String>) -> Result<Vec<String>, String> {
        self.call(|reply| Cmd::StartPoll { protocols, reply }, Duration::from_secs(25)).await
    }

    /// 停止轮询
    pub async fn stop_poll(&self) -> Result<(), String> {
        self.call(Cmd::StopPoll, Duration::from_secs(15)).await
    }

    /// 开始 NFC-DEP 监听（点对点目标模式）
    pub async fn start_listen(&self) -> Result<(), String> {
        self.call(Cmd::StartListen, Duration::from_secs(20)).await
    }

    /// 停止监听
    pub async fn stop_listen(&self) -> Result<(), String> {
        self.call(Cmd::StopListen, Duration::from_secs(15)).await
    }

    /// 读取卡片（NDEF 或原始页）
    pub async fn read(&self, req: ReadReq) -> Result<ReadResult, String> {
        self.call(|reply| Cmd::Read { req, reply }, Duration::from_secs(25)).await
    }

    /// 写入卡片
    pub async fn write(&self, req: WriteReq) -> Result<WriteResult, String> {
        self.call(|reply| Cmd::Write { req, reply }, Duration::from_secs(45)).await
    }

    /// 原始帧收发
    pub async fn raw(&self, req: RawReq) -> Result<RawResult, String> {
        self.call(|reply| Cmd::Raw { req, reply }, Duration::from_secs(12)).await
    }
}

// ───────────────────────── 初始状态与能力 ─────────────────────────

fn initial_status() -> NfcStatus {
    NfcStatus {
        available: false,
        platform: std::env::consts::OS.to_string(),
        message: "NFC 服务尚未初始化".to_string(),
        device: None,
        polling: false,
        listening: false,
        listen_protocols: vec!["nfc_dep".to_string()],
        target_count: 0,
        secure_elements: Vec::new(),
        capabilities: Capabilities {
            discover: Capability { ok: false, reason: "尚未初始化".into() },
            read: Capability { ok: false, reason: "尚未初始化".into() },
            write: Capability { ok: false, reason: "尚未初始化".into() },
            emulate: Capability { ok: false, reason: "尚未初始化".into() },
            emulate_uid: Capability {
                ok: true,
                reason: "用户态直驱芯片（解绑 nxp-nci_i2c 后经 /dev/i2c-3 直说 NCI）：可自定义 UID 并模拟 T4T NDEF 卡；模拟期间内核 nfc0 暂时下线".into(),
            },
        },
        last_error: None,
        last_op: None,
    }
}

fn capabilities_for(has_device: bool) -> Capabilities {
    Capabilities {
        discover: Capability {
            ok: has_device,
            reason: if has_device {
                "内核 NFC netlink 可用：DEV_UP + START_POLL 轮询发现卡片".into()
            } else {
                "未发现 NFC 控制器（/sys/class/nfc 为空）".into()
            },
        },
        read: Capability {
            ok: has_device,
            reason: if has_device {
                "可通过 AF_NFC 原始帧读取 Type 2（NTAG/Ultralight）与 Type 4（ISO-DEP）的 NDEF；\
                 MIFARE Classic 需要 CRYPTO1 密钥认证，暂不支持自动读取"
                    .into()
            } else {
                "未发现 NFC 控制器".into()
            },
        },
        write: Capability {
            ok: has_device,
            reason: if has_device {
                "支持写入 Type 2 / Type 4 的 NDEF 区（MIFARE Classic 需密钥，暂不支持）".into()
            } else {
                "未发现 NFC 控制器".into()
            },
        },
        emulate: Capability {
            ok: has_device,
            reason: "NFC-DEP 点对点监听（目标模式）走内核；像门禁卡那样被读卡器读到固定 UID 请用「UID 卡模拟」（用户态直驱）"
                .into(),
        },
        emulate_uid: Capability {
            ok: true,
            reason: "用户态直驱芯片（解绑 nxp-nci_i2c 后经 /dev/i2c-3 直说 NCI，CORE_SET_CONFIG 配置 LA_NFCID1）：\
                     可自定义 UID 并模拟 T4T NDEF 卡；模拟期间内核 nfc0 暂时下线，停止后自动交还驱动"
                .into(),
        },
    }
}

// ───────────────────────── worker 线程 ─────────────────────────

fn worker(rx: Receiver<Cmd>, shared: Arc<Shared>, ready: std::sync::mpsc::SyncSender<()>) {
    let mut nl = match Netlink::open() {
        Ok(n) => n,
        Err(e) => {
            let msg = format!("NFC 不可用：{e}");
            {
                let mut st = lock(&shared.status);
                st.available = false;
                st.message = msg.clone();
                st.capabilities = capabilities_for(false);
                st.last_error = Some(msg.clone());
            }
            shared.event("error", msg.clone(), serde_json::json!({}));
            let _ = ready.send(());
            // 仍要响应命令，统一回错误
            while let Ok(cmd) = rx.recv() {
                fail_cmd(cmd, &msg);
            }
            return;
        }
    };

    refresh_all(&mut nl, &shared);
    shared.event("info", "NFC 服务已启动", serde_json::json!({}));
    let _ = ready.send(());

    let idle = Duration::from_millis(150);
    let mut last_refresh = Instant::now();
    loop {
        match rx.recv_timeout(idle) {
            Ok(cmd) => handle_cmd(&mut nl, &shared, cmd),
            Err(RecvTimeoutError::Timeout) => {
                pump_events(&mut nl, &shared);
                // 周期性同步控制器状态（上电标记 / rfkill / 设备增删）
                if last_refresh.elapsed() >= Duration::from_secs(5) {
                    refresh_device(&mut nl, &shared);
                    last_refresh = Instant::now();
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// 服务不可用时统一回复错误
fn fail_cmd(cmd: Cmd, msg: &str) {
    let msg = msg.to_string();
    match cmd {
        Cmd::SetPower { reply, .. } => {
            let _ = reply.send(Err(msg));
        }
        Cmd::StartPoll { reply, .. } => {
            let _ = reply.send(Err(msg));
        }
        Cmd::StopPoll(r) => {
            let _ = r.send(Err(msg));
        }
        Cmd::StartListen(r) => {
            let _ = r.send(Err(msg));
        }
        Cmd::StopListen(r) => {
            let _ = r.send(Err(msg));
        }
        Cmd::Read { reply, .. } => {
            let _ = reply.send(Err(msg));
        }
        Cmd::Write { reply, .. } => {
            let _ = reply.send(Err(msg));
        }
        Cmd::Raw { reply, .. } => {
            let _ = reply.send(Err(msg));
        }
    }
}

/// 读取设备并刷新状态；返回控制器是否可用
fn refresh_device(nl: &mut Netlink, shared: &Arc<Shared>) -> bool {
    match nl.get_devices() {
        Ok(devs) => {
            let dev = devs.into_iter().next();
            let (soft, hard) = rfkill_state();
            let mut st = lock(&shared.status);
            st.available = dev.is_some();
            st.device = dev.as_ref().map(|d| DeviceView {
                index: d.index,
                name: d.name.clone(),
                protocols_mask: d.protocols,
                protocols: proto::names(d.protocols),
                powered: d.powered,
                rf_mode: d.rf_mode,
                rfkill_soft_blocked: soft,
                rfkill_hard_blocked: hard,
            });
            st.capabilities = capabilities_for(st.available);
            st.message = if st.available { String::new() } else { "未发现 NFC 控制器（nfc0）".to_string() };
            st.available
        }
        Err(e) => {
            let msg = format!("读取 NFC 设备失败：{e}");
            let mut st = lock(&shared.status);
            st.available = false;
            st.device = None;
            st.message = msg.clone();
            st.capabilities = capabilities_for(false);
            st.last_error = Some(msg);
            false
        }
    }
}

/// 读取设备与安全单元，刷新状态
fn refresh_all(nl: &mut Netlink, shared: &Arc<Shared>) {
    if !refresh_device(nl, shared) {
        return;
    }
    let ses = current_device(shared).map(|d| nl.get_ses(d).unwrap_or_default()).unwrap_or_default();
    lock(&shared.status).secure_elements = ses;
}

/// rfkill 软/硬阻塞状态（读不到就当未阻塞）
fn rfkill_state() -> (bool, bool) {
    let mut soft = false;
    let mut hard = false;
    if let Ok(entries) = std::fs::read_dir("/sys/class/rfkill") {
        for e in entries.flatten() {
            let p = e.path();
            let name = std::fs::read_to_string(p.join("name")).unwrap_or_default();
            if !name.to_lowercase().contains("nfc") {
                continue;
            }
            let read_flag = |f: &str| std::fs::read_to_string(p.join(f)).map(|v| v.trim() == "1").unwrap_or(false);
            soft |= read_flag("soft");
            hard |= read_flag("hard");
        }
    }
    (soft, hard)
}

/// 处理一条命令
fn handle_cmd(nl: &mut Netlink, shared: &Arc<Shared>, cmd: Cmd) {
    match cmd {
        Cmd::SetPower { up, reply } => {
            let dev = match current_device(shared) {
                Some(d) => d,
                None => {
                    let _ = reply.send(Err("未发现 NFC 控制器".to_string()));
                    return;
                }
            };
            let r = if up { nl.dev_up(dev) } else { nl.dev_down(dev) };
            match r {
                Ok(()) => {
                    set_op(shared, if up { "控制器上电" } else { "控制器下电" });
                    if !up {
                        let mut st = lock(&shared.status);
                        st.polling = false;
                        st.listening = false;
                        lock(&shared.tags).clear();
                    }
                    // 上电后立刻刷新状态（DEVICE_POWERED 属性）
                    refresh_all(nl, shared);
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    let msg = format!("{}失败：{e}", if up { "上电" } else { "下电" });
                    set_error(shared, &msg);
                    let _ = reply.send(Err(msg));
                }
            }
        }
        Cmd::StartPoll { protocols, reply } => {
            let dev = match current_device(shared) {
                Some(d) => d,
                None => {
                    let _ = reply.send(Err("未发现 NFC 控制器".to_string()));
                    return;
                }
            };
            // 默认轮询所有发起方协议（NFC-DEP 一起开，便于等待点对点对端）
            let names = if protocols.is_empty() {
                vec![
                    "iso14443a".to_string(),
                    "mifare".to_string(),
                    "jewel".to_string(),
                    "iso14443b".to_string(),
                    "felica".to_string(),
                    "nfc_dep".to_string(),
                ]
            } else {
                protocols
            };
            let mut mask = 0u32;
            let mut effective = Vec::new();
            for n in &names {
                match proto::from_name(n) {
                    Some(p) => {
                        mask |= 1 << p;
                        effective.push(n.clone());
                    }
                    None => {
                        let _ = reply.send(Err(format!("未知协议名：{n}")));
                        return;
                    }
                }
            }
            // 保证控制器上电
            if !lock(&shared.status).device.as_ref().map(|d| d.powered).unwrap_or(false) {
                if let Err(e) = nl.dev_up(dev) {
                    // 已经上电时内核返回 EALREADY/EINPROGRESS，忽略
                    let raw = e.raw_os_error().unwrap_or(0);
                    if raw != 114 && raw != 16 {
                        let msg = format!("控制器上电失败：{e}");
                        set_error(shared, &msg);
                        let _ = reply.send(Err(msg));
                        return;
                    }
                }
            }
            // 轮询与监听共用同一个 RF_DISCOVER：先无条件停一次，让 NFCC 回到干净状态再开始。
            // 否则残留的 discovery 会让新的 RF_DISCOVER 被 NFCC 忽略（内核仍返回 0），
            // 表现为「轮询中但永远等不到 TARGETS_FOUND」。
            let _ = nl.stop_poll(dev);
            std::thread::sleep(Duration::from_millis(150));
            {
                let mut st = lock(&shared.status);
                st.listening = false;
            }
            match nl.start_poll(dev, mask, 0) {
                Ok(()) => {
                    {
                        let mut st = lock(&shared.status);
                        st.polling = true;
                        st.listening = false;
                        st.last_error = None;
                    }
                    // 卡片缓存只由 TARGETS_FOUND 事件填充：内核的 target 列表在
                    // 轮询结束后仍保留上一轮的旧条目，dump 出来会把旧卡当新卡。
                    lock(&shared.tags).clear();
                    refresh_device(nl, shared);
                    set_op(shared, &format!("开始轮询（{}）", effective.join("+")));
                    shared.event(
                        "poll",
                        format!("开始轮询：{}（等待卡片进入感应区）", effective.join(" + ")),
                        serde_json::json!({ "protocols": effective }),
                    );
                    let _ = reply.send(Ok(effective));
                }
                Err(e) => {
                    let msg = format!("启动轮询失败：{e}");
                    set_error(shared, &msg);
                    let _ = reply.send(Err(msg));
                }
            }
        }
        Cmd::StopPoll(reply) => {
            let dev = current_device(shared).unwrap_or(0);
            let r = nl.stop_poll(dev);
            let mut st = lock(&shared.status);
            st.polling = false;
            st.listening = false;
            drop(st);
            lock(&shared.tags).clear();
            set_op(shared, "停止轮询");
            shared.event("poll", "停止轮询", serde_json::json!({}));
            // 内核认为「本来就没在轮询/监听」时不算失败（EINVAL=22 / EALREADY=114 / EBUSY=16）
            let _ = reply.send(match r {
                Ok(()) => Ok(()),
                Err(e) if matches!(e.raw_os_error(), Some(22) | Some(114) | Some(16)) => Ok(()),
                Err(e) => {
                    let msg = format!("停止轮询失败：{e}");
                    set_error(shared, &msg);
                    Err(msg)
                }
            });
        }
        Cmd::StartListen(reply) => {
            let dev = match current_device(shared) {
                Some(d) => d,
                None => {
                    let _ = reply.send(Err("未发现 NFC 控制器".to_string()));
                    return;
                }
            };
            if let Err(e) = nl.dev_up(dev) {
                let raw = e.raw_os_error().unwrap_or(0);
                if raw != 114 && raw != 16 {
                    let msg = format!("控制器上电失败：{e}");
                    set_error(shared, &msg);
                    let _ = reply.send(Err(msg));
                    return;
                }
            }
            // 监听与轮询共用同一个 RF_DISCOVER：正在轮询时先停掉，否则内核返回 EBUSY
            if lock(&shared.status).polling {
                let _ = nl.stop_poll(dev);
                std::thread::sleep(Duration::from_millis(150));
                lock(&shared.status).polling = false;
            }
            // 目标模式：只监听 NFC-DEP（内核 nci 唯一支持的 listen 组合）
            let mut started = nl.start_poll(dev, 0, proto::NFC_DEP_MASK);
            if let Err(e) = &started {
                if e.raw_os_error() == Some(16) {
                    // 上一次 RF_DEACTIVATE 还没完成，稍等重试一次
                    std::thread::sleep(Duration::from_millis(250));
                    started = nl.start_poll(dev, 0, proto::NFC_DEP_MASK);
                }
            }
            match started {
                Ok(()) => {
                    {
                        let mut st = lock(&shared.status);
                        st.listening = true;
                        st.polling = false;
                        st.last_error = None;
                    }
                    set_op(shared, "进入 NFC-DEP 监听");
                    shared.event(
                        "emulate",
                        "进入 NFC-DEP 监听（点对点目标模式，非卡模拟）",
                        serde_json::json!({ "protocols": ["nfc_dep"] }),
                    );
                    let _ = reply.send(Ok(()));
                }
                Err(e) => {
                    let msg = format!("开启监听失败：{e}");
                    set_error(shared, &msg);
                    let _ = reply.send(Err(msg));
                }
            }
        }
        Cmd::StopListen(reply) => {
            let dev = current_device(shared).unwrap_or(0);
            let r = nl.stop_poll(dev);
            {
                let mut st = lock(&shared.status);
                st.listening = false;
                st.polling = false;
            }
            set_op(shared, "停止监听");
            shared.event("emulate", "停止监听", serde_json::json!({}));
            let _ = reply.send(r.map_err(|e| format!("停止监听失败：{e}")));
        }
        Cmd::Read { req, reply } => {
            let out = do_read(nl, shared, &req);
            if out.is_err() {
                if let Err(e) = &out {
                    set_error(shared, e);
                }
            }
            let _ = reply.send(out);
            resume_poll(nl, shared);
        }
        Cmd::Write { req, reply } => {
            let out = do_write(nl, shared, &req);
            if let Err(e) = &out {
                set_error(shared, e);
            }
            let _ = reply.send(out);
            resume_poll(nl, shared);
        }
        Cmd::Raw { req, reply } => {
            let out = do_raw(shared, &req);
            if let Err(e) = &out {
                set_error(shared, e);
            }
            let _ = reply.send(out);
            resume_poll(nl, shared);
        }
    }
}

/// 原始帧操作结束后，如果用户仍在轮询，重新拉起轮询
fn resume_poll(nl: &mut Netlink, shared: &Arc<Shared>) {
    let (polling, dev) = {
        let st = lock(&shared.status);
        (st.polling, st.device.as_ref().map(|d| d.index))
    };
    if !polling {
        return;
    }
    let Some(dev) = dev else { return };
    let mask = proto::JEWEL_MASK
        | proto::MIFARE_MASK
        | proto::ISO14443A_MASK
        | proto::ISO14443B_MASK
        | proto::FELICA_MASK
        | proto::NFC_DEP_MASK;
    for attempt in 0..3 {
        if nl.start_poll(dev, mask, 0).is_ok() {
            // 新会话里内核的旧 target 列表已失效，等新的 TARGETS_FOUND 再填缓存
            lock(&shared.tags).clear();
            return;
        }
        std::thread::sleep(Duration::from_millis(120 * (attempt + 1)));
    }
    shared.event("warn", "重新开始轮询失败（卡片可能仍处于激活状态）", serde_json::json!({}));
}

/// 排空内核事件
fn pump_events(nl: &mut Netlink, shared: &Arc<Shared>) {
    for _ in 0..8 {
        match nl.next_event(Duration::from_millis(0)) {
            Ok(Some(ev)) => handle_event(nl, shared, ev),
            Ok(None) => break,
            Err(_) => break,
        }
    }
}

fn handle_event(nl: &mut Netlink, shared: &Arc<Shared>, ev: NfcEvent) {
    match ev {
        NfcEvent::TargetsFound => {
            refresh_targets(nl, shared);
            let n = lock(&shared.tags).len();
            shared.event("targets", format!("发现 {n} 张卡片"), serde_json::json!({ "count": n }));
        }
        NfcEvent::TargetLost => {
            refresh_targets(nl, shared);
            shared.event("targets", "卡片离开感应区", serde_json::json!({}));
        }
        NfcEvent::TmActivated { protocol, comm_mode } => {
            {
                let mut st = lock(&shared.status);
                st.listening = true;
            }
            shared.event(
                "emulate",
                format!(
                    "NFC-DEP 被激活（协议 {}，通信模式 {}）",
                    proto::name(protocol),
                    if comm_mode == 0 { "主动" } else { "被动" }
                ),
                serde_json::json!({ "protocol": proto::name(protocol), "comm_mode": comm_mode }),
            );
        }
        NfcEvent::TmDeactivated => {
            shared.event("emulate", "NFC-DEP 连接结束", serde_json::json!({}));
        }
        NfcEvent::DeviceAdded | NfcEvent::DeviceRemoved => {
            refresh_all(nl, shared);
            shared.event("device", "NFC 控制器状态变化", serde_json::json!({}));
        }
        NfcEvent::SeAdded | NfcEvent::SeRemoved | NfcEvent::SeTransaction => {
            refresh_all(nl, shared);
            shared.event("se", "安全单元状态变化", serde_json::json!({}));
        }
        NfcEvent::Other(c) => {
            shared.event("other", format!("内核事件 {c}"), serde_json::json!({ "cmd": c }));
        }
    }
}

/// 重新 dump 卡片列表并写入共享状态
fn refresh_targets(nl: &mut Netlink, shared: &Arc<Shared>) {
    let dev = match current_device(shared) {
        Some(d) => d,
        None => return,
    };
    let targets = nl.get_targets(dev).unwrap_or_default();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let new_tags: Vec<TagView> = targets.iter().map(|t| tag_view(t, &now)).collect();
    // 旧卡片若这次没出现，标记为已丢失（保留展示，便于看历史）
    let mut merged: Vec<TagView> = new_tags;
    let old = lock(&shared.tags).clone();
    for mut o in old {
        if !merged.iter().any(|t| t.index == o.index && t.uid == o.uid) {
            o.lost = true;
            merged.push(o);
        }
    }
    *lock(&shared.tags) = merged;
}

/// 卡片标识：ISO14443-A 用 NFCID1，FeliCa 用 IDm，ISO14443-B 用 PUPI，ISO15693 用 UID
fn uid_of(t: &TargetInfo) -> String {
    if !t.nfcid1.is_empty() {
        return tag::hex_colon(&t.nfcid1);
    }
    if t.sensf_res.len() >= 10 {
        // SENSF_RES：0x01 0x02 + IDm(8)
        return tag::hex_colon(&t.sensf_res[2..10]);
    }
    if t.sensb_res.len() >= 5 {
        // SENSB_RES：0x50 + PUPI(4)
        return tag::hex_colon(&t.sensb_res[1..5]);
    }
    tag::hex_colon(&t.iso15693_uid)
}

fn tag_view(t: &TargetInfo, seen_at: &str) -> TagView {
    let best = t.best_protocol().map(proto::name).unwrap_or("unknown").to_string();
    TagView {
        index: t.index,
        protocol: best.clone(),
        protocols: proto::names(t.protocols),
        uid: uid_of(t),
        atqa: t.sens_res.map(|v| format!("{v:04x}")).unwrap_or_default(),
        sak: t.sel_res,
        ats: if t.ats.is_empty() { None } else { Some(tag::hex(&t.ats)) },
        technology: technology_of(t),
        readable: t.protocols & proto::NFC_DEP_MASK == 0,
        best_protocol: best,
        seen_at: seen_at.to_string(),
        lost: false,
    }
}

/// 由 ATQA/SAK 推断卡片技术类型（展示用）
fn technology_of(t: &TargetInfo) -> String {
    if t.protocols & proto::ISO14443B_MASK != 0 {
        return "ISO14443-B".to_string();
    }
    if t.protocols & proto::FELICA_MASK != 0 {
        return "FeliCa".to_string();
    }
    if t.protocols & proto::JEWEL_MASK != 0 {
        return "Jewel / Topaz".to_string();
    }
    match t.sel_res {
        Some(sak) => {
            if sak & 0x20 != 0 {
                return format!("ISO14443-4 / ISO-DEP（SAK {sak:02X}）");
            }
            match sak {
                0x00 => "MIFARE Ultralight / NTAG（Type 2）".to_string(),
                0x08 => "MIFARE Classic 1K".to_string(),
                0x18 => "MIFARE Classic 4K".to_string(),
                0x09 => "MIFARE Mini".to_string(),
                0x28 => "SmartMX（JCOP）".to_string(),
                0x38 => "MIFARE Plus SL2".to_string(),
                other => format!("ISO14443-A（SAK {other:02X}）"),
            }
        }
        None => "NFC-DEP 对端".to_string(),
    }
}

fn current_device(shared: &Arc<Shared>) -> Option<u32> {
    lock(&shared.status).device.as_ref().map(|d| d.index)
}

fn set_op(shared: &Arc<Shared>, op: &str) {
    lock(&shared.status).last_op = Some(op.to_string());
}

fn set_error(shared: &Arc<Shared>, msg: &str) {
    let mut st = lock(&shared.status);
    st.last_error = Some(msg.to_string());
    drop(st);
    shared.event("error", msg.to_string(), serde_json::json!({}));
}

// ───────────────────────── 读 / 写 / 原始帧 ─────────────────────────

/// 取本次轮询会话里发现到的卡片
///
/// 只认 `TARGETS_FOUND` 事件填充的缓存：内核的 `dev->targets` 在轮询结束后仍留着
/// 上一轮的旧条目，直接 dump 会把已经离开感应区的旧卡当成「当前卡片」，
/// 而且拿旧 `target_idx` 去 connect 一定失败（NCI 状态已不是 W4_HOST_SELECT）。
fn cached_target(shared: &Arc<Shared>, index: u32) -> Result<TagView, String> {
    let tags = shared.tags();
    if tags.is_empty() {
        return Err("当前没有卡片：请点「开始轮询」并把卡片贴住手机背面，等列表里出现卡片再操作".to_string());
    }
    let t = tags
        .iter()
        .find(|t| t.index == index && !t.lost)
        .ok_or_else(|| format!("卡片 index={index} 不在本次发现列表里（已离开感应区或轮询已重启）"))?;
    Ok(t.clone())
}

/// 解析请求协议（空/auto 时用卡片的最佳协议），并校验卡片是否支持
fn resolve_protocol_view(t: &TagView, want: &str) -> Result<u32, String> {
    let name = if want.is_empty() || want == "auto" { t.best_protocol.as_str() } else { want };
    let p = proto::from_name(name).ok_or_else(|| format!("未知协议名：{name}"))?;
    if !t.protocols.iter().any(|n| n == name) {
        return Err(format!("卡片不支持协议 {name}（支持：{}）", t.protocols.join("、")));
    }
    Ok(p)
}

fn do_read(_nl: &mut Netlink, shared: &Arc<Shared>, req: &ReadReq) -> Result<ReadResult, String> {
    let dev = current_device(shared).ok_or("未发现 NFC 控制器")?;
    let tag = cached_target(shared, req.target_index)?;
    let protocol = resolve_protocol_view(&tag, &req.protocol)?;
    let tech = tag.technology.clone();
    let raw = tag::RawTag::connect(dev, tag.index, protocol).map_err(|e| format!("激活卡片失败：{e}"))?;

    let mut result = ReadResult { uid: tag.uid.clone(), technology: tech.clone(), ..Default::default() };
    set_op(shared, &format!("读取卡片 #{}", tag.index));

    // 1) 先试 Type 2（页 0 的 CC 魔数 E1 是最可靠判据）
    let mut tlv_log = Vec::new();
    let first = tag::type2_read_logged(&raw, 0, &mut tlv_log);
    if let Ok(first16) = first {
        result.raw_exchanges.extend(tlv_log.clone());
        let is_type2 = first16.len() == 16 && first16[12] == 0xE1;
        if req.mode == "pages" {
            let start = req.start_page.unwrap_or(0);
            let count = req.page_count.unwrap_or(16).clamp(1, 64);
            let mut buf = Vec::new();
            let mut page = start;
            while buf.len() < count as usize * 4 {
                let chunk =
                    if page == 0 && start == 0 { Some(first16.clone()) } else { tag::type2_read(&raw, page).ok() };
                match chunk {
                    Some(c) => buf.extend_from_slice(&c),
                    None => break,
                }
                page = page.wrapping_add(4);
            }
            result.pages = Some(tag::page_dump(start, &buf[..buf.len().min(count as usize * 4)]));
            result.note = if is_type2 {
                format!(
                    "Type 2 页转储（每页 4 字节，容量 {}）",
                    tag::parse_type2_cc(&first16[12..16]).map(|c| c.size_bytes).unwrap_or(0)
                )
            } else {
                "原始页/块转储：MIFARE Classic 的 READ 返回 16 字节/块（此卡若需要密钥会失败）".to_string()
            };
            return Ok(result);
        }
        if is_type2 {
            // 完整 NDEF 读取
            let (buf, cc, message, log) = tag::type2_dump(&raw, 64).map_err(|e| format!("读取 Type 2 失败：{e}"))?;
            result.raw_exchanges = log;
            result.pages = Some(tag::page_dump(0, &buf));
            if let Some(msg) = message {
                if !msg.is_empty() {
                    result.ndef = Some(ndef::parse_message(&msg)?);
                }
            }
            result.note = format!(
                "Type 2（NTAG/Ultralight）容量 {} 字节，CC {}",
                cc.as_ref().map(|c| c.size_bytes).unwrap_or(0),
                cc.as_ref().map(|c| c.magic.clone()).unwrap_or_default()
            );
            return Ok(result);
        }
        // ATQA/SAK 像 ISO14443-4 时走 APDU
        if tag.sak.map(|s| s & 0x20 != 0).unwrap_or(false) {
            let (msg, log) = tag::type4_read_ndef(&raw).map_err(|e| format!("读取 Type 4/ISO-DEP 失败：{e}"))?;
            result.raw_exchanges = log;
            if !msg.is_empty() {
                result.ndef = Some(ndef::parse_message(&msg)?);
            }
            result.note = "ISO14443-4（ISO-DEP）通过 APDU 读取 NDEF 文件".to_string();
            return Ok(result);
        }
        // 剩下的是 MIFARE Classic 之类：READ 通了说明卡允许免认证读块
        result.pages = Some(tag::page_dump(0, &first16));
        result.note = format!(
            "{tech}：READ 命令无认证可用，已转储首块；MIFARE Classic 的扇区数据需要 CRYPTO1 密钥认证，本页暂不支持（可用原始帧控制台手动发送 60/61 认证帧）"
        );
        return Ok(result);
    }

    // 2) READ 失败：可能是 ISO14443-4，或 MIFARE Classic 需要认证
    if tag.sak.map(|s| s & 0x20 != 0).unwrap_or(false) {
        let (msg, log) = tag::type4_read_ndef(&raw).map_err(|e| format!("读取 Type 4/ISO-DEP 失败：{e}"))?;
        result.raw_exchanges = log;
        if !msg.is_empty() {
            result.ndef = Some(ndef::parse_message(&msg)?);
        }
        result.note = "ISO14443-4（ISO-DEP）通过 APDU 读取 NDEF 文件".to_string();
        return Ok(result);
    }
    let err = first.unwrap_err();
    Err(format!(
        "无法读取该卡（{tech}）：{err}。MIFARE Classic 需要 CRYPTO1 密钥认证，本页暂不支持自动读取；\
         可用原始帧控制台手动尝试"
    ))
}

fn do_write(_nl: &mut Netlink, shared: &Arc<Shared>, req: &WriteReq) -> Result<WriteResult, String> {
    let dev = current_device(shared).ok_or("未发现 NFC 控制器")?;
    let target = cached_target(shared, req.target_index)?;
    // 构造 NDEF 消息
    let (message, template) = match req.kind.as_str() {
        "text" => {
            let lang = if req.lang.is_empty() { "en" } else { req.lang.as_str() };
            (
                ndef::message_from_record(ndef::text_record(&req.value, lang)),
                format!("文本记录（语言 {lang}）"),
            )
        }
        "uri" => (ndef::message_from_record(ndef::uri_record(&req.value)), "URI 记录".to_string()),
        "raw" => {
            let payload = if !req.raw_hex.is_empty() { req.raw_hex.clone() } else { req.value.clone() };
            (
                ndef::message_from_record(ndef::raw_record(1, "T", &payload)?),
                "原始 NDEF 记录（TNF=1，类型 T）".to_string(),
            )
        }
        other => return Err(format!("未知写入类型：{other}")),
    };
    if message.is_empty() {
        return Err("内容为空".to_string());
    }

    let proto_name = if req.protocol.is_empty() || req.protocol == "auto" {
        target.best_protocol.clone()
    } else {
        req.protocol.clone()
    };
    let protocol = proto::from_name(&proto_name).ok_or_else(|| format!("卡片协议 {proto_name} 不支持写入"))?;
    let raw = tag::RawTag::connect(dev, target.index, protocol).map_err(|e| format!("激活卡片失败：{e}"))?;
    set_op(shared, &format!("写入卡片 #{}", target.index));

    // 判断卡型：先按 Type 2 读 CC
    let probe = tag::type2_read(&raw, 0);
    match probe {
        Ok(first16) if first16.len() == 16 && first16[12] == 0xE1 => {
            let cc = tag::parse_type2_cc(&first16[12..16]).ok_or("CC 页解析失败（不是 Type 2 标签）")?;
            if !cc.writable {
                return Err(format!("该标签 NDEF 区为只读（CC access=0x{:02x}）", cc.access));
            }
            let wrote =
                tag::type2_write_ndef(&raw, &message, cc.size_bytes).map_err(|e| format!("写入 Type 2 失败：{e}"))?;
            // 回读校验
            let (_buf, _cc, back, _log) = tag::type2_dump(&raw, 64).map_err(|e| format!("回读校验失败：{e}"))?;
            let verify = back.map(|m| VerifyResult {
                raw_hex: tag::hex(&m),
                records: ndef::parse_message(&m).map(|p| p.records.len()).unwrap_or(0),
            });
            Ok(WriteResult {
                wrote_bytes: wrote,
                template: format!("{template} → Type 2（容量 {} 字节，页 4 起）", cc.size_bytes),
                verify,
            })
        }
        _ => {
            let sak = target.sak.unwrap_or(0);
            if sak & 0x20 != 0 {
                let capacity = tag::type4_capacity(&raw).unwrap_or(0);
                let wrote =
                    tag::type4_write_ndef(&raw, &message, capacity).map_err(|e| format!("写入 Type 4 失败：{e}"))?;
                let verify = tag::type4_read_ndef(&raw).ok().map(|(m, _)| VerifyResult {
                    raw_hex: tag::hex(&m),
                    records: ndef::parse_message(&m).map(|p| p.records.len()).unwrap_or(0),
                });
                Ok(WriteResult {
                    wrote_bytes: wrote,
                    template: format!("{template} → Type 4（NDEF 文件容量 {capacity} 字节）"),
                    verify,
                })
            } else {
                Err(format!(
                    "{} 暂不支持自动写入：MIFARE Classic 需要 CRYPTO1 密钥认证；\
                     可在原始帧控制台手动发送认证与写块帧",
                    target.technology
                ))
            }
        }
    }
}

fn do_raw(shared: &Arc<Shared>, req: &RawReq) -> Result<RawResult, String> {
    let dev = current_device(shared).ok_or("未发现 NFC 控制器")?;
    let tx = tag::unhex(&req.tx_hex)?;
    if tx.is_empty() {
        return Err("发送内容为空".to_string());
    }
    // 用共享列表里的卡片协议（raw 不走 netlink dump，避免依赖轮询状态）
    let target = cached_target(shared, req.target_index)?;
    let protocol = if req.protocol.is_empty() || req.protocol == "auto" {
        proto::from_name(&target.best_protocol)
            .ok_or_else(|| format!("卡片协议 {} 不支持原始帧", target.best_protocol))?
    } else {
        proto::from_name(&req.protocol).ok_or_else(|| format!("未知协议名：{}", req.protocol))?
    };
    let raw = tag::RawTag::connect(dev, target.index, protocol).map_err(|e| format!("激活卡片失败：{e}"))?;
    set_op(shared, &format!("原始帧 → 卡片 #{}", target.index));
    let timeout = Duration::from_millis(req.timeout_ms.clamp(50, 5000));
    let started = Instant::now();
    let mut out = RawResult { tx: tag::hex(&tx), ..Default::default() };
    match raw.transceive(&tx, timeout) {
        Ok(rx) => {
            out.rx = tag::hex(&rx);
        }
        Err(e) => {
            out.error = Some(e.to_string());
        }
    }
    out.elapsed_ms = started.elapsed().as_millis() as u64;
    shared.event(
        "raw",
        format!(
            "原始帧 {} → {}（{} ms）",
            out.tx,
            if out.rx.is_empty() { "无响应".into() } else { out.rx.clone() },
            out.elapsed_ms
        ),
        serde_json::json!({ "tx": out.tx, "rx": out.rx, "error": out.error }),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 技术类型推断() {
        let mut t = TargetInfo { protocols: proto::MIFARE_MASK, sel_res: Some(0x08), ..Default::default() };
        assert_eq!(technology_of(&t), "MIFARE Classic 1K");
        t.sel_res = Some(0x00);
        assert_eq!(technology_of(&t), "MIFARE Ultralight / NTAG（Type 2）");
        t.sel_res = Some(0x20);
        assert!(technology_of(&t).starts_with("ISO14443-4"));
        let f = TargetInfo { protocols: proto::FELICA_MASK, ..Default::default() };
        assert_eq!(technology_of(&f), "FeliCa");
    }

    #[test]
    fn 卡片视图带冒号_uid() {
        let t = TargetInfo {
            index: 3,
            protocols: proto::MIFARE_MASK,
            sel_res: Some(0x08),
            sens_res: Some(0x0004),
            nfcid1: vec![0xdb, 0x4c, 0x1b, 0x5a],
            ..Default::default()
        };
        let v = tag_view(&t, "2026-01-01 00:00:00");
        assert_eq!(v.uid, "db:4c:1b:5a");
        assert_eq!(v.atqa, "0004");
        assert_eq!(v.best_protocol, "mifare");
        assert_eq!(v.protocols, vec!["mifare"]);
        assert!(v.readable);
    }

    #[test]
    fn 初始状态不可用且能力有说明() {
        let s = initial_status();
        assert!(!s.available);
        // UID 卡模拟走用户态直驱，不依赖内核 nfc0 是否在
        assert!(s.capabilities.emulate_uid.ok);
        assert!(!s.capabilities.emulate_uid.reason.is_empty());
    }
}
