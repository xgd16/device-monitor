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
pub mod clock;
pub mod display;
pub mod font;
pub mod layout;
pub mod theme;
pub mod token;
pub mod weather;

use std::sync::atomic::{AtomicU64, Ordering};
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
/// - 数据变化（采集周期即刷新间隔，Web 端可调 1/3/5/10s）→ 整屏重绘
/// - 其余每秒 → 只重绘顶栏时钟，其余像素不动（差分刷新，避免闪烁）
pub fn run(
    mut screen: Screen,
    mut rx: watch::Receiver<SystemOverview>,
    db: Option<Arc<Database>>,
    refresh_secs: Arc<AtomicU64>,
) -> Result<(), String> {
    let mut hist = layout::History::new();
    let mut aux = layout::Aux::new();
    let tokens = token::start_feed();
    // 天气每 15 分钟刷新一次，放独立线程，别阻塞 1Hz 渲染循环
    weather::start_feed();
    let mut last_ts: i64 = -1;
    let mut last_page: u8 = 255;
    let mut frames: u64 = 0;
    let mut force_full = true;
    let mut panel_was_on = true;
    // 朝向状态以 hotkeys 为准（双击音量上翻转），启动时 main 已把它初始化成
    // 与屏上实际朝向一致，所以这里不会一进来就误判成「需要切换」
    let mut cur_rot270 = crate::collector::hotkeys::rot270();
    // 主题同理（双击音量下切换），初始与落盘值一致
    let mut cur_light = crate::collector::hotkeys::light();

    loop {
        // 等 1 秒，或被「按键翻了页/转了朝向」立刻唤醒。
        //
        // 以前是 `sleep(1000)` 然后才采样页面状态：按键落到两次采样之间就白等，
        // 平均 500ms、最多 1000ms 才重绘。实测翻页延迟 673~1584ms，这里是主因之一。
        crate::collector::hotkeys::wait_change(1000);
        let o = rx.borrow_and_update().clone();

        // ── 页面切换（音量键）──
        let page = crate::collector::hotkeys::page();
        if page != last_page {
            last_page = page;
            force_full = true;
            // Token 页数据偏旧时**只置个标志**让后台 feed 线程去刷 REST：
            // refresh_http() 是 ureq 同步调用、单请求超时 6s、一次连发好几个，
            // 就地刷会把「按键 → 翻页」这条路一起堵住。
            if page == 1 {
                token::request_refresh();
            }
            tracing::info!("screen: 切到页面 {}/{}", page + 1, crate::collector::hotkeys::PAGE_COUNT);
        }

        // ── 屏幕朝向切换（双击音量上）──
        //
        // 朝向变了必须**重建画布**：字形是按旋转方向预先栅格化缓存的
        // （`FontSet::load(rot)`），只改 `rot` 字段会让整屏字都歪着。
        let want270 = crate::collector::hotkeys::rot270();
        if want270 != cur_rot270 {
            cur_rot270 = want270;
            let rot = if want270 { Rotation::Rot270 } else { Rotation::Rot90 };
            match Canvas::new(screen.canvas.lw, screen.canvas.lh, rot) {
                Ok(c) => {
                    screen.canvas = c;
                    screen.rot = rot;
                    force_full = true;
                    tracing::info!("screen: 朝向切换为 {rot:?}，画布已重建");
                }
                Err(e) => tracing::error!("screen: 切换朝向失败，保持原朝向: {e}"),
            }
        }

        // ── 深浅主题切换（双击音量下）──
        //
        // 颜色是画布底层按当前主题实时解析的（`theme::resolve`），不像朝向那样
        // 需要重建画布；把 force_full 置上让整屏按新配色重画一次即可。
        let want_light = crate::collector::hotkeys::light();
        if want_light != cur_light {
            cur_light = want_light;
            force_full = true;
            tracing::info!("screen: 主题切换为{}", if want_light { "浅色" } else { "深色" });
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

        // 「数据该刷新了」和「画布该整屏重绘了」是两件事，必须分开：
        //
        // 以前只要 `force_full`（切页/转朝向/熄屏恢复/自愈）就顺带跑一次
        // `aux.refresh()`，而它里面有 list_processes()（枚举 239 个进程）、
        // 4 次 `systemctl is-active`、一次 DB 查询、磁盘与网络差分 ——
        // 实测这一下约 580ms，全都压在按键响应上。切页并不需要重新采集这些，
        // 复用上一份即可（数据本来每 refresh_secs 就更一次）。
        let data_dirty = o.timestamp != last_ts;
        if data_dirty {
            last_ts = o.timestamp;
            aux.refresh(&o, db.as_deref());
            hist.push(&o, &aux, layout::hist_cap(refresh_secs.load(Ordering::Relaxed)));
        }

        if data_dirty || force_full {
            let t0 = std::time::Instant::now();
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
                2 => clock::render(&mut screen.canvas, &o),
                _ => layout::render(&mut screen.canvas, &o, &aux, &hist),
            }
            if frames % 60 == 0 {
                tracing::debug!("screen: 整屏重绘 {:.1}ms", t0.elapsed().as_secs_f64() * 1000.0);
            }
        } else {
            match page {
                1 => token::render_clock(&mut screen.canvas, &o),
                2 => clock::render_clock(&mut screen.canvas, &o),
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
    // 预览要对齐真机所见：页脚会读 hotkeys::page() 显示「当前第 N 页」，
    // 新进程里它是 0，不同步就会导出「第 1/3 页」而真机在第 3 页。
    crate::collector::hotkeys::set_page(page);
    // 朝向状态也要同步：页脚会按朝向显示按键方向，不同步就会导出「指示写反」的图
    crate::collector::hotkeys::set_rot270(matches!(rot, Rotation::Rot270));
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
    } else if page == 2 {
        // 预览要贴近真机所见：等首轮天气快照（最多 8s），否则导出的只有「正在获取天气…」
        let wf = weather::start_feed();
        for _ in 0..80 {
            std::thread::sleep(Duration::from_millis(100));
            if wf.lock().map(|w| w.updated.is_some()).unwrap_or(false) {
                break;
            }
        }
        clock::render(&mut canvas, o);
    } else {
        let mut aux = layout::Aux::new();
        aux.refresh(o, None);
        let mut hist = layout::History::new();
        hist.seed_wave(
            o.cpu.overall_usage.max(8.0),
            o.memory.usage_percent as f32,
            120_000.0,
        );
        // 预览按 5s 基准采样填充，与 seed_wave 的波形密度一致
        hist.push(o, &aux, layout::hist_cap(5));
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
