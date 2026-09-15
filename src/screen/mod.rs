//! 物理屏横屏仪表：DRM/KMS 直绘 + 软件 90° 旋转 + CJK 灰度抗锯齿排版。
//!
//! 与原有 `tui` 模块的关系：
//! - `tui`：写 ANSI 转义到 tty（依赖 kmscon 提供 CJK 字体），竖屏字符网格布局；
//! - `screen`：自己接管 DRM 输出并用像素级排版渲染**横屏卡片仪表**，
//!   文本清晰度不字符网格限制（数值可到 62px），可画真实折线/胶囊/圆角卡片。
//!
//! 旋转说明：内核 plane 仅支持 `rotate-0/180`，无 90/270，故旋转在渲染层完成
//! （字形预旋转 + 坐标映射），硬件只做扫描输出。
//! - `Rot90`：手机**顺时针转 90°**（左侧边朝上）观看
//! - `Rot270`：反向（若画面上下颠倒，用 `--rotate 270`）

pub mod canvas;
pub mod display;
pub mod font;
pub mod layout;
pub mod theme;
pub mod token;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::collector::SystemOverview;
use crate::store::Database;

pub use canvas::{Canvas, Rect};
pub use display::Display;

/// 旋转方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    Rot90,
    Rot270,
}

impl Rotation {
    /// 逻辑画布尺寸 → 物理尺寸（两者互为转置）。
    pub fn phys_size(&self, lw: u32, lh: u32) -> (u32, u32) {
        (lh, lw)
    }

    /// 逻辑矩形 → 物理矩形。
    pub fn map_rect(&self, x: i32, y: i32, w: i32, h: i32, lw: i32, lh: i32) -> Rect {
        match self {
            Rotation::Rot90 => Rect { x: y, y: lw - x - w, w: h, h: w },
            Rotation::Rot270 => Rect { x: lh - y - h, y: x, w: h, h: w },
        }
    }

    /// 逻辑像素 → 物理像素（左上角像素坐标）。
    pub fn map_point(&self, x: i32, y: i32, lw: i32, lh: i32) -> (i32, i32) {
        match self {
            Rotation::Rot90 => (y, lw - 1 - x),
            Rotation::Rot270 => (lh - 1 - y, x),
        }
    }
}

/// 已就绪的物理屏仪表。
pub struct Screen {
    pub display: Display,
    pub canvas: Canvas,
    pub rot: Rotation,
}

/// 初始化 DRM 输出 + 字体 + 逻辑画布。失败时调用方应回退到 kmscon/ASCII。
pub fn open(rot: Rotation) -> Result<Screen, String> {
    // 启动即点亮面板并把状态同步为「亮」：
    // msm 把背光 bl_power=4 当 DPMS 用，关掉后写回 0 不会恢复输出，
    // 而 bl_power 只反映「上次写入」，所以每次启动都显式拉一次，避免继承上次的灭屏状态。
    let _ = crate::collector::hardware::set_screen_power(true);
    crate::collector::power_key::update_screen_state(true);

    // 注意：局部变量不能叫 display —— tracing 的宏内部有 field::display 函数，会冲突
    let disp = Display::open()?;
    // 逻辑宽 = 物理高，逻辑高 = 物理宽
    let canvas = Canvas::new(disp.ph as i32, disp.pw as i32, rot)?;
    tracing::info!(
        "screen: 显示 {} {} → 逻辑画布 {}x{} ({:?})",
        disp.connector,
        disp.mode,
        canvas.lw,
        canvas.lh,
        rot
    );
    Ok(Screen { display: disp, canvas, rot })
}

