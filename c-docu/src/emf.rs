//! Windows metafiles (EMF, WMF) as SVG, for rendering the pictures Office documents keep in them:
//! a GDI player — objects (pens, brushes, fonts), the device context and its saved states, the
//! window/viewport mapping and EMF's world transform — drawing lines, shapes, polygons, Béziers,
//! paths, text and bitmaps (DIBs become PNG). EMF+ records are skipped: the GDI records a dual
//! EMF+ file also holds are what is drawn. Clipping is not applied.

use std::fmt::Write;

type R<T> = std::result::Result<T, String>;

fn u16_at(d: &[u8], i: usize) -> u16 {
    d.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0)
}
fn i16_at(d: &[u8], i: usize) -> i16 {
    u16_at(d, i) as i16
}
fn u32_at(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}
fn i32_at(d: &[u8], i: usize) -> i32 {
    u32_at(d, i) as i32
}
fn f32_at(d: &[u8], i: usize) -> f64 {
    f32::from_bits(u32_at(d, i)) as f64
}

pub fn to_svg(data: &[u8]) -> R<String> {
    if data.len() > 44 && u32_at(data, 0) == 1 && &data[40..44] == b" EMF" {
        emf(data)
    } else if data.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || data.starts_with(&[1, 0, 9, 0]) || data.starts_with(&[2, 0, 9, 0]) {
        wmf(data)
    } else {
        Err("not a metafile".into())
    }
}

// ── the device context ──────────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Obj {
    Pen { style: u32, width: f64, color: String },
    Brush { style: u32, color: String },
    Font { height: f64, weight: i32, italic: bool, underline: bool, strike: bool, face: String, escapement: f64, charset: u8 },
    /// A monochrome pattern, drawn in the colors of the moment; the share of 1 bits, and
    /// whether 0 bits take the text color (pattern brushes, as GDI documents) or the background
    /// color (EMF's mono brushes, as LibreOffice plays them and the files expect).
    MonoBrush(f64, bool),
    Other,
}

#[derive(Clone)]
struct Dc {
    pen: Obj,
    brush: Obj,
    font: Obj,
    text_color: String,
    bk_color: String,
    bk_opaque: bool,
    text_align: u32,
    winding: bool,
    map_mode: u32,
    win_org: (f64, f64),
    win_ext: (f64, f64),
    vp_org: (f64, f64),
    vp_ext: (f64, f64),
    /// EMF's world transform: [m11, m12, m21, m22, dx, dy].
    world: [f64; 6],
    cur: (f64, f64),
}

impl Default for Dc {
    fn default() -> Dc {
        Dc {
            pen: Obj::Pen { style: 0, width: 0.0, color: "#000000".into() },
            brush: Obj::Brush { style: 0, color: "#ffffff".into() },
            font: Obj::Font { height: 12.0, weight: 400, italic: false, underline: false, strike: false, face: "Arial".into(), escapement: 0.0, charset: 0 },
            text_color: "#000000".into(),
            bk_color: "#ffffff".into(),
            bk_opaque: true,
            text_align: 0,
            winding: false,
            map_mode: 1,
            win_org: (0.0, 0.0),
            win_ext: (1.0, 1.0),
            vp_org: (0.0, 0.0),
            vp_ext: (1.0, 1.0),
            world: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            cur: (0.0, 0.0),
        }
    }
}

fn colorref(c: u32) -> String {
    format!("#{:02x}{:02x}{:02x}", c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF)
}

