//! 语义色板与字号级别。
//!
//! 遵循 TUI Design System 的语义槽约定：绘制代码只引用 `Palette::*` / `Type::*`，
//! 不出现硬编码十六进制，配色可整体替换。
//!
//! 颜色为 `0xAARRGGBB`，直接按 DRM `Argb8888`（小端内存序 B,G,R,A）写入帧缓冲。
//!
//! 主题切换：`Palette` 常量即**规范色值**（= 深色主题色值），全代码库 300+ 处引用
//! 它、不需要感知主题；落屏前由画布底层（`canvas.rs` 的 clear/rect/row/字形混合
//! 四处）统一经 [`resolve`] 换算成当前主题的实际像素色。浅色主题见 [`LightPalette`]，
//! 由双击音量下切换（状态在 `hotkeys`，落盘 `theme.txt`）。

/// 0xAARRGGBB 像素值。
pub type Argb = u32;

/// 编译期构造颜色值。
pub const fn rgb(r: u32, g: u32, b: u32) -> Argb {
    0xFF00_0000 | ((r & 0xFF) << 16) | ((g & 0xFF) << 8) | (b & 0xFF)
}

/// 语义色板（规范色值 = 深色主题，OLED 近黑底省电）。
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

/// 浅色主题色板：槽位与 [`Palette`] 一一对应。
///
/// 层次设计：白底卡片 + 浅灰画布（浅色主题里「抬升 = 更白」，与深色相反）；
/// 语义色整体加深，保证浅色底上可读——对比度由单测把关（见 `tests`），
/// 以后手抖把某个色调浅到看不清，测试会先炸而不是等肉眼发现。
pub struct LightPalette;

impl LightPalette {
    // ── 背景层次：base（画布）< surface（卡片白）──
    pub const BG_BASE: Argb = rgb(0xF2, 0xF4, 0xF7);
    pub const BG_SURFACE: Argb = rgb(0xFF, 0xFF, 0xFF);
    pub const BG_SURFACE_ALT: Argb = rgb(0xE8, 0xEC, 0xF2);
    pub const BG_TRACK: Argb = rgb(0xDD, 0xE3, 0xEC);
    pub const BORDER: Argb = rgb(0xD3, 0xD9, 0xE3);

    // ── 前景 ──
    /// 正文
    pub const FG_DEFAULT: Argb = rgb(0x33, 0x3D, 0x4D);
    /// 次要信息（元数据、单位、标签）
    pub const FG_MUTED: Argb = rgb(0x5F, 0x6E, 0x83);
    /// 标题与强调
    pub const FG_EMPHASIS: Argb = rgb(0x0E, 0x14, 0x1E);

    // ── 强调 / 状态（白底可读的加深版）──
    pub const ACCENT: Argb = rgb(0x25, 0x63, 0xEB);
    pub const ACCENT_2: Argb = rgb(0x7C, 0x3A, 0xED);
    pub const SUCCESS: Argb = rgb(0x15, 0x80, 0x3D);
    pub const WARNING: Argb = rgb(0xB4, 0x53, 0x09);
    pub const ERROR: Argb = rgb(0xC6, 0x28, 0x28);
    pub const INFO: Argb = rgb(0x0E, 0x74, 0x90);
}

