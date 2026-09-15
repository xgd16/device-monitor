//! DRM/KMS 显示输出：取得 master、设置 CRTC 模式、分配 dumb 扫描缓冲。
//!
//! 本设备 MSM/DPU 的 plane 只支持 `rotate-0/180`（+reflect），无 90/270，
//! 因此旋转由渲染层的预旋转字形 + 坐标映射完成，硬件只负责扫描输出。
//!
//! 缓冲为单缓冲直写：刷新只推脏矩形（每帧通常几十 KB），撕裂窗口极小。

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, BorrowedFd};

use drm::Device as BasicDevice;
use drm::buffer::Buffer;
use std::sync::atomic::{AtomicBool, Ordering};

use drm::control::{
    ClipRect, Device as ControlDevice, Mode, PageFlipFlags, connector, crtc, dumbbuffer::DumbBuffer,
    dumbbuffer::DumbMapping, framebuffer,
};
use drm_fourcc::DrmFourcc;

/// 实现 drm crate 所需的两个 trait。
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl BasicDevice for Card {}

impl ControlDevice for Card {}

/// 已就绪的扫描输出。
pub struct Display {
    pub pw: u32,
    pub ph: u32,
    pub pitch: usize,
    pub connector: String,
    pub mode: String,
    /// 双缓冲：绘制用的 mmap 映射（其内部 &mut 由 Box::leak 保活）
    slots: [DumbMapping<'static>; 2],
    fbs: [framebuffer::Handle; 2],
    /// 当前用于绘制的 slot（提交后与另一个 slot 交换）
    cur: usize,
    card: Card,
    fb: framebuffer::Handle,
    crtc: crtc::Handle,
    conn: connector::Handle,
    drm_mode: Mode,
}

impl Display {
    /// 打开第一个已连接的输出并用其首选模式点亮。
    pub fn open() -> Result<Self, String> {
        let path = "/dev/dri/card0";
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("打开 {path} 失败: {e}"))?;
        let card = Card(file);

        // 取 DRM master：打开主节点时内核通常已自动授予；若失败（kmscon/fbcon 占用）
        // 不立即退出，交给后面的 set_crtc 暴露真实原因
        if let Err(e) = card.acquire_master_lock() {
            tracing::warn!("screen: acquire_master_lock 失败（{e}），尝试直接设置模式");
        }

        let res = card.resource_handles().map_err(|e| format!("resource_handles: {e}"))?;

        let mut picked: Option<(connector::Handle, Mode, crtc::Handle, String)> = None;
        for &ch in res.connectors() {
            let info = match card.get_connector(ch, false) {
                Ok(i) => i,
                Err(_) => continue,
            };
            if info.state() != connector::State::Connected {
                continue;
            }
            let Some(&mode) = info.modes().first() else { continue };
            let mut target_crtc: Option<crtc::Handle> = None;
            for &eh in info.encoders() {
                if let Ok(e) = card.get_encoder(eh) {
                    if let Some(c) = e.crtc() {
                        target_crtc = Some(c);
                        break;
                    }
                    // 编码器尚未绑定 CRTC：用它的 possible_crtcs 位掩码过滤
                    if let Some(c) = res.filter_crtcs(e.possible_crtcs()).first().copied() {
                        target_crtc = Some(c);
                        break;
                    }
                }
            }
            if target_crtc.is_none() {
                target_crtc = res.crtcs().first().copied();
            }
            let Some(c) = target_crtc else { continue };
            let name = format!("{}-{}", if info.interface() == connector::Interface::DSI { "DSI" } else { "CONN" }, u32::from(ch));
            picked = Some((ch, mode, c, name));
            break;
        }

        let (conn, mode, crtc_h, conn_name) =
            picked.ok_or_else(|| "没有找到已连接且带模式的显示输出".to_string())?;

        let (pw, ph) = mode.size();
        let (pw, ph) = (pw as u32, ph as u32);
        if pw == 0 || ph == 0 {
            return Err("显示模式尺寸为 0".to_string());
        }