/// Two "#rrggbb" colors mixed: `f` of the second.
fn mix(a: &str, b: &str, f: f64) -> String {
    let ch = |s: &str, i: usize| u8::from_str_radix(s.get(1 + i * 2..3 + i * 2).unwrap_or("00"), 16).unwrap_or(0) as f64;
    let m = |i: usize| (ch(a, i) * (1.0 - f) + ch(b, i) * f).round() as u8;
    format!("#{:02x}{:02x}{:02x}", m(0), m(1), m(2))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

struct Player {
    dc: Dc,
    saved: Vec<Dc>,
    objects: Vec<Option<Obj>>,
    out: String,
    /// Device units per millimetre (EMF's reference device), for the fixed map modes.
    px_per_mm: (f64, f64),
    /// A path being recorded (BeginPath … EndPath), as SVG path data.
    path: Option<String>,
    path_done: Option<String>,
    /// WMF: logical coordinates go to the placeable header's box through the window.
    wmf_box: Option<(f64, f64, f64, f64)>,
    images: usize,
}

impl Player {
    fn new() -> Player {
        Player { dc: Dc::default(), saved: Vec::new(), objects: Vec::new(), out: String::new(), px_per_mm: (3.78, 3.78), path: None, path_done: None, wmf_box: None, images: 0 }
    }

    /// Logical → device (the SVG's) coordinates.
    fn pt(&self, x: f64, y: f64) -> (f64, f64) {
        let w = &self.dc.world;
        let (x, y) = (x * w[0] + y * w[2] + w[4], x * w[1] + y * w[3] + w[5]);
        if let Some((l, t, r, b)) = self.wmf_box {
            // WMF: the window maps onto the picture's box.
            let (wo, we) = (self.dc.win_org, self.dc.win_ext);
            let sx = if we.0 != 0.0 { (r - l) / we.0 } else { 1.0 };
            let sy = if we.1 != 0.0 { (b - t) / we.1 } else { 1.0 };
            return (l + (x - wo.0) * sx, t + (y - wo.1) * sy);
        }
        let d = &self.dc;
        match d.map_mode {
            7 | 8 => {
                let sx = if d.win_ext.0 != 0.0 { d.vp_ext.0 / d.win_ext.0 } else { 1.0 };
                let sy = if d.win_ext.1 != 0.0 { d.vp_ext.1 / d.win_ext.1 } else { 1.0 };
                ((x - d.win_org.0) * sx + d.vp_org.0, (y - d.win_org.1) * sy + d.vp_org.1)
            }
            2..=6 => {
                let mm = match d.map_mode {
                    2 => 0.1,
                    3 => 0.01,
                    4 => 0.254,
                    5 => 0.0254,
                    _ => 25.4 / 1440.0,
                };
                ((x - d.win_org.0) * mm * self.px_per_mm.0 + d.vp_org.0, -(y - d.win_org.1) * mm * self.px_per_mm.1 + d.vp_org.1)
            }
            _ => (x - d.win_org.0 + d.vp_org.0, y - d.win_org.1 + d.vp_org.1),
        }
    }

    /// The scale of a logical length (for pen widths and font sizes).
    fn scale(&self) -> (f64, f64) {
        let (a, b) = self.pt(0.0, 0.0);
        let (c, _) = self.pt(1.0, 0.0);
        let (_, e) = self.pt(0.0, 1.0);
        ((c - a).abs().max(1e-9), (e - b).abs().max(1e-9))
    }

    fn stroke(&self) -> String {
        match &self.dc.pen {
            Obj::Pen { style, width, color } => {
                if style & 0xF == 5 {
                    return "stroke=\"none\"".into();
                }
                let w = (width * self.scale().0).max(if *width == 0.0 { 1.0 } else { 0.1 });
                let dash = match style & 0xF {
                    1 => format!(" stroke-dasharray=\"{} {}\"", w * 3.0, w * 1.5),
                    2 => format!(" stroke-dasharray=\"{} {}\"", w, w),
                    3 => format!(" stroke-dasharray=\"{} {} {} {}\"", w * 3.0, w, w, w),
                    4 => format!(" stroke-dasharray=\"{} {} {} {} {} {}\"", w * 3.0, w, w, w, w, w),
                    _ => String::new(),
                };
                let cap = match style & 0xF00 {
                    0x100 => "square",
                    0x200 => "butt",
                    _ => "round",
                };
                let join = match style & 0xF000 {
                    0x1000 => "bevel",
                    0x2000 => "miter",
                    _ => "round",
                };
                format!("stroke=\"{color}\" stroke-width=\"{w:.3}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\"{dash}")
            }
            _ => "stroke=\"none\"".into(),
        }
    }

    fn fill(&self) -> String {
        let rule = if self.dc.winding { "nonzero" } else { "evenodd" };
        if let Obj::MonoBrush(f, zero_text) = self.dc.brush {
            let (zero, one) = if zero_text { (&self.dc.text_color, &self.dc.bk_color) } else { (&self.dc.bk_color, &self.dc.text_color) };
            return format!("fill=\"{}\" fill-rule=\"{rule}\"", mix(zero, one, f));
        }
        match &self.dc.brush {
            Obj::Brush { style, color } => match style {
                1 => "fill=\"none\"".into(),
                2 => format!("fill=\"{color}\" fill-opacity=\"0.5\" fill-rule=\"{rule}\""),
                _ => format!("fill=\"{color}\" fill-rule=\"{rule}\""),
            },
            _ => "fill=\"none\"".into(),
        }
    }

    fn d_of(&self, pts: &[(f64, f64)], close: bool) -> String {
        let mut d = String::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            let (x, y) = self.pt(*x, *y);
            let _ = write!(d, "{}{x:.2} {y:.2} ", if i == 0 { "M" } else { "L" });
        }
        if close {
            d.push('Z');
        }
        d
    }

    fn shape(&mut self, d: &str, fill: bool, stroke: bool) {
        if d.is_empty() {
            return;
        }
        if let Some(p) = self.path.as_mut() {
            p.push_str(d);
            return;
        }
        let f = if fill { self.fill() } else { "fill=\"none\"".into() };
        let s = if stroke { self.stroke() } else { "stroke=\"none\"".into() };
        let _ = writeln!(self.out, "<path d=\"{d}\" {f} {s}/>");
    }

    fn polygon(&mut self, pts: &[(f64, f64)]) {
        let d = self.d_of(pts, true);
        self.shape(&d, true, true);
    }

    fn polyline(&mut self, pts: &[(f64, f64)]) {
        let d = self.d_of(pts, false);
        self.shape(&d, false, true);
    }

    fn polypolygon(&mut self, polys: &[Vec<(f64, f64)>]) {
        let d: String = polys.iter().map(|p| self.d_of(p, true)).collect();
        self.shape(&d, true, true);
    }

    fn bezier(&mut self, start: Option<(f64, f64)>, pts: &[(f64, f64)]) {
        let (first, rest) = match start {
            Some(s) => (s, pts),
            None if !pts.is_empty() => (pts[0], &pts[1..]),
            None => return,
        };
        let (x, y) = self.pt(first.0, first.1);
        let m = format!("M{x:.2} {y:.2} ");
        let mut curves = String::new();
        for c in rest.chunks(3) {
            if c.len() < 3 {
                break;
            }
            let q: Vec<(f64, f64)> = c.iter().map(|v| self.pt(v.0, v.1)).collect();
            let _ = write!(curves, "C{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} ", q[0].0, q[0].1, q[1].0, q[1].1, q[2].0, q[2].1);
            self.dc.cur = c[2];
        }
        if let Some(path) = self.path.as_mut() {
            // In a path, BezierTo goes on from the current point.
            if start.is_none() || path.is_empty() {
                path.push_str(&m);
            }
            path.push_str(&curves);
            return;
        }
        self.shape(&format!("{m}{curves}"), false, true);
    }

    /// LineTo (and PolylineTo): in a path, more of the figure; else lines.
    fn line_to(&mut self, pts: &[(f64, f64)]) {
        if self.path.is_some() {
            let from = self.pt(self.dc.cur.0, self.dc.cur.1);
            let segs: String = pts.iter().map(|q| {
                let (x, y) = self.pt(q.0, q.1);
                format!("L{x:.2} {y:.2} ")
            }).collect();
            let path = self.path.as_mut().unwrap();
            if path.is_empty() {
                let _ = write!(path, "M{:.2} {:.2} ", from.0, from.1);
            }
            path.push_str(&segs);
        } else {
            let mut all = vec![self.dc.cur];
            all.extend_from_slice(pts);
            self.polyline(&all);
        }
        if let Some(l) = pts.last() {
            self.dc.cur = *l;
        }
    }

    fn move_to(&mut self, to: (f64, f64)) {
        self.dc.cur = to;
        if self.path.is_some() {
            let (x, y) = self.pt(to.0, to.1);
            let _ = write!(self.path.as_mut().unwrap(), "M{x:.2} {y:.2} ");
        }
    }

    fn rect(&mut self, l: f64, t: f64, r: f64, b: f64) {
        self.polygon(&[(l, t), (r, t), (r, b), (l, b)]);
    }

    fn ellipse_pts(&self, l: f64, t: f64, r: f64, b: f64, a0: f64, a1: f64) -> Vec<(f64, f64)> {
        let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l).abs() / 2.0, (b - t).abs() / 2.0);
        let mut sweep = a1 - a0;
        if sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
        let n = ((sweep / 0.08).ceil() as usize).max(4);
        (0..=n).map(|i| {
            let a = a0 + sweep * i as f64 / n as f64;
            (cx + rx * a.cos(), cy + ry * a.sin())
        }).collect()
    }

    fn ellipse(&mut self, l: f64, t: f64, r: f64, b: f64) {
        let pts = self.ellipse_pts(l, t, r, b, 0.0, std::f64::consts::TAU);
        self.polygon(&pts);
    }

    /// Arc, chord and pie: from the radial through (x1, y1) to the one through (x2, y2),
    /// counterclockwise as GDI draws by default.
    fn arc(&mut self, kind: u8, l: f64, t: f64, r: f64, b: f64, x1: f64, y1: f64, x2: f64, y2: f64) {
        let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
        let (rx, ry) = ((r - l).abs() / 2.0, (b - t).abs() / 2.0);
        // Angles as on paper (y up); GDI goes counterclockwise from the start radial to the end.
        let a1 = -(y1 - cy).atan2(x1 - cx);
        let a2 = -(y2 - cy).atan2(x2 - cx);
        let mut sweep = a2 - a1;
        if sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
        let n = ((sweep / 0.08).ceil() as usize).max(4);
        let pts: Vec<(f64, f64)> = (0..=n).map(|i| {
            let a = a1 + sweep * i as f64 / n as f64;
            (cx + rx * a.cos(), cy - ry * a.sin())
        }).collect();
        match kind {
            0 => self.polyline(&pts),
            1 => self.polygon(&pts),
            _ => {
                let mut q = vec![(cx, cy)];
                q.extend(pts);
                self.polygon(&q);
            }
        }
    }

    fn select(&mut self, obj: Obj) {
        match obj {
            Obj::Pen { .. } => self.dc.pen = obj,
            Obj::Brush { .. } => self.dc.brush = obj,
            Obj::Font { .. } => self.dc.font = obj,
            Obj::MonoBrush(..) => self.dc.brush = obj,
            Obj::Other => {}
        }
    }

    fn stock(&mut self, n: u32) {
        let o = match n {
            0 => Obj::Brush { style: 0, color: "#ffffff".into() },
            1 => Obj::Brush { style: 0, color: "#c0c0c0".into() },
            2 => Obj::Brush { style: 0, color: "#808080".into() },
            3 => Obj::Brush { style: 0, color: "#404040".into() },
            4 => Obj::Brush { style: 0, color: "#000000".into() },
            5 => Obj::Brush { style: 1, color: "#000000".into() },
            6 => Obj::Pen { style: 0, width: 0.0, color: "#ffffff".into() },
            7 => Obj::Pen { style: 0, width: 0.0, color: "#000000".into() },
            8 => Obj::Pen { style: 5, width: 0.0, color: "#000000".into() },
            10..=17 => Obj::Font { height: 12.0, weight: 400, italic: false, underline: false, strike: false, face: if n == 10 || n == 11 || n == 16 { "Courier New".into() } else { "Arial".into() }, escapement: 0.0, charset: 0 },
            _ => Obj::Other,
        };
        self.select(o);
    }

    fn text(&mut self, x: f64, y: f64, s: &str, dx: Option<Vec<f64>>) {
        if s.trim().is_empty() {
            if self.dc.text_align & 1 != 0 {
                self.dc.cur = (x, y);
            }
            return;
        }
        let (height, weight, italic, underline, strike, face, esc_angle) = match &self.dc.font {
            Obj::Font { height, weight, italic, underline, strike, face, escapement, .. } => (*height, *weight, *italic, *underline, *strike, face.clone(), *escapement),
            _ => (12.0, 400, false, false, false, "Arial".to_string(), 0.0),
        };
        let (ox, oy) = if self.dc.text_align & 1 != 0 { self.dc.cur } else { (x, y) };
        let (px, py) = self.pt(ox, oy);
        let sy = self.scale().1;
        // A negative height is the characters' own; a positive one the cell's (about 1/0.85 of it).
        let size = if height < 0.0 { -height * sy } else if height == 0.0 { 12.0 * sy } else { height * sy * 0.85 };
        let size = size.max(0.5);
        let anchor = match self.dc.text_align & 6 {
            6 => "middle",
            2 => "end",
            _ => "start",
        };
        let base = self.dc.text_align & 24;
        let shift = match base {
            24 => 0.0,
            8 => -size * 0.21,
            _ => size * 0.89,
        };
        let family = match face.to_lowercase().as_str() {
            "" | "ms sans serif" | "ms shell dlg" | "system" => "Arial".to_string(),
            "ms serif" => "Times New Roman".to_string(),
            _ => face,
        };
        // The first family installed wins for every glyph: CJK text needs a CJK font first.
        let cjk = s.chars().any(|c| ('\u{1100}'..='\u{11FF}').contains(&c) || ('\u{2E80}'..='\u{9FFF}').contains(&c) || ('\u{AC00}'..='\u{D7AF}').contains(&c) || ('\u{F900}'..='\u{FAFF}').contains(&c) || ('\u{FF00}'..='\u{FFEF}').contains(&c));
        let fallbacks = if cjk { "NanumGothic, Droid Sans Fallback, Liberation Sans, DejaVu Sans" } else { "Liberation Sans, NanumGothic, Droid Sans Fallback, DejaVu Sans" };
        let deco = match (underline, strike) {
            (true, true) => " text-decoration=\"underline line-through\"",
            (true, false) => " text-decoration=\"underline\"",
            (false, true) => " text-decoration=\"line-through\"",
            _ => "",
        };
        let rot = if esc_angle != 0.0 { format!(" transform=\"rotate({:.2} {px:.2} {py:.2})\"", -esc_angle / 10.0) } else { String::new() };
        // Advances: each character at its own x.
        let xs = match (&dx, anchor) {
            (Some(adv), "start") if adv.len() >= s.chars().count() && s.chars().count() > 1 => {
                let sx = self.scale().0;
                let mut cx = px;
                let mut v = Vec::new();
                for a in adv.iter().take(s.chars().count()) {
                    v.push(format!("{cx:.2}"));
                    cx += a * sx;
                }
                v.join(" ")
            }
            _ => format!("{px:.2}"),
        };
        let _ = writeln!(
            self.out,
            "<text x=\"{xs}\" y=\"{:.2}\" font-family=\"{}, {fallbacks}\" font-size=\"{size:.2}\" fill=\"{}\"{}{}{deco} text-anchor=\"{anchor}\" xml:space=\"preserve\"{rot}>{}</text>",
            py + shift,
            esc(&family),
            self.dc.text_color,
            if weight >= 600 { " font-weight=\"bold\"" } else { "" },
            if italic { " font-style=\"italic\"" } else { "" },
            esc(s)
        );
    }

    /// A device-independent bitmap drawn into a destination rectangle (logical units).
    fn dib(&mut self, bmi: &[u8], bits: &[u8], x: f64, y: f64, w: f64, h: f64) {
        let Some(png) = dib_to_png(bmi, bits) else { return };
        let (x0, y0) = self.pt(x, y);
        let (x1, y1) = self.pt(x + w, y + h);
        let (l, t) = (x0.min(x1), y0.min(y1));
        let (ww, hh) = ((x1 - x0).abs(), (y1 - y0).abs());
        let flip_x = x1 < x0;
        let flip_y = y1 < y0;
        let tr = if flip_x || flip_y {
            format!(" transform=\"translate({:.2} {:.2}) scale({} {}) translate({:.2} {:.2})\"", l + ww / 2.0, t + hh / 2.0, if flip_x { -1 } else { 1 }, if flip_y { -1 } else { 1 }, -(l + ww / 2.0), -(t + hh / 2.0))
        } else {
            String::new()
        };
        self.images += 1;
        let _ = writeln!(self.out, "<image x=\"{l:.2}\" y=\"{t:.2}\" width=\"{ww:.2}\" height=\"{hh:.2}\" preserveAspectRatio=\"none\"{tr} href=\"data:image/png;base64,{}\"/>", base64(&png));
    }

    /// A blit with no source: the brush (PATCOPY), black or white over the rectangle.
    fn rop_fill(&mut self, rop: u32, x: f64, y: f64, w: f64, h: f64) {
        match rop {
            0x0000_0042 | 0x0042 => {
                let b = std::mem::replace(&mut self.dc.brush, Obj::Brush { style: 0, color: "#000000".into() });
                self.fill_rect_brush(x, y, w, h);
                self.dc.brush = b;
            }
            0x00FF_0062 => {
                let b = std::mem::replace(&mut self.dc.brush, Obj::Brush { style: 0, color: "#ffffff".into() });
                self.fill_rect_brush(x, y, w, h);
                self.dc.brush = b;
            }
            0x00F0_0021 | 0x00A0_00C9 | 0x00CC_0020 => self.fill_rect_brush(x, y, w, h),
            _ => {}
        }
    }

    fn fill_rect_brush(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let pen = std::mem::replace(&mut self.dc.pen, Obj::Pen { style: 5, width: 0.0, color: "#000000".into() });
        self.rect(x, y, x + w, y + h);
        self.dc.pen = pen;
    }

    fn save(&mut self) {
        self.saved.push(self.dc.clone());
    }

    fn charset(&self) -> u8 {
        match &self.dc.font {
            Obj::Font { charset, .. } => *charset,
            _ => 0,
        }
    }

    fn restore(&mut self, n: i32) {
        // A negative count is relative (−1: the last saved); a positive one absolute.
        let k = if n < 0 { (-n) as usize } else { self.saved.len().saturating_sub(n as usize - 1).max(1) };
        for _ in 0..k {
            if let Some(d) = self.saved.pop() {
                self.dc = d;
            }
        }
    }

    fn put_obj(&mut self, i: usize, o: Obj) {
        if self.objects.len() <= i {
            self.objects.resize(i + 1, None);
        }
        self.objects[i] = Some(o);
    }

    /// WMF: a new object takes the lowest free slot.
    fn add_obj(&mut self, o: Obj) {
        match self.objects.iter().position(|x| x.is_none()) {
            Some(i) => self.objects[i] = Some(o),
            None => self.objects.push(Some(o)),
        }
    }

    fn finish(self, vb: (f64, f64, f64, f64), size_mm: (f64, f64)) -> String {
        let (x, y, w, h) = vb;
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"{x:.2} {y:.2} {:.2} {:.2}\" width=\"{:.3}mm\" height=\"{:.3}mm\" preserveAspectRatio=\"none\">\n{}</svg>\n",
            w.max(1e-3),
            h.max(1e-3),
            size_mm.0.max(0.01),
            size_mm.1.max(0.01),
            self.out
        )
    }
}