/// 按当前主题把规范色值解析为实际像素色。
///
/// - 深色主题（默认）：恒等映射（`Palette` 常量本身就是最终色）；
/// - 浅色主题：按槽位映射到 [`LightPalette`] 的对应色；
/// - 非调色板颜色：原样透传（当前代码库不应出现，属防御性兜底）。
///
/// 之所以做「映射」而不是让绘制代码调函数取色：绘制代码有 300+ 处引用
/// `Palette::X`，集中在这里换算能让它们零改动，也让新增绘制自动跟随主题。
pub fn resolve(c: Argb) -> Argb {
    if !crate::collector::hotkeys::light() {
        return c;
    }
    match c {
        Palette::BG_BASE => LightPalette::BG_BASE,
        Palette::BG_SURFACE => LightPalette::BG_SURFACE,
        Palette::BG_SURFACE_ALT => LightPalette::BG_SURFACE_ALT,
        Palette::BG_TRACK => LightPalette::BG_TRACK,
        Palette::BORDER => LightPalette::BORDER,
        Palette::FG_DEFAULT => LightPalette::FG_DEFAULT,
        Palette::FG_MUTED => LightPalette::FG_MUTED,
        Palette::FG_EMPHASIS => LightPalette::FG_EMPHASIS,
        Palette::ACCENT => LightPalette::ACCENT,
        Palette::ACCENT_2 => LightPalette::ACCENT_2,
        Palette::SUCCESS => LightPalette::SUCCESS,
        Palette::WARNING => LightPalette::WARNING,
        Palette::ERROR => LightPalette::ERROR,
        Palette::INFO => LightPalette::INFO,
        other => other,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::hotkeys;

    /// 主题状态是全局的、并行测试会互相抢，所以浅/深两态的断言集中在一个
    /// 测试函数里顺序跑（拆开反而会互相打架而变得偶发失败）。
    #[test]
    fn 浅深主题解析与页脚提示联动() {
        hotkeys::set_light(false);
        // 深色：恒等映射
        assert_eq!(resolve(Palette::BG_BASE), Palette::BG_BASE);
        assert_eq!(resolve(Palette::FG_MUTED), Palette::FG_MUTED);
        assert_eq!(resolve(Palette::ERROR), Palette::ERROR);
        assert_eq!(hotkeys::theme_hint(), "浅色", "深色时应提示双击切到浅色");

        // 浅色：逐槽位映射
        hotkeys::set_light(true);
        assert_eq!(resolve(Palette::BG_BASE), LightPalette::BG_BASE);
        assert_eq!(resolve(Palette::BG_SURFACE), LightPalette::BG_SURFACE);
        assert_eq!(resolve(Palette::BG_SURFACE_ALT), LightPalette::BG_SURFACE_ALT);
        assert_eq!(resolve(Palette::BG_TRACK), LightPalette::BG_TRACK);
        assert_eq!(resolve(Palette::BORDER), LightPalette::BORDER);
        assert_eq!(resolve(Palette::FG_DEFAULT), LightPalette::FG_DEFAULT);
        assert_eq!(resolve(Palette::FG_MUTED), LightPalette::FG_MUTED);
        assert_eq!(resolve(Palette::FG_EMPHASIS), LightPalette::FG_EMPHASIS);
        assert_eq!(resolve(Palette::ACCENT), LightPalette::ACCENT);
        assert_eq!(resolve(Palette::ACCENT_2), LightPalette::ACCENT_2);
        assert_eq!(resolve(Palette::SUCCESS), LightPalette::SUCCESS);
        assert_eq!(resolve(Palette::WARNING), LightPalette::WARNING);
        assert_eq!(resolve(Palette::ERROR), LightPalette::ERROR);
        assert_eq!(resolve(Palette::INFO), LightPalette::INFO);
        assert_eq!(hotkeys::theme_hint(), "深色", "浅色时应提示双击切到深色");

        // 非调色板颜色原样透传
        assert_eq!(resolve(0xFF12_3456), 0xFF12_3456);

        hotkeys::set_light(false);
    }

    /// 浅色语义色对最不利的底（浅灰画布，比白卡更暗）的对比度下限。
    /// 大数值 ≥3:1、正文/标签 ≥4.5:1（WCAG），调色时先过这关再上屏。
    #[test]
    fn 浅色语义色对比度达标() {
        fn lum(c: Argb) -> f64 {
            fn ch(v: u32) -> f64 {
                let s = v as f64 / 255.0;
                if s <= 0.03928 {
                    s / 12.92
                } else {
                    ((s + 0.055) / 1.055).powf(2.4)
                }
            }
            0.2126 * ch((c >> 16) & 0xFF) + 0.7152 * ch((c >> 8) & 0xFF) + 0.0722 * ch(c & 0xFF)
        }
        fn contrast(a: Argb, b: Argb) -> f64 {
            let (hi, lo) = if lum(a) > lum(b) { (lum(a), lum(b)) } else { (lum(b), lum(a)) };
            (hi + 0.05) / (lo + 0.05)
        }
        let bg = LightPalette::BG_BASE;
        for (name, c, min) in [
            ("FG_DEFAULT", LightPalette::FG_DEFAULT, 4.5),
            ("FG_MUTED", LightPalette::FG_MUTED, 4.5),
            ("FG_EMPHASIS", LightPalette::FG_EMPHASIS, 7.0),
            ("ACCENT", LightPalette::ACCENT, 4.5),
            ("ACCENT_2", LightPalette::ACCENT_2, 4.5),
            ("SUCCESS", LightPalette::SUCCESS, 3.0),
            ("WARNING", LightPalette::WARNING, 4.5),
            ("ERROR", LightPalette::ERROR, 4.5),
            ("INFO", LightPalette::INFO, 4.5),
        ] {
            let r = contrast(c, bg);
            assert!(r >= min, "{name} 对浅底的对比度 {r:.2} 低于 {min}");
        }
    }
}
