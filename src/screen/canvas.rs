//! 横屏逻辑画布：所有绘制 API 使用**逻辑坐标**（左上原点，x 右 y 下），
//! 内部通过 `Rotation` 映射到物理帧缓冲；脏矩形记录用于差分刷新。

use bytemuck::cast_slice;

use super::Rotation;
use super::font::{FontSet, GlyphTile, Weight};
use super::theme::{self, Argb, Palette, Type};

/// 物理坐标矩形（脏矩形单位）。
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 逻辑画布 + 物理合成缓冲。
pub struct Canvas {
    /// 逻辑宽（= 物理高）
    pub lw: i32,
    /// 逻辑高（= 物理宽）
    pub lh: i32,
    /// 物理宽
    pub pw: i32,
    /// 物理高
    pub ph: i32,
    pub rot: Rotation,
    /// 物理像素缓冲（stride = pw）
    pub buf: Vec<u32>,
    /// 待刷新到扫描输出的物理矩形
    pub dirty: Vec<Rect>,
    pub fonts: FontSet,
}

impl Canvas {
    pub fn new(lw: i32, lh: i32, rot: Rotation) -> Result<Self, String> {
        let (pw, ph) = rot.phys_size(lw as u32, lh as u32);
        let fonts = FontSet::load(rot)?;
        Ok(Self {
            lw,
            lh,
            pw: pw as i32,
            ph: ph as i32,
            rot,
            buf: vec![0; (pw as usize) * (ph as usize)],
            dirty: Vec::new(),
            fonts,
        })
    }

    fn phys(&self, x: i32, y: i32, w: i32, h: i32) -> Rect {
        self.rot.map_rect(x, y, w, h, self.lw, self.lh)
    }

    /// 全屏填充。
    pub fn clear(&mut self, color: Argb) {
        self.buf.fill(theme::resolve(color));
        self.dirty.push(Rect { x: 0, y: 0, w: self.pw, h: self.ph });
    }