// ── EMF ─────────────────────────────────────────────────────────────────────────────────────

fn points16(d: &[u8], at: usize, n: usize) -> Vec<(f64, f64)> {
    (0..n).map(|k| (i16_at(d, at + k * 4) as f64, i16_at(d, at + k * 4 + 2) as f64)).collect()
}
fn points32(d: &[u8], at: usize, n: usize) -> Vec<(f64, f64)> {
    (0..n).map(|k| (i32_at(d, at + k * 8) as f64, i32_at(d, at + k * 8 + 4) as f64)).collect()
}

/// Text in a font's character set (ANSI records): Japanese, Korean, Chinese, Cyrillic, …
fn decode_ansi(b: &[u8], charset: u8) -> String {
    let enc = match charset {
        128 => encoding_rs::SHIFT_JIS,
        129 | 130 => encoding_rs::EUC_KR,
        134 => encoding_rs::GBK,
        136 => encoding_rs::BIG5,
        161 => encoding_rs::WINDOWS_1253,
        162 => encoding_rs::WINDOWS_1254,
        163 => encoding_rs::WINDOWS_1258,
        177 => encoding_rs::WINDOWS_1255,
        178 => encoding_rs::WINDOWS_1256,
        186 => encoding_rs::WINDOWS_1257,
        204 => encoding_rs::WINDOWS_1251,
        222 => encoding_rs::WINDOWS_874,
        238 => encoding_rs::WINDOWS_1250,
        // Symbol fonts: their code points as they are (the font maps them).
        2 => return b.iter().map(|c| *c as char).collect(),
        _ => encoding_rs::WINDOWS_1252,
    };
    enc.decode_without_bom_handling(b).0.trim_end_matches('\0').to_string()
}

