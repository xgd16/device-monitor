//! CJK 字体光栅化 + 90°/270° **预旋转字形缓存**。
//!
//! 因为内核只提供 plane `rotate-0/180`（无 90/270），横屏必须软件旋转。
//! 若每帧对整屏做转置，10MB 的缓存不友好访问在手机上代价过高；
//! 这里改为在字形光栅化阶段一次性把覆盖位图旋转好并缓存，
//! 绘制时按物理行连续写内存，开销与普通正屏绘制相同。
//!
//! 旋转映射（见 `Rotation::map_point`）：
//! - `Rot90`：字形逻辑左边界 → 物理 y 递减方向；逻辑上边界 → 物理 x 递增方向。

use std::collections::HashMap;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};

use super::Rotation;

/// 字重。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Weight {
    Regular,
    Bold,
}

/// 字体候选：(路径, TTC face 索引)。Noto Sans CJK 的简体中文面是 index 2。
const FONT_REGULAR: &[(&str, u32)] = &[
    ("/usr/share/fonts/noto/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/noto/NotoSansCJK-Regular.ttc", 0),
    ("/usr/share/fonts/TTF/DejaVuSans.ttf", 0),
];

const FONT_BOLD: &[(&str, u32)] = &[
    ("/usr/share/fonts/noto/NotoSansCJK-Bold.ttc", 2),
    ("/usr/share/fonts/noto/NotoSansCJK-Bold.ttc", 0),
    ("/usr/share/fonts/TTF/DejaVuSans-Bold.ttf", 0),
];

/// 一个已按目标旋转方向排布好的字形位图。
#[derive(Clone)]
pub struct GlyphTile {
    /// 物理 x 方向宽度（= 字形逻辑高度）
    pub tw: u32,
    /// 物理 y 方向高度（= 字形逻辑宽度）
    pub th: u32,
    /// 字形逻辑宽度
    pub lw: u32,
    /// 字形逻辑高度
    pub lh: u32,
    /// `tw * th` 覆盖率（0-255），已旋转
    pub cov: Vec<u8>,
    /// 相对笔位置的左边界（逻辑坐标）
    pub left: i32,
    /// 相对基线的上边界（逻辑坐标，通常为负）
    pub top: i32,
    /// 水平步进
    pub adv: f32,
}

/// 字体集合 + 字形缓存。
pub struct FontSet {
    regular: FontVec,
    bold: FontVec,
    rot: Rotation,
    cache: HashMap<(u32, u32, u8), GlyphTile>,
}

fn load_first(cands: &[(&str, u32)]) -> Option<FontVec> {
    for (path, idx) in cands {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(f) = FontVec::try_from_vec_and_index(data, *idx) {
                return Some(f);
            }
        }
    }
    None
}

impl FontSet {
    /// 加载字体。Bold 缺失时回退 Regular。
    pub fn load(rot: Rotation) -> Result<Self, String> {
        let regular = load_first(FONT_REGULAR)
            .ok_or_else(|| "找不到可用字体（尝试 Noto Sans CJK / DejaVuSans）".to_string())?;
        let bold = load_first(FONT_BOLD).unwrap_or_else(|| {
            load_first(FONT_REGULAR).expect("regular font loaded above")
        });
        Ok(Self { regular, bold, rot, cache: HashMap::new() })
    }

    fn font(&self, weight: Weight) -> &FontVec {
        match weight {
            Weight::Bold => &self.bold,
            Weight::Regular => &self.regular,
        }
    }

    /// (ascent, descent, line_gap)
    pub fn metrics(&self, size: f32, weight: Weight) -> (f32, f32, f32) {
        let sf = self.font(weight).as_scaled(PxScale::from(size));
        (sf.ascent(), sf.descent(), sf.line_gap())
    }

    /// 行高（像素）
    pub fn line_height(&self, size: f32) -> i32 {
        let (a, d, g) = self.metrics(size, Weight::Regular);
        (a - d + g).ceil().max(1.0) as i32
    }

    /// 单字符步进
    pub fn advance(&self, ch: char, size: f32, weight: Weight) -> f32 {
        let sf = self.font(weight).as_scaled(PxScale::from(size));
        sf.h_advance(sf.glyph_id(ch))
    }