/// 渲染主循环（阻塞线程内运行）。
///
/// - 数据变化（采集周期 5s）→ 整屏重绘
/// - 其余每秒 → 只重绘顶栏时钟，其余像素不动（差分刷新，避免闪烁）
pub fn run(mut screen: Screen, mut rx: watch::Receiver<SystemOverview>, db: Option<Arc<Database>>) -> Result<(), String> {
    let mut hist = layout::History::new();
    let mut aux = layout::Aux::new();
    let tokens = token::start_feed();
    let mut last_ts: i64 = -1;
    let mut last_page: u8 = 255;
    let mut frames: u64 = 0;
    let mut force_full = true;
    let mut panel_was_on = true;

    loop {
        std::thread::sleep(Duration::from_millis(1000));
        let o = rx.borrow_and_update().clone();

        // ── 页面切换（音量键）──
        let page = crate::collector::hotkeys::page();
        if page != last_page {
            last_page = page;
            force_full = true;
            // 切到 Token 页时若快照偏旧就立刻刷一次 REST
            if let Ok(mut f) = tokens.lock() {
                if f.http_at.map(|t| t.elapsed().as_secs() >= 5).unwrap_or(true) {
                    f.refresh_http();
                }
            }
            tracing::info!("screen: 切到页面 {}/{}", page + 1, crate::collector::hotkeys::PAGE_COUNT);
        }

        // ── 熄屏（电源键 bl_power=4）时暂停渲染 ──
        // 面板看不见时仍每秒拷贝 ~10MB 并 flip 是纯浪费（实测屏灭时仍占 ~4% CPU），
        // 这里只保留 1Hz 状态轮询；点亮后强制整屏重绘，避免显示熄屏期间的旧帧。
        let panel_on = std::fs::read_to_string("/sys/class/backlight/ae94000.dsi.0/bl_power")
            .map(|s| s.trim() != "4")
            .unwrap_or(true);
        if !panel_on {
            if panel_was_on {
                panel_was_on = false;
                tracing::info!("screen: 面板已熄屏 → 暂停渲染与提交（数据采集/WS 继续）");
            }
            continue;
        }
        if !panel_was_on {
            panel_was_on = true;
            force_full = true;
            tracing::info!("screen: 面板已点亮 → 恢复渲染并整屏重绘");
        }

        // ── 自愈：CRTC 上的 framebuffer 不是我们的就重新提交模式 ──
        //
        // 触发场景：电源键灭屏（msm 把 bl_power=4 当 DPMS，关掉输出后写回 0 不恢复）、
        // fbcon/fbdev 抢回控制台、驱动 reset。开机首帧也走这条路。
        // 用面板真实状态判断是否该保持画面（bl_power=4 表示用户主动灭屏）
        // 不用 power_key 的内存标志：那个可能因为一次误触发而长期为 false，
        // 会导致 fbcon 抢屏后我们再也不敢自愈。
        if screen.display.crtc_framebuffer() != Some(screen.display.framebuffer()) {
            match screen.display.reassert() {
                Ok(()) => {
                    tracing::warn!("screen: CRTC 被接管/熄灭，已重新提交模式");
                    force_full = true;
                }
                Err(e) => tracing::error!("screen: {e}"),
            }
        }

        if o.timestamp != last_ts || force_full {
            last_ts = o.timestamp;
            let t0 = std::time::Instant::now();
            aux.refresh(&o, db.as_deref());
            hist.push(&o, &aux);
            if force_full {
                screen.canvas.invalidate_all();
                force_full = false;
            }
            match page {
                1 => {
                    if let Ok(f) = tokens.lock() {
                        token::render(&mut screen.canvas, &o, &f);
                    }
                }
                _ => layout::render(&mut screen.canvas, &o, &aux, &hist),
            }
            if frames % 60 == 0 {
                tracing::debug!("screen: 整屏重绘 {:.1}ms", t0.elapsed().as_secs_f64() * 1000.0);
            }
        } else {
            match page {
                1 => token::render_clock(&mut screen.canvas, &o),
                _ => layout::render_clock(&mut screen.canvas, &o),
            }
        }

        // 双缓冲 + page flip：每帧整帧拷贝到绘制 slot，再 flip 上屏
        let pitch = screen.display.pitch;
        {
            let Screen { display, canvas, .. } = &mut screen;
            canvas.flush_full(display.pixels_mut(), pitch);
        }
        if let Err(e) = screen.display.commit() {
            if frames % 30 == 0 {
                tracing::warn!("screen: 提交未成功: {e}");
            }
        }
        frames += 1;
    }
}

/// 离屏渲染一帧并导出 PPM，用于预览与验证（**不需要 DRM**，可在服务运行时执行）。
///
/// 导出两份：
/// - `<path>`：逻辑方向（横屏所见，2340x1080）
/// - `<path>.raw.ppm`：物理方向（面板实际扫描输出，1080x2340）
///
/// 两份互为 90° 旋转，可用来核对旋转映射是否正确。
pub fn dump(o: &SystemOverview, rot: Rotation, path: &str, page: u8) -> Result<(), String> {
    let mut canvas = Canvas::new(2340, 1080, rot)?;
    if page == 1 {
        let feed = token::start_feed();
        // 等首轮 REST 快照 + WS 首帧吞吐（最多 8s），让预览接近真机所见
        for _ in 0..80 {
            std::thread::sleep(Duration::from_millis(100));
            if let Ok(f) = feed.lock() {
                if f.http_at.is_some() && f.live.ws_connected && f.live.tps.len() >= 4 {
                    break;
                }
            }
        }
        let f = feed.lock().map_err(|e| e.to_string())?;
        token::render(&mut canvas, o, &f);
        drop(f);
    } else {
        let mut aux = layout::Aux::new();
        aux.refresh(o, None);
        let mut hist = layout::History::new();
        hist.seed_wave(
            o.cpu.overall_usage.max(8.0),
            o.memory.usage_percent as f32,
            120_000.0,
        );
        hist.push(o, &aux);
        layout::render(&mut canvas, o, &aux, &hist);
    }

    write_ppm_logical(&canvas, path)?;
    write_ppm_physical(&canvas, &format!("{path}.raw.ppm"))?;
    Ok(())
}

fn ppm_header(w: i32, h: i32) -> Vec<u8> {
    format!("P6\n{} {}\n255\n", w, h).into_bytes()
}

fn push_pixel(out: &mut Vec<u8>, v: u32) {
    out.push(((v >> 16) & 0xFF) as u8);
    out.push(((v >> 8) & 0xFF) as u8);
    out.push((v & 0xFF) as u8);
}

/// 逻辑方向导出（把物理缓冲按旋转逆映射回逻辑坐标）。
fn write_ppm_logical(c: &Canvas, path: &str) -> Result<(), String> {
    use std::io::Write;
    let (lw, lh, pw) = (c.lw, c.lh, c.pw);
    let mut out = ppm_header(lw, lh);
    out.reserve((lw * lh * 3) as usize);
    for ly in 0..lh {
        for lx in 0..lw {
            let (px, py) = c.rot.map_point(lx, ly, lw, lh);
            let v = if px >= 0 && px < pw && py >= 0 && py < c.ph {
                c.buf[(py * pw + px) as usize]
            } else {
                0
            };
            push_pixel(&mut out, v);
        }
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&out))
        .map_err(|e| format!("写出 {path} 失败: {e}"))
}

/// 物理方向导出（原样，即面板扫描内容）。
fn write_ppm_physical(c: &Canvas, path: &str) -> Result<(), String> {
    use std::io::Write;
    let mut out = ppm_header(c.pw, c.ph);
    out.reserve((c.pw * c.ph * 3) as usize);
    for v in &c.buf {
        push_pixel(&mut out, *v);
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&out))
        .map_err(|e| format!("写出 {path} 失败: {e}"))
}