fn utf16(d: &[u8], at: usize, n: usize) -> String {
    let u: Vec<u16> = (0..n).map(|k| u16_at(d, at + k * 2)).collect();
    String::from_utf16_lossy(&u).trim_end_matches('\0').to_string()
}

fn emf(data: &[u8]) -> R<String> {
    let bounds = (i32_at(data, 8) as f64, i32_at(data, 12) as f64, i32_at(data, 16) as f64, i32_at(data, 20) as f64);
    let frame = (i32_at(data, 24) as f64, i32_at(data, 28) as f64, i32_at(data, 32) as f64, i32_at(data, 36) as f64);
    let dev = (i32_at(data, 72) as f64, i32_at(data, 76) as f64);
    let mm = (i32_at(data, 80) as f64, i32_at(data, 84) as f64);
    let mut p = Player::new();
    if mm.0 > 0.0 && mm.1 > 0.0 && dev.0 > 0.0 && dev.1 > 0.0 {
        p.px_per_mm = (dev.0 / mm.0, dev.1 / mm.1);
    }
    let mut i = 0usize;
    let mut n = 0;
    while i + 8 <= data.len() {
        let typ = u32_at(data, i);
        let size = u32_at(data, i + 4) as usize;
        if size < 8 || i + size > data.len() {
            break;
        }
        let r = &data[i..i + size];
        n += 1;
        emf_record(&mut p, typ, r);
        if typ == 14 {
            break;
        }
        i += size;
    }
    if n < 2 {
        return Err("an empty metafile".into());
    }
    // The picture: its frame (0.01 mm) in device units, or the bounds.
    let (fl, ft, fr, fb) = frame;
    let vb = if fr > fl && fb > ft {
        (fl / 100.0 * p.px_per_mm.0, ft / 100.0 * p.px_per_mm.1, (fr - fl) / 100.0 * p.px_per_mm.0, (fb - ft) / 100.0 * p.px_per_mm.1)
    } else {
        (bounds.0, bounds.1, bounds.2 - bounds.0 + 1.0, bounds.3 - bounds.1 + 1.0)
    };
    let size_mm = if fr > fl && fb > ft { ((fr - fl) / 100.0, (fb - ft) / 100.0) } else { (vb.2 / p.px_per_mm.0, vb.3 / p.px_per_mm.1) };
    Ok(p.finish(vb, size_mm))
}