    /// 逻辑矩形填充（物理行连续写入，最快路径）。
    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Argb) {
        if w <= 0 || h <= 0 {
            return;
        }
        let color = theme::resolve(color);
        let r = self.phys(x, y, w, h);
        let x0 = r.x.max(0);
        let y0 = r.y.max(0);
        let x1 = (r.x + r.w).min(self.pw);
        let y1 = (r.y + r.h).min(self.ph);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let pw = self.pw as usize;
        for row in y0..y1 {
            let off = row as usize * pw + x0 as usize;
            self.buf[off..off + (x1 - x0) as usize].fill(color);
        }
        self.dirty.push(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 });
    }

    /// 逻辑单行像素（仅用于圆角弧、折线等小范围绘制）。
    fn row(&mut self, x0: i32, x1: i32, y: i32, color: Argb) {
        if y < 0 || y >= self.lh {
            return;
        }
        let color = theme::resolve(color);
        let a = x0.max(0);
        let b = x1.min(self.lw);
        if b <= a {
            return;
        }
        for x in a..b {
            let (px, py) = self.rot.map_point(x, y, self.lw, self.lh);
            if px >= 0 && px < self.pw && py >= 0 && py < self.ph {
                let i = (py * self.pw + px) as usize;
                self.buf[i] = color;
            }
        }
        // 单行映射到物理列，脏矩形取包围盒
        let mut dx0 = i32::MAX;
        let mut dx1 = i32::MIN;
        let mut dy0 = i32::MAX;
        let mut dy1 = i32::MIN;
        for x in [a, b - 1] {
            let (px, py) = self.rot.map_point(x, y, self.lw, self.lh);
            dx0 = dx0.min(px);
            dx1 = dx1.max(px + 1);
            dy0 = dy0.min(py);
            dy1 = dy1.max(py + 1);
        }
        if dx1 > dx0 && dy1 > dy0 {
            self.dirty.push(Rect { x: dx0, y: dy0, w: dx1 - dx0, h: dy1 - dy0 });
        }
    }

    /// 圆角矩形填充。
    pub fn round_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, color: Argb) {
        if w <= 0 || h <= 0 {
            return;
        }
        let r = r.min(w / 2).min(h / 2).max(0);
        if r == 0 {
            self.rect(x, y, w, h, color);
            return;
        }
        // 中部两大块用连续写入（快）
        self.rect(x + r, y, w - 2 * r, h, color);
        self.rect(x, y + r, w, h - 2 * r, color);
        // 四角弧（每行 ≤ r 像素）
        let rr = (r * r) as f64;
        for dy in 0..r {
            let d = (r - 1 - dy) as f64;
            let cut = (rr - d * d).max(0.0).sqrt().round() as i32;
            let inset = r - cut;
            self.row(x + inset, x + r, y + dy, color);
            self.row(x + w - r, x + w - inset, y + dy, color);
            self.row(x + inset, x + r, y + h - 1 - dy, color);
            self.row(x + w - r, x + w - inset, y + h - 1 - dy, color);
        }
    }

    /// 圆角矩形描边（厚度 t）。
    pub fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, color: Argb) {
        let r = r.min(w / 2).min(h / 2).max(0);
        let t = t.max(1);
        self.rect(x + r, y, w - 2 * r, t, color);
        self.rect(x + r, y + h - t, w - 2 * r, t, color);
        self.rect(x, y + r, t, h - 2 * r, color);
        self.rect(x + w - t, y + r, t, h - 2 * r, color);
        let rr = (r * r) as f64;
        for dy in 0..r {
            let d = (r - 1 - dy) as f64;
            let cut = (rr - d * d).max(0.0).sqrt().round() as i32;
            let inset = r - cut;
            self.row(x + inset, x + inset + t, y + dy, color);
            self.row(x + w - inset - t, x + w - inset, y + dy, color);
            self.row(x + inset, x + inset + t, y + h - 1 - dy, color);
            self.row(x + w - inset - t, x + w - inset, y + h - 1 - dy, color);
        }
    }

    /// 水平细线（逻辑坐标）。
    pub fn hline(&mut self, x: i32, y: i32, w: i32, t: i32, color: Argb) {
        self.rect(x, y, w, t, color);
    }

    /// 画文字，返回结束 x（逻辑坐标）。`baseline` 为基线 y。
    pub fn text(
        &mut self,
        x: i32,
        baseline: i32,
        s: &str,
        size: f32,
        weight: Weight,
        color: Argb,
    ) -> i32 {
        let line = self.fonts.line_height(size);
        let mut pen = x as f32;
        let mut base = baseline;
        let mut max_x = x;
        for ch in s.chars() {
            match ch {
                '\n' => {
                    max_x = max_x.max(pen.round() as i32);
                    pen = x as f32;
                    base += line;
                    continue;
                }
                ' ' | '\t' => {
                    pen += self.fonts.advance(ch, size, weight);
                    continue;
                }
                _ => {}
            }
            let t: &GlyphTile = self.fonts.tile(ch, size, weight);
            if t.tw > 0 && t.th > 0 {
                let gx = pen.round() as i32 + t.left;
                let gy = base + t.top;
                // 旋转后 tile 原点的物理坐标（推导见 font.rs / Rotation::map_point）
                let (px, py) = match self.rot {
                    Rotation::Rot90 => self.rot.map_point(gx + t.lw as i32 - 1, gy, self.lw, self.lh),
                    Rotation::Rot270 => self.rot.map_point(gx, gy + t.lh as i32 - 1, self.lw, self.lh),
                };
                blit_tile(
                    &mut self.buf,
                    self.pw,
                    self.ph,
                    px,
                    py,
                    t,
                    color,
                    &mut self.dirty,
                );
            }
            pen += t.adv;
        }
        max_x.max(pen.round() as i32)
    }

    /// 右对齐文字（`x_right` 为右边界）。
    pub fn text_right(
        &mut self,
        x_right: i32,
        baseline: i32,
        s: &str,
        size: f32,
        weight: Weight,
        color: Argb,
    ) {
        let w = self.fonts.text_width(s, size, weight);
        self.text(x_right - w, baseline, s, size, weight, color);
    }

    /// 居中文字（`cx` 为中心）。
    pub fn text_center(
        &mut self,
        cx: i32,
        baseline: i32,
        s: &str,
        size: f32,
        weight: Weight,
        color: Argb,
    ) {
        let w = self.fonts.text_width(s, size, weight);
        self.text(cx - w / 2, baseline, s, size, weight, color);
    }

    /// 胶囊标签，返回占用宽度。
    pub fn pill(&mut self, x: i32, y: i32, label: &str, size: f32, fg: Argb, bg: Argb) -> i32 {
        let tw = self.fonts.text_width(label, size, Weight::Bold);
        let h = (size * 2.0).round() as i32;
        let w = tw + h;
        self.round_rect(x, y, w, h, h / 2, bg);
        let (asc, _, _) = self.fonts.metrics(size, Weight::Bold);
        let baseline = y + (h + asc.round() as i32) / 2 - 1;
        self.text(x + h / 2, baseline, label, size, Weight::Bold, fg);
        w
    }

    /// 进度条（胶囊形，轨道 + 填充）。
    pub fn bar(&mut self, x: i32, y: i32, w: i32, h: i32, pct: f64, color: Argb) {
        self.round_rect(x, y, w, h, h / 2, Palette::BG_TRACK);
        let fw = ((w as f64) * (pct / 100.0).clamp(0.0, 1.0)).round() as i32;
        if fw > 0 {
            self.round_rect(x, y, fw.max(h), h, h / 2, color);
        }
    }

    /// 实心圆点。
    pub fn dot(&mut self, cx: i32, cy: i32, r: i32, color: Argb) {
        let r2 = (r * r) as f64;
        for dy in -r..=r {
            let d = (r2 - (dy * dy) as f64).max(0.0).sqrt().round() as i32;
            self.row(cx - d, cx + d + 1, cy + dy, color);
        }
    }

    /// 折线（逻辑坐标，粗度 t）。
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, t: i32, color: Argb) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        let half = (t - 1) / 2;
        loop {
            for oy in -half..=half {
                self.row(x, x + t, y + oy, color);
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// 迷你折线图（真实像素折线，非块字符）。
    pub fn sparkline(&mut self, x: i32, y: i32, w: i32, h: i32, vals: &[f32], max: f32, color: Argb) {
        if vals.len() < 2 || w < 4 || h < 4 {
            return;
        }
        let n = vals.len();
        let max = if max <= 0.0 { 1.0 } else { max };
        let mut prev: Option<(i32, i32)> = None;
        for (i, v) in vals.iter().enumerate() {
            let px = x + ((i as f64 / (n - 1) as f64) * (w - 1) as f64).round() as i32;
            let norm = (*v as f64 / max as f64).clamp(0.0, 1.0);
            let py = y + h - 1 - (norm * (h - 1) as f64).round() as i32;
            if let Some((lx, ly)) = prev {
                self.line(lx, ly, px, py, 3, color);
            }
            prev = Some((px, py));
        }
    }

    /// 整帧拷贝到扫描缓冲（双缓冲场景下源必须是完整画布，不能用脏矩形拼）。
    pub fn flush_full(&self, scan: &mut [u8], pitch: usize) {
        let pw = self.pw as usize;
        for row in 0..self.ph as usize {
            let src = &self.buf[row * pw..(row + 1) * pw];
            let bytes = cast_slice::<u32, u8>(src);
            let off = row * pitch;
            let end = (off + bytes.len()).min(scan.len());
            if end > off {
                scan[off..end].copy_from_slice(&bytes[..end - off]);
            }
        }
    }

    /// 把整屏标记为脏（画面被别人覆盖后需要整体重绘）。
    pub fn invalidate_all(&mut self) {
        self.dirty.push(Rect { x: 0, y: 0, w: self.pw, h: self.ph });
    }

    /// 把脏矩形推送到扫描输出并清空脏列表。
    pub fn flush(&mut self, scan: &mut [u8], pitch: usize) {
        let pw = self.pw as usize;
        for r in &self.dirty {
            let x0 = r.x.max(0) as usize;
            let y0 = r.y.max(0) as usize;
            let x1 = ((r.x + r.w).max(0) as usize).min(self.pw as usize);
            let y1 = ((r.y + r.h).max(0) as usize).min(self.ph as usize);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            for row in y0..y1 {
                let start = row * pw + x0;
                let src = cast_slice::<u32, u8>(&self.buf[start..start + (x1 - x0)]);
                let off = row * pitch + x0 * 4;
                let end = (off + src.len()).min(scan.len());
                if end > off {
                    scan[off..end].copy_from_slice(&src[..end - off]);
                }
            }
        }
        self.dirty.clear();
    }
}

/// 把预旋转字形 tile 混合写入物理缓冲（按物理行连续写）。
fn blit_tile(
    buf: &mut [u32],
    pw: i32,
    ph: i32,
    px: i32,
    py: i32,
    tile: &GlyphTile,
    color: Argb,
    dirty: &mut Vec<Rect>,
) {
    let color = theme::resolve(color);
    let cr = (color >> 16) & 0xFF;
    let cg = (color >> 8) & 0xFF;
    let cb = color & 0xFF;
    let tw = tile.tw as i32;
    let th = tile.th as i32;
    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;
    for ty in 0..th {
        let row = py + ty;
        if row < 0 || row >= ph {
            continue;
        }
        let x_start = 0.max(-px);
        let x_end = tw.min(pw - px);
        if x_end <= x_start {
            continue;
        }
        let base = (row * pw) as usize;
        for tx in x_start..x_end {
            let a = tile.cov[(ty * tw + tx) as usize] as u32;
            if a == 0 {
                continue;
            }
            let idx = base + (px + tx) as usize;
            let d = buf[idx];
            let inv = 255 - a;
            let r = (cr * a + ((d >> 16) & 0xFF) * inv + 127) / 255;
            let g = (cg * a + ((d >> 8) & 0xFF) * inv + 127) / 255;
            let b = (cb * a + (d & 0xFF) * inv + 127) / 255;
            buf[idx] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
        }
        min_x = min_x.min(px + x_start);
        max_x = max_x.max(px + x_end);
        min_y = min_y.min(row);
        max_y = max_y.max(row);
    }
    if max_y >= min_y && max_x > min_x {
        dirty.push(Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y + 1 });
    }
}

/// 便捷：在卡片内画一行「标签 + 值」。
pub fn kv(
    c: &mut Canvas,
    x: i32,
    baseline: i32,
    label: &str,
    value: &str,
    vcolor: Argb,
) -> i32 {
    c.text(x, baseline, label, Type::LABEL, Weight::Regular, Palette::FG_MUTED);
    let lw = c.fonts.text_width(label, Type::LABEL, Weight::Regular);
    c.text(x + lw + 10, baseline, value, Type::BODY, Weight::Regular, vcolor)
}