        // 双缓冲：CMD 模式面板要靠 page flip 才会把帧推上去，单缓冲翻不了
        let mut slots: Vec<DumbMapping<'static>> = Vec::new();
        let mut fbs: Vec<framebuffer::Handle> = Vec::new();
        let mut pitch = 0usize;
        for _ in 0..2 {
            let db = card
                .create_dumb_buffer((pw, ph), DrmFourcc::Argb8888, 32)
                .map_err(|e| format!("create_dumb_buffer({pw}x{ph}): {e}"))?;
            pitch = db.pitch() as usize;
            let fbi = card
                .add_framebuffer(&db, 32, 32)
                .map_err(|e| format!("add_framebuffer: {e}"))?;
            // 泄漏 DumbBuffer 使其活到进程结束（也避免自引用结构）：
            // 把 &'static mut 直接移交给 DumbMapping，之后不再持有第二个可变借用
            let db: &'static mut DumbBuffer = Box::leak(Box::new(db));
            let map = card
                .map_dumb_buffer(db)
                .map_err(|e| format!("map_dumb_buffer: {e}"))?;
            slots.push(map);
            fbs.push(fbi);
        }
        let slots: [DumbMapping<'static>; 2] = slots.try_into().map_err(|_| "双缓冲初始化失败".to_string())?;
        let fbs: [framebuffer::Handle; 2] = fbs.try_into().map_err(|_| "双缓冲初始化失败".to_string())?;
        let fb = fbs[0];

        card.set_crtc(crtc_h, Some(fb), (0, 0), &[conn], Some(mode))
            .map_err(|e| format!("set_crtc: {e}"))?;

        let mode_str = format!("{}x{}", pw, ph);
        tracing::info!("screen: DRM 输出 {} {} pitch={}", conn_name, mode_str, pitch);

        Ok(Self {
            pw,
            ph,
            pitch,
            connector: conn_name,
            mode: mode_str,
            slots,
            fbs,
            cur: 1, // 0 号已上屏，先从 1 号画起
            card,
            fb,
            crtc: crtc_h,
            conn,
            drm_mode: mode,
        })
    }

    /// 提交一次帧更新。
    ///
    /// 这块面板是 **CMD 模式** DSI（`dsi_samsung_fhd_ea8076_cmd_display`），不像 video 模式
    /// 那样持续扫描内存：改完 dumb buffer 必须显式提交，否则屏上永远是开局那张（全黑）。
    /// 这里优先用 damage 上报（dirtyfb），失败则回退到整屏 page flip 之外的日志提示。
    pub fn commit(&mut self) -> Result<(), String> {
        static WARNED: AtomicBool = AtomicBool::new(false);
        let target = self.fbs[self.cur];
        // 主路径：page flip（kmscon 当年就是靠它把帧推到这块 CMD 面板的）
        match self.card.page_flip(self.crtc, target, PageFlipFlags::empty(), None) {
            Ok(()) => {
                self.fb = target;
                self.cur ^= 1;
                return Ok(());
            }
            Err(e) => {
                // 回退：damage 上报
                let clip = ClipRect::new(0, 0, self.pw.min(u16::MAX as u32) as u16, self.ph.min(u16::MAX as u32) as u16);
                if self.card.dirty_framebuffer(target, &[clip]).is_ok() {
                    self.fb = target;
                    self.cur ^= 1;
                    return Ok(());
                }
                if !WARNED.swap(true, Ordering::Relaxed) {
                    tracing::error!("screen: 提交失败（page flip 与 dirtyfb 都不可用）：{e}");
                }
                Err(format!("page_flip/dirtyfb: {e}"))
            }
        }
    }

    /// 当前 CRTC 上挂着的 framebuffer（用于自愈判断）。
    pub fn crtc_framebuffer(&self) -> Option<framebuffer::Handle> {
        self.card.get_crtc(self.crtc).ok().and_then(|i| i.framebuffer())
    }

    /// 我们的 framebuffer 句柄。
    pub fn framebuffer(&self) -> framebuffer::Handle {
        self.fb
    }

    /// 重新提交模式。
    ///
    /// msm DSI 把背光 `bl_power=4`（电源键灭屏）当 DPMS 用：会关掉输出，**写回 0 不会自动恢复**。
    /// 幂等地重跑一次 set_crtc 即可把画面要回来。
    pub fn reassert(&self) -> Result<(), String> {
        self.card
            .set_crtc(self.crtc, Some(self.fb), (0, 0), &[self.conn], Some(self.drm_mode))
            .map_err(|e| format!("重新提交模式失败: {e}"))
    }

    /// 当前绘制 slot 的可写像素缓冲（`0xAARRGGBB` 小端，长度 = pitch * ph）。
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        let i = self.cur;
        &mut self.slots[i][..]
    }
}