fn emf_record(p: &mut Player, typ: u32, r: &[u8]) {
    match typ {
        // Mapping.
        9 => p.dc.win_ext = (i32_at(r, 8) as f64, i32_at(r, 12) as f64),
        10 => p.dc.win_org = (i32_at(r, 8) as f64, i32_at(r, 12) as f64),
        11 => p.dc.vp_ext = (i32_at(r, 8) as f64, i32_at(r, 12) as f64),
        12 => p.dc.vp_org = (i32_at(r, 8) as f64, i32_at(r, 12) as f64),
        17 => p.dc.map_mode = u32_at(r, 8),
        35 => p.dc.world = [f32_at(r, 8), f32_at(r, 12), f32_at(r, 16), f32_at(r, 20), f32_at(r, 24), f32_at(r, 28)],
        36 => {
            let m = [f32_at(r, 8), f32_at(r, 12), f32_at(r, 16), f32_at(r, 20), f32_at(r, 24), f32_at(r, 28)];
            let w = p.dc.world;
            let mul = |a: [f64; 6], b: [f64; 6]| [a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3], a[4] * b[0] + a[5] * b[2] + b[4], a[4] * b[1] + a[5] * b[3] + b[5]];
            p.dc.world = match u32_at(r, 32) {
                1 => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                2 => mul(m, w),
                3 => mul(w, m),
                _ => m,
            };
        }
        // State.
        33 => p.save(),
        34 => p.restore(i32_at(r, 8)),
        24 => p.dc.text_color = colorref(u32_at(r, 8)),
        25 => p.dc.bk_color = colorref(u32_at(r, 8)),
        18 => p.dc.bk_opaque = u32_at(r, 8) == 2,
        22 => p.dc.text_align = u32_at(r, 8),
        19 => p.dc.winding = u32_at(r, 8) == 2,
        // Objects.
        38 => p.put_obj(u32_at(r, 8) as usize, Obj::Pen { style: u32_at(r, 12), width: i32_at(r, 16) as f64, color: colorref(u32_at(r, 24)) }),
        95 => p.put_obj(u32_at(r, 8) as usize, Obj::Pen { style: u32_at(r, 28), width: u32_at(r, 32) as f64, color: colorref(u32_at(r, 40)) }),
        39 => p.put_obj(u32_at(r, 8) as usize, Obj::Brush { style: u32_at(r, 12), color: colorref(u32_at(r, 16)) }),
        93 => {
            // CreateMonoBrush: a 1-bit pattern.
            let (ob, obits, cbits) = (u32_at(r, 16) as usize, u32_at(r, 24) as usize, u32_at(r, 28) as usize);
            let bits = r.get(obits..obits + cbits).unwrap_or(&[]);
            let w = (i32_at(r, ob + 4).unsigned_abs() as usize).max(1);
            let h = (i32_at(r, ob + 8).unsigned_abs() as usize).max(1);
            let f = ones_share(bits, w, h);
            p.put_obj(u32_at(r, 8) as usize, Obj::MonoBrush(f, false));
        }
        94 => {
            let (ob, cb, obits, cbits) = (u32_at(r, 16) as usize, u32_at(r, 20) as usize, u32_at(r, 24) as usize, u32_at(r, 28) as usize);
            let brush = match (r.get(ob..ob + cb), r.get(obits..obits + cbits)) {
                (Some(bmi), Some(bits)) => pattern_brush(bmi, bits, 5),
                _ => Obj::Brush { style: 0, color: "#808080".into() },
            };
            p.put_obj(u32_at(r, 8) as usize, brush);
        }
        82 => {
            let lf = 12;
            p.put_obj(
                u32_at(r, 8) as usize,
                Obj::Font {
                    height: i32_at(r, lf) as f64,
                    escapement: i32_at(r, lf + 8) as f64,
                    weight: i32_at(r, lf + 16),
                    italic: r.get(lf + 20).copied().unwrap_or(0) != 0,
                    underline: r.get(lf + 21).copied().unwrap_or(0) != 0,
                    strike: r.get(lf + 22).copied().unwrap_or(0) != 0,
                    charset: r.get(lf + 23).copied().unwrap_or(0),
                    face: utf16(r, lf + 28, 32),
                },
            );
        }
        37 => {
            let ih = u32_at(r, 8);
            if ih & 0x8000_0000 != 0 {
                p.stock(ih & 0x7FFF_FFFF);
            } else if let Some(Some(o)) = p.objects.get(ih as usize).cloned() {
                p.select(o);
            }
        }
        40 => {
            let ih = u32_at(r, 8) as usize;
            if let Some(slot) = p.objects.get_mut(ih) {
                *slot = None;
            }
        }
        // Lines and shapes.
        27 => p.move_to((i32_at(r, 8) as f64, i32_at(r, 12) as f64)),
        54 => p.line_to(&[(i32_at(r, 8) as f64, i32_at(r, 12) as f64)]),
        43 => p.rect(i32_at(r, 8) as f64, i32_at(r, 12) as f64, i32_at(r, 16) as f64, i32_at(r, 20) as f64),
        42 => p.ellipse(i32_at(r, 8) as f64, i32_at(r, 12) as f64, i32_at(r, 16) as f64, i32_at(r, 20) as f64),
        44 => {
            // A rounded rectangle: as a rectangle with its corners rounded by the given ellipse.
            let (l, t, rr, b) = (i32_at(r, 8) as f64, i32_at(r, 12) as f64, i32_at(r, 16) as f64, i32_at(r, 20) as f64);
            let (cw, ch) = (i32_at(r, 24) as f64 / 2.0, i32_at(r, 28) as f64 / 2.0);
            let mut pts = Vec::new();
            for (cx, cy, a0) in [(rr - cw, t + ch, -std::f64::consts::FRAC_PI_2), (rr - cw, b - ch, 0.0), (l + cw, b - ch, std::f64::consts::FRAC_PI_2), (l + cw, t + ch, std::f64::consts::PI)] {
                for k in 0..=6 {
                    let a = a0 + std::f64::consts::FRAC_PI_2 * k as f64 / 6.0;
                    pts.push((cx + cw * a.cos(), cy + ch * a.sin()));
                }
            }
            p.polygon(&pts);
        }
        45 | 46 | 47 => {
            let v: Vec<f64> = (0..8).map(|k| i32_at(r, 8 + k * 4) as f64).collect();
            p.arc(match typ {
                45 => 0,
                46 => 1,
                _ => 2,
            }, v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7]);
        }
        // Polygons and lines: 16-bit and 32-bit points.
        86 | 87 | 85 | 88 | 89 | 3 | 4 | 2 | 5 | 6 => {
            let small = typ >= 85;
            let count = u32_at(r, 24) as usize;
            let pts = if small { points16(r, 28, count) } else { points32(r, 28, count) };
            match typ {
                86 | 3 => p.polygon(&pts),
                87 | 4 => p.polyline(&pts),
                85 | 2 => p.bezier(None, &pts),
                88 | 5 => {
                    let cur = p.dc.cur;
                    p.bezier(Some(cur), &pts);
                }
                _ => p.line_to(&pts),
            }
        }
        90 | 91 | 7 | 8 => {
            let small = typ >= 90;
            let npolys = u32_at(r, 24) as usize;
            let mut at = 32 + npolys * 4;
            let mut polys = Vec::new();
            for k in 0..npolys {
                let c = u32_at(r, 32 + k * 4) as usize;
                let pts = if small { points16(r, at, c) } else { points32(r, at, c) };
                at += c * if small { 4 } else { 8 };
                polys.push(pts);
            }
            if typ == 91 || typ == 8 {
                p.polypolygon(&polys);
            } else {
                for poly in polys {
                    p.polyline(&poly);
                }
            }
        }
        // Paths.
        59 => p.path = Some(String::new()),
        60 => p.path_done = p.path.take(),
        61 => {
            if let Some(path) = p.path.as_mut() {
                path.push_str("Z ");
            }
        }
        62 | 63 | 64 => {
            if let Some(d) = p.path_done.take().or_else(|| p.path.take()) {
                let (fill, stroke) = match typ {
                    62 => (true, false),
                    63 => (true, true),
                    _ => (false, true),
                };
                // The path's segments start with M from the shapes added to it; a path built
                // by MoveTo/LineTo needs its first point.
                p.shape(&d, fill, stroke);
            }
        }
        67 | 68 => {
            p.path = None;
            p.path_done = None;
        }
        // Text.
        84 | 83 => {
            let x = i32_at(r, 36) as f64;
            let y = i32_at(r, 40) as f64;
            let nchars = u32_at(r, 44) as usize;
            let off = u32_at(r, 48) as usize;
            let off_dx = u32_at(r, 68) as usize;
            let s = if typ == 84 { utf16(r, off, nchars) } else { decode_ansi(r.get(off..off + nchars).unwrap_or(&[]), p.charset()) };
            let dx = if off_dx > 0 && off_dx + nchars * 4 <= r.len() { Some((0..nchars).map(|k| i32_at(r, off_dx + k * 4) as f64).collect()) } else { None };
            p.text(x, y, &s, dx);
        }
        // Bitmaps.
        81 => {
            let (xd, yd) = (i32_at(r, 24) as f64, i32_at(r, 28) as f64);
            let (off_bmi, cb_bmi, off_bits, cb_bits) = (u32_at(r, 48) as usize, u32_at(r, 52) as usize, u32_at(r, 56) as usize, u32_at(r, 60) as usize);
            let (cxd, cyd) = (i32_at(r, 72) as f64, i32_at(r, 76) as f64);
            if let (Some(bmi), Some(bits)) = (r.get(off_bmi..off_bmi + cb_bmi), r.get(off_bits..off_bits + cb_bits)) {
                p.dib(bmi, bits, xd, yd, cxd, cyd);
            }
        }
        76 | 77 | 114 | 116 => {
            let (xd, yd, cxd, cyd) = (i32_at(r, 24) as f64, i32_at(r, 28) as f64, i32_at(r, 32) as f64, i32_at(r, 36) as f64);
            let (off_bmi, cb_bmi, off_bits, cb_bits) = (u32_at(r, 84) as usize, u32_at(r, 88) as usize, u32_at(r, 92) as usize, u32_at(r, 96) as usize);
            if cb_bmi == 0 {
                // No source: the brush fills the rectangle (PATCOPY and the like).
                p.rop_fill(u32_at(r, 40), xd, yd, cxd, cyd);
            } else if let (Some(bmi), Some(bits)) = (r.get(off_bmi..off_bmi + cb_bmi), r.get(off_bits..off_bits + cb_bits)) {
                p.dib(bmi, bits, xd, yd, cxd, cyd);
            }
        }
        _ => {}
    }
}

