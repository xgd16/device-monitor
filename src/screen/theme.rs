//! 语义色板与字号级别。
//!
//! 遵循 TUI Design System 的语义槽约定：绘制代码只引用 `Palette::*` / `Type::*`，
//! 不出现硬编码十六进制，配色可整体替换。
//!
//! 颜色为 `0xAARRGGBB`，直接按 DRM `Argb8888`（小端内存序 B,G,R,A）写入帧缓冲。

/// 0xAARRGGBB 像素值。
pub type Argb = u32;

/// 编译期构造颜色值。
pub const fn rgb(r: u32, g: u32, b: u32) -> Argb {
    0xFF00_0000 | ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)
}

/// 语义色板（深色主题，OLED 近黑底省电）。
pub struct Palette;

impl Palette {
    // ── 背景层次：base < surface < surface_alt，靠亮度差造层级，少用边框 ──
    pub const BG_BASE: Argb = rgb(0x07, 0x09, 0x0D);
    pub const BG_SURFACE: Argb = rgb(0x10, 0x15, 0x1D);
    pub const BG_SURFACE_ALT: Argb = rgb(0x1A, 0x22, 0x2E);
    pub const BG_TRACK: Argb = rgb(0x1E, 0x27, 0x34);
    pub const BORDER: Argb = rgb(0x26, 0x31, 0x42);

    // ── 前景 ──
    /// 正文
    pub const FG_DEFAULT: Argb = rgb(0xCB, 0xD5, 0xE5);
    /// 次要信息（元数据、单位、标签）
    pub const FG_MUTED: Argb = rgb(0x7C, 0x8B, 0xA5);
    /// 标题与强调
    pub const FG_EMPHASIS: Argb = rgb(0xFF, 0xFF, 0xFF);

    // ── 强调 / 状态 ──
    pub const ACCENT: Argb = rgb(0x4E, 0xA3, 0xFF);
    pub const ACCENT_2: Argb = rgb(0xBB, 0x9A, 0xF7);
    pub const SUCCESS: Argb = rgb(0x3F, 0xD0, 0x7C);
    pub const WARNING: Argb = rgb(0xFF, 0xB4, 0x28);
    pub const ERROR: Argb = rgb(0xFF, 0x5D, 0x68);
    pub const INFO: Argb = rgb(0x45, 0xD3, 0xD3);
}

/// 使用率 → 语义色（绿/黄/红）。
///
/// 注意：颜色只是辅助，数值文本始终同时展示，不依赖颜色单独表达含义。
pub fn level(pct: f64) -> Argb {
    if pct >= 85.0 {
        Palette::ERROR
    } else if pct >= 65.0 {
        Palette::WARNING
    } else {
        Palette::SUCCESS
    }
}

/// 温度 → 语义色。
pub fn temp_level(c: f64) -> Argb {
    if c >= 75.0 {
        Palette::ERROR
    } else if c >= 60.0 {
        Palette::WARNING
    } else {
        Palette::SUCCESS
    }
}

/// 字号级别（像素，针对 2340x1080 逻辑画布 / 6.39" 屏标定）。
pub struct Type;

impl Type {
    /// 设备主标题
    pub const H1: f32 = 38.0;
    /// 时钟
    pub const CLOCK: f32 = 44.0;
    /// 超大数值（CPU/内存/电池百分比）
    pub const VALUE_XL: f32 = 62.0;
    /// 大数值
    pub const VALUE_L: f32 = 44.0;
    /// 中数值
    pub const VALUE_M: f32 = 32.0;
    /// 卡片标题
    pub const TITLE: f32 = 23.0;
    /// 正文
    pub const BODY: f32 = 25.0;
    /// 标签/元数据
    pub const LABEL: f32 = 21.0;
    /// 极小（表格表头）
    pub const TINY: f32 = 19.0;
}