    /// 文本宽度（支持 `\n`，取最长行）
    pub fn text_width(&self, s: &str, size: f32, weight: Weight) -> i32 {
        let sf = self.font(weight).as_scaled(PxScale::from(size));
        let mut cur = 0.0f32;
        let mut max = 0.0f32;
        for ch in s.chars() {
            if ch == '\n' {
                max = max.max(cur);
                cur = 0.0;
            } else {
                cur += sf.h_advance(sf.glyph_id(ch));
            }
        }
        max.max(cur).round() as i32
    }

    /// 取字形位图（首次光栅化并缓存）。
    pub fn tile(&mut self, ch: char, size: f32, weight: Weight) -> &GlyphTile {
        let key = (
            ch as u32,
            (size * 4.0).round() as u32,
            if weight == Weight::Bold { 1 } else { 0 },
        );
        if !self.cache.contains_key(&key) {
            let t = self.rasterize(ch, size, weight);
            self.cache.insert(key, t);
        }
        self.cache.get(&key).expect("just inserted")
    }

    fn rasterize(&self, ch: char, size: f32, weight: Weight) -> GlyphTile {
        let font = self.font(weight);
        let scale = PxScale::from(size);
        let sf = font.as_scaled(scale);
        let gid = sf.glyph_id(ch);
        let adv = sf.h_advance(gid);

        // 字体缺该字形时 glyph_id 为 0（.notdef）。若照画就是一个空方块（tofu），
        // 代理节点名里常带的 ✅/⭐ 之类 BMP 符号就会中招，这里直接不画。
        if gid.0 == 0 && ch != ' ' && ch != '\t' {
            return GlyphTile { tw: 0, th: 0, lw: 0, lh: 0, cov: Vec::new(), left: 0, top: 0, adv };
        }

        let mut lw = 0u32;
        let mut lh = 0u32;
        let mut cov_flat: Vec<u8> = Vec::new();
        let mut left = 0i32;
        let mut top = 0i32;

        if ch != ' ' && ch != '\t' {
            let glyph = gid.with_scale_and_position(scale, point(0.0, 0.0));
            if let Some(og) = font.outline_glyph(glyph) {
                let b = og.px_bounds();
                lw = b.width().ceil().max(0.0) as u32;
                lh = b.height().ceil().max(0.0) as u32;
                left = b.min.x.floor() as i32;
                top = b.min.y.floor() as i32;
                if lw > 0 && lh > 0 {
                    let mut c = vec![0u8; (lw * lh) as usize];
                    og.draw(|gx, gy, v| {
                        let i = (gy * lw + gx) as usize;
                        if i < c.len() {
                            c[i] = (v * 255.0).clamp(0.0, 255.0).round() as u8;
                        }
                    });
                    cov_flat = c;
                }
            }
        }

        let (tw, th, cov) = if lw == 0 || lh == 0 {
            (0, 0, Vec::new())
        } else {
            rotate_cov(&cov_flat, lw, lh, self.rot)
        };

        GlyphTile { tw, th, lw, lh, cov, left, top, adv }
    }
}

/// 把逻辑方向的覆盖率位图旋转成物理方向。
///
/// - `Rot90`：`tile[ty][tx] = cov[gy = tx][gx = lw-1-ty]`
/// - `Rot270`：`tile[ty][tx] = cov[gy = lh-1-tx][gx = ty]`
///
/// 返回 `(tw, th)` = `(lh, lw)`。
fn rotate_cov(src: &[u8], lw: u32, lh: u32, rot: Rotation) -> (u32, u32, Vec<u8>) {
    let tw = lh;
    let th = lw;
    let mut out = vec![0u8; (tw * th) as usize];
    for ty in 0..th {
        for tx in 0..tw {
            let i = match rot {
                Rotation::Rot90 => (tx * lw + (lw - 1 - ty)) as usize,
                Rotation::Rot270 => ((lh - 1 - tx) * lw + ty) as usize,
            };
            out[(ty * tw + tx) as usize] = src.get(i).copied().unwrap_or(0);
        }
    }
    (tw, th, out)
}