// ── WMF ─────────────────────────────────────────────────────────────────────────────────────

fn wmf(data: &[u8]) -> R<String> {
    let mut p = Player::new();
    let mut at = 0;
    let mut inch = 1440.0;
    let mut bbox = None;
    if data.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        let (l, t, r, b) = (i16_at(data, 6) as f64, i16_at(data, 8) as f64, i16_at(data, 10) as f64, i16_at(data, 12) as f64);
        inch = (u16_at(data, 14) as f64).max(1.0);
        bbox = Some((l.min(r), t.min(b), l.max(r), t.max(b)));
        at = 22;
    }
    let header_words = u16_at(data, at + 2) as usize;
    let mut i = at + header_words * 2;
    let mut first_window: Option<((f64, f64), (f64, f64))> = None;
    // Without a placeable header, the window gives the picture's box.
    let mut n = 0;
    while i + 6 <= data.len() {
        let size = u32_at(data, i) as usize * 2;
        let func = u16_at(data, i + 4);
        if size < 6 || i + size > data.len() {
            break;
        }
        let r = &data[i..i + size];
        n += 1;
        if func == 0 {
            break;
        }
        if bbox.is_some() && p.wmf_box.is_none() {
            p.wmf_box = bbox;
        }
        wmf_record(&mut p, func, r);
        if (func == 0x020C || func == 0x020B) && first_window.is_none() && p.dc.win_ext != (1.0, 1.0) {
            first_window = Some((p.dc.win_org, p.dc.win_ext));
        }
        i += size;
    }
    if n < 2 {
        return Err("an empty metafile".into());
    }
    let (vb, size_mm) = match bbox {
        Some((l, t, r, b)) => ((l, t, r - l, b - t), ((r - l) / inch * 25.4, (b - t) / inch * 25.4)),
        None => {
            let ((ox, oy), (ex, ey)) = first_window.unwrap_or(((0.0, 0.0), (1000.0, 1000.0)));
            let (l, t) = (ox.min(ox + ex), oy.min(oy + ey));
            ((l, t, ex.abs(), ey.abs()), (ex.abs() / 1440.0 * 25.4, ey.abs() / 1440.0 * 25.4))
        }
    };
    Ok(p.finish(vb, size_mm))
}

fn wmf_record(p: &mut Player, func: u16, r: &[u8]) {
    let a = |k: usize| i16_at(r, 6 + k * 2) as f64;
    match func {
        0x020B => p.dc.win_org = (a(1), a(0)),
        0x020C => p.dc.win_ext = (a(1), a(0)),
        0x020D => p.dc.vp_org = (a(1), a(0)),
        0x020E => p.dc.vp_ext = (a(1), a(0)),
        0x0103 => p.dc.map_mode = u16_at(r, 6) as u32,
        0x001E => p.save(),
        0x0127 => p.restore(i16_at(r, 6) as i32),
        0x0209 => p.dc.text_color = colorref(u32_at(r, 6)),
        0x0201 => p.dc.bk_color = colorref(u32_at(r, 6)),
        0x0102 => p.dc.bk_opaque = u16_at(r, 6) == 2,
        0x012E => p.dc.text_align = u16_at(r, 6) as u32,
        0x0106 => p.dc.winding = u16_at(r, 6) == 2,
        0x02FA => p.add_obj(Obj::Pen { style: u16_at(r, 6) as u32, width: i16_at(r, 8) as f64, color: colorref(u32_at(r, 12)) }),
        0x02FC => p.add_obj(Obj::Brush { style: u16_at(r, 6) as u32, color: colorref(u32_at(r, 8)) }),
        0x0142 => {
            // DIBCreatePatternBrush: style, usage, a packed DIB.
            let (bmi, bits) = split_dib(r.get(10..).unwrap_or(&[]));
            p.add_obj(pattern_brush(bmi, bits, u16_at(r, 6) as u32));
        }
        0x01F9 => p.add_obj(Obj::Brush { style: 0, color: "#808080".into() }),
        0x02FB => {
            let charset = r.get(19).copied().unwrap_or(0);
            let face_bytes: Vec<u8> = r.get(24..56).map(|b| b.iter().take_while(|c| **c != 0).copied().collect()).unwrap_or_default();
            let face = decode_ansi(&face_bytes, charset);
            p.add_obj(Obj::Font {
                height: i16_at(r, 6) as f64,
                escapement: i16_at(r, 10) as f64,
                weight: i16_at(r, 14) as i32,
                italic: r.get(16).copied().unwrap_or(0) != 0,
                underline: r.get(17).copied().unwrap_or(0) != 0,
                strike: r.get(18).copied().unwrap_or(0) != 0,
                charset,
                face,
            });
        }
        0x00F7 | 0x06FF => p.add_obj(Obj::Other),
        0x012D => {
            let k = u16_at(r, 6) as usize;
            if let Some(Some(o)) = p.objects.get(k).cloned() {
                p.select(o);
            }
        }
        0x01F0 => {
            let k = u16_at(r, 6) as usize;
            if let Some(slot) = p.objects.get_mut(k) {
                *slot = None;
            }
        }
        0x0214 => p.move_to((a(1), a(0))),
        0x0213 => p.line_to(&[(a(1), a(0))]),
        0x041B => p.rect(a(3), a(2), a(1), a(0)),
        0x0418 => p.ellipse(a(3), a(2), a(1), a(0)),
        0x061C => {
            let (l, t, rr, b, cw, ch) = (a(5), a(4), a(3), a(2), a(1) / 2.0, a(0) / 2.0);
            let mut pts = Vec::new();
            for (cx, cy, a0) in [(rr - cw, t + ch, -std::f64::consts::FRAC_PI_2), (rr - cw, b - ch, 0.0), (l + cw, b - ch, std::f64::consts::FRAC_PI_2), (l + cw, t + ch, std::f64::consts::PI)] {
                for k in 0..=6 {
                    let an = a0 + std::f64::consts::FRAC_PI_2 * k as f64 / 6.0;
                    pts.push((cx + cw * an.cos(), cy + ch * an.sin()));
                }
            }
            p.polygon(&pts);
        }
        0x0817 | 0x0830 | 0x081A => {
            // yEnd, xEnd, yStart, xStart, bottom, right, top, left
            let kind = match func {
                0x0817 => 0,
                0x0830 => 1,
                _ => 2,
            };
            p.arc(kind, a(7), a(6), a(5), a(4), a(3), a(2), a(1), a(0));
        }
        0x0324 | 0x0325 => {
            let n = u16_at(r, 6) as usize;
            let pts = points16(r, 8, n);
            if func == 0x0324 {
                p.polygon(&pts);
            } else {
                p.polyline(&pts);
            }
        }
        0x0538 => {
            let np = u16_at(r, 6) as usize;
            let mut at = 8 + np * 2;
            let mut polys = Vec::new();
            for k in 0..np {
                let c = u16_at(r, 8 + k * 2) as usize;
                polys.push(points16(r, at, c));
                at += c * 4;
            }
            p.polypolygon(&polys);
        }
        0x0521 => {
            let n = u16_at(r, 6) as usize;
            let s = decode_ansi(r.get(8..8 + n).unwrap_or(&[]), p.charset());
            let at = 8 + n + (n & 1);
            let (y, x) = (i16_at(r, at) as f64, i16_at(r, at + 2) as f64);
            p.text(x, y, &s, None);
        }
        0x0A32 => {
            let (y, x) = (a(0), a(1));
            let n = u16_at(r, 10) as usize;
            let opts = u16_at(r, 12);
            let mut at = 14;
            if opts & 0x6 != 0 {
                at += 8;
            }
            let s = decode_ansi(r.get(at..at + n).unwrap_or(&[]), p.charset());
            let dx_at = at + n + (n & 1);
            // One advance per byte: with double-byte text, per character no more.
            let dx = if dx_at + n * 2 <= r.len() && s.chars().count() == n { Some((0..n).map(|k| i16_at(r, dx_at + k * 2) as f64).collect()) } else { None };
            p.text(x, y, &s, dx);
        }
        0x0F43 => {
            // StretchDIB: rop(4) usage(2) srcH srcW ySrc xSrc destH destW yDest xDest, DIB.
            let (dh, dw, yd, xd) = (a(7), a(8), a(9), a(10));
            let dib = r.get(28..).unwrap_or(&[]);
            let (bmi, bits) = split_dib(dib);
            p.dib(bmi, bits, xd, yd, dw, dh);
        }
        0x0B41 | 0x0B23 => {
            // (DIB)StretchBlt: rop(4) srcH srcW ySrc xSrc destH destW yDest xDest, a DIB — or,
            // with no bitmap (a record of 14 words), a reserved word before the destination.
            let rop = u32_at(r, 6);
            if r.len() == 28 {
                p.rop_fill(rop, a(10), a(9), a(8), a(7));
            } else if func == 0x0B41 {
                let (bmi, bits) = split_dib(&r[26..]);
                p.dib(bmi, bits, a(9), a(8), a(7), a(6));
            }
        }
        0x0940 | 0x0922 => {
            // (DIB)BitBlt: rop(4) ySrc xSrc height width yDest xDest, a DIB — or, with no bitmap
            // (12 words), a reserved word after xSrc.
            let rop = u32_at(r, 6);
            if r.len() == 24 {
                p.rop_fill(rop, a(8), a(7), a(6), a(5));
            } else if func == 0x0940 {
                let (bmi, bits) = split_dib(&r[22..]);
                p.dib(bmi, bits, a(7), a(6), a(5), a(4));
            }
        }
        0x061D => {
            // PatBlt: rop(4) height width y x.
            p.rop_fill(u32_at(r, 6), a(5), a(4), a(3), a(2));
        }
        _ => {}
    }
}

/// A packed DIB: (its header and color table, its bits).
fn split_dib(d: &[u8]) -> (&[u8], &[u8]) {
    let hs = u32_at(d, 0) as usize;
    if hs < 12 || hs > d.len() {
        return (&[], &[]);
    }
    let (bits, colors, comp) = if hs == 12 { (u16_at(d, 10) as u32, 0, 0) } else { (u16_at(d, 14) as u32, u32_at(d, 32), u32_at(d, 16)) };
    let entries = if bits <= 8 { if colors == 0 { 1 << bits } else { colors } } else if comp == 3 && hs == 40 { 3 } else { colors };
    let pal = entries as usize * if hs == 12 { 3 } else { 4 };
    let split = (hs + pal).min(d.len());
    (&d[..split], &d[split..])
}

// ── bitmaps ─────────────────────────────────────────────────────────────────────────────────

/// A DIB (header + color table, and its bits) as PNG.
fn dib_to_png(bmi: &[u8], bits: &[u8]) -> Option<Vec<u8>> {
    let hs = u32_at(bmi, 0) as usize;
    if hs >= 40 && matches!(u32_at(bmi, 16), 4 | 5) {
        return Some(bits.to_vec()).filter(|b| b.starts_with(b"\x89PNG") || b.starts_with(&[0xFF, 0xD8]));
    }
    let (w, h, raw) = dib_rgba(bmi, bits)?;
    Some(png_rgba(w as u32, h as u32, &raw))
}

/// A pattern brush from a DIB: a 1-bit BS_PATTERN one draws in the text and background colors
/// (the share of its 1 bits); a DIB pattern (BS_DIBPATTERN…) in its own colors, as their average.
fn pattern_brush(bmi: &[u8], bits: &[u8], style: u32) -> Obj {
    let hs = u32_at(bmi, 0) as usize;
    let bpp = if hs == 12 { u16_at(bmi, 10) } else { u16_at(bmi, 14) };
    if bpp == 1 && style == 3 {
        let (w, h) = if hs == 12 { (u16_at(bmi, 4) as usize, u16_at(bmi, 6) as usize) } else { (i32_at(bmi, 4).unsigned_abs() as usize, i32_at(bmi, 8).unsigned_abs() as usize) };
        return Obj::MonoBrush(ones_share(bits, w.max(1), h.max(1)), true);
    }
    Obj::Brush { style: 0, color: dib_color(bmi, bits).unwrap_or_else(|| "#808080".into()) }
}

/// The share of 1 bits in a 1-bit bitmap (rows padded to 32 bits).
fn ones_share(bits: &[u8], w: usize, h: usize) -> f64 {
    let stride = w.div_ceil(32) * 4;
    let mut ones = 0usize;
    for y in 0..h {
        for x in 0..w {
            if bits.get(y * stride + x / 8).map(|b| b & (0x80 >> (x % 8)) != 0).unwrap_or(false) {
                ones += 1;
            }
        }
    }
    ones as f64 / (w * h) as f64
}

/// The average color of a DIB (for pattern brushes, drawn as their color).
fn dib_color(bmi: &[u8], bits: &[u8]) -> Option<String> {
    let (w, h, raw) = dib_rgba(bmi, bits)?;
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            let o = y * (w * 4 + 1) + 1 + x * 4;
            r += raw[o] as u64;
            g += raw[o + 1] as u64;
            b += raw[o + 2] as u64;
            n += 1;
        }
    }
    (n > 0).then(|| format!("#{:02x}{:02x}{:02x}", r / n, g / n, b / n))
}

/// A DIB's pixels as PNG scanlines (a filter byte, then RGBA), with its size.
fn dib_rgba(bmi: &[u8], bits: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let hs = u32_at(bmi, 0) as usize;
    if hs < 12 {
        return None;
    }
    let (w, h, bpp, comp, colors) = if hs == 12 {
        (u16_at(bmi, 4) as i64, i16_at(bmi, 6) as i64, u16_at(bmi, 10) as u32, 0, 0)
    } else {
        (i32_at(bmi, 4) as i64, i32_at(bmi, 8) as i64, u16_at(bmi, 14) as u32, u32_at(bmi, 16), u32_at(bmi, 32))
    };
    if !matches!(comp, 0 | 3) {
        return None;
    }
    let (w, top_down) = (w.unsigned_abs() as usize, h < 0);
    let h = h.unsigned_abs() as usize;
    if w == 0 || h == 0 || w * h > 40_000_000 {
        return None;
    }
    let entries = if bpp <= 8 { if colors == 0 { 1usize << bpp } else { colors as usize } } else { 0 };
    let ps = if hs == 12 { 3 } else { 4 };
    let pal: Vec<[u8; 3]> = (0..entries).map(|k| {
        let o = hs + k * ps;
        [bmi.get(o + 2).copied().unwrap_or(0), bmi.get(o + 1).copied().unwrap_or(0), bmi.get(o).copied().unwrap_or(0)]
    }).collect();
    let masks = if comp == 3 { Some((u32_at(bmi, 40), u32_at(bmi, 44), u32_at(bmi, 48))) } else { None };
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    let mut raw = Vec::with_capacity((w * 4 + 1) * h);
    let mut any_alpha = false;
    for y in 0..h {
        let row = if top_down { y } else { h - 1 - y };
        let line = bits.get(row * stride..row * stride + stride).unwrap_or(&[]);
        raw.push(0u8);
        for x in 0..w {
            let px: [u8; 4] = match bpp {
                1 | 4 | 8 => {
                    let bitpos = x * bpp as usize;
                    let byte = line.get(bitpos / 8).copied().unwrap_or(0);
                    let idx = ((byte >> (8 - bpp as usize - bitpos % 8)) & ((1u16 << bpp) - 1) as u8) as usize;
                    let c = pal.get(idx).copied().unwrap_or([0, 0, 0]);
                    [c[0], c[1], c[2], 255]
                }
                16 => {
                    let v = u16_at(line, x * 2) as u32;
                    let (rm, gm, bm) = masks.unwrap_or((0x7C00, 0x03E0, 0x001F));
                    let ch = |m: u32| -> u8 {
                        if m == 0 {
                            return 0;
                        }
                        let shift = m.trailing_zeros();
                        let max = m >> shift;
                        (((v & m) >> shift) * 255 / max) as u8
                    };
                    [ch(rm), ch(gm), ch(bm), 255]
                }
                24 => {
                    let o = x * 3;
                    [line.get(o + 2).copied().unwrap_or(0), line.get(o + 1).copied().unwrap_or(0), line.get(o).copied().unwrap_or(0), 255]
                }
                32 => {
                    let o = x * 4;
                    let a = line.get(o + 3).copied().unwrap_or(0);
                    if a != 0 {
                        any_alpha = true;
                    }
                    [line.get(o + 2).copied().unwrap_or(0), line.get(o + 1).copied().unwrap_or(0), line.get(o).copied().unwrap_or(0), a]
                }
                _ => return None,
            };
            raw.extend_from_slice(&px);
        }
    }
    // 32-bit pixels whose alpha is all zero are opaque (the byte is unused).
    if bpp == 32 && !any_alpha {
        for y in 0..h {
            for x in 0..w {
                raw[y * (w * 4 + 1) + 1 + x * 4 + 3] = 255;
            }
        }
    }
    Some((w, h, raw))
}

fn png_rgba(w: u32, h: u32, filtered: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crate::zip::crc32(&c).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(filtered, 6));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn base64(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
