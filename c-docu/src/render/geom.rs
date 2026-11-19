//! DrawingML geometry: the preset shapes (from ECMA-376's presetShapeDefinitions.xml) and custom
//! geometry, their guide formulas evaluated for a shape's size and adjustments, their paths
//! turned into Typst curves.

use crate::xml::{Document, Element, A, NONE};
use std::collections::HashMap;
use std::sync::OnceLock;

static DEFS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/presetShapeDefinitions.xml"));

fn defs() -> &'static HashMap<String, Element> {
    static MAP: OnceLock<HashMap<String, Element>> = OnceLock::new();
    MAP.get_or_init(|| {
        let text = String::from_utf8_lossy(DEFS);
        let doc = Document::parse(&text).expect("preset shape definitions");
        doc.root.elements().map(|e| (e.local().to_string(), e.clone())).collect()
    })
}

pub fn has_preset(name: &str) -> bool {
    defs().contains_key(name)
}

#[derive(Clone, Debug)]
pub enum Cmd {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Quad(f64, f64, f64, f64),
    Close,
}

#[derive(Clone, Debug)]
pub struct Path {
    pub cmds: Vec<Cmd>,
    /// none, norm, lighten, lightenLess, darken, darkenLess
    pub fill: String,
    pub stroke: bool,
}

pub struct Shape {
    pub paths: Vec<Path>,
    /// The text rectangle (l, t, r, b) in points.
    pub text_rect: (f64, f64, f64, f64),
}

struct Guides {
    v: HashMap<String, f64>,
}

impl Guides {
    fn new(w: f64, h: f64) -> Guides {
        let mut v = HashMap::new();
        let ss = w.min(h);
        let ls = w.max(h);
        for (k, x) in [
            ("w", w),
            ("h", h),
            ("l", 0.0),
            ("t", 0.0),
            ("r", w),
            ("b", h),
            ("hc", w / 2.0),
            ("vc", h / 2.0),
            ("ss", ss),
            ("ls", ls),
            ("cd2", 10800000.0),
            ("cd4", 5400000.0),
            ("cd8", 2700000.0),
            ("3cd4", 16200000.0),
            ("3cd8", 8100000.0),
            ("5cd8", 13500000.0),
            ("7cd8", 18900000.0),
        ] {
            v.insert(k.to_string(), x);
        }
        for d in [2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 32.0] {
            v.insert(format!("wd{}", d as i32), w / d);
            v.insert(format!("hd{}", d as i32), h / d);
            v.insert(format!("ssd{}", d as i32), ss / d);
        }
        Guides { v }
    }

    fn get(&self, s: &str) -> f64 {
        if let Some(x) = self.v.get(s) {
            return *x;
        }
        s.parse::<f64>().unwrap_or(0.0)
    }

    fn eval(&mut self, name: &str, fmla: &str) {
        let p: Vec<&str> = fmla.split_whitespace().collect();
        let a = |i: usize| p.get(i).map(|x| self.get(x)).unwrap_or(0.0);
        let deg = |x: f64| (x / 60000.0).to_radians();
        let r = match p.first().copied().unwrap_or("val") {
            "*/" => {
                let z = a(3);
                if z == 0.0 { 0.0 } else { a(1) * a(2) / z }
            }
            "+-" => a(1) + a(2) - a(3),
            "+/" => {
                let z = a(3);
                if z == 0.0 { 0.0 } else { (a(1) + a(2)) / z }
            }
            "?:" => {
                if a(1) > 0.0 {
                    a(2)
                } else {
                    a(3)
                }
            }
            "abs" => a(1).abs(),
            "at2" => a(2).atan2(a(1)).to_degrees() * 60000.0,
            "cat2" => a(1) * a(3).atan2(a(2)).cos(),
            "sat2" => a(1) * a(3).atan2(a(2)).sin(),
            "cos" => a(1) * deg(a(2)).cos(),
            "sin" => a(1) * deg(a(2)).sin(),
            "tan" => a(1) * deg(a(2)).tan(),
            "max" => a(1).max(a(2)),
            "min" => a(1).min(a(2)),
            "mod" => (a(1) * a(1) + a(2) * a(2) + a(3) * a(3)).sqrt(),
            "pin" => {
                let (lo, x, hi) = (a(1), a(2), a(3));
                if x < lo {
                    lo
                } else if x > hi {
                    hi
                } else {
                    x
                }
            }
            "sqrt" => a(1).max(0.0).sqrt(),
            "val" => a(1),
            _ => 0.0,
        };
        self.v.insert(name.to_string(), r);
    }
}

/// The geometry of a preset (`prst`) with the adjustments of an `a:avLst`, or of an `a:custGeom`,
/// at w × h points.
pub fn shape(preset: Option<&str>, custom: Option<&Element>, av: Option<&Element>, w: f64, h: f64) -> Option<Shape> {
    let def: &Element = match (custom, preset) {
        (Some(c), _) => c,
        (None, Some(p)) => defs().get(p)?,
        _ => return None,
    };
    let mut g = Guides::new(w, h);
    // Adjust values: the definition's defaults, then the shape's own.
    if let Some(defaults) = def.child(A, "avLst").or_else(|| def.elements().find(|e| e.local() == "avLst")) {
        for gd in defaults.elements() {
            if let (Some(n), Some(f)) = (gd.attr_ns(NONE, "name"), gd.attr_ns(NONE, "fmla")) {
                g.eval(n, f);
            }
        }
    }
    if let Some(av) = av {
        for gd in av.elements() {
            if let (Some(n), Some(f)) = (gd.attr_ns(NONE, "name"), gd.attr_ns(NONE, "fmla")) {
                g.eval(n, f);
            }
        }
    }
    if let Some(gl) = def.elements().find(|e| e.local() == "gdLst") {
        for gd in gl.elements() {
            if let (Some(n), Some(f)) = (gd.attr_ns(NONE, "name"), gd.attr_ns(NONE, "fmla")) {
                g.eval(n, f);
            }
        }
    }
    let rect = def.elements().find(|e| e.local() == "rect");
    let text_rect = match rect {
        Some(r) => (g.get(r.attr_ns(NONE, "l").unwrap_or("l")), g.get(r.attr_ns(NONE, "t").unwrap_or("t")), g.get(r.attr_ns(NONE, "r").unwrap_or("r")), g.get(r.attr_ns(NONE, "b").unwrap_or("b"))),
        None => (0.0, 0.0, w, h),
    };
    let mut paths = Vec::new();
    let Some(pl) = def.elements().find(|e| e.local() == "pathLst") else { return Some(Shape { paths, text_rect }) };
    for p in pl.elements() {
        let pw = p.attr_ns(NONE, "w").and_then(|v| v.parse::<f64>().ok());
        let ph = p.attr_ns(NONE, "h").and_then(|v| v.parse::<f64>().ok());
        // A path with its own size: its coordinates scale to the shape's.
        let (sx, sy) = (pw.filter(|v| *v > 0.0).map(|v| w / v).unwrap_or(1.0), ph.filter(|v| *v > 0.0).map(|v| h / v).unwrap_or(1.0));
        let scaled = custom.is_some() || pw.is_some();
        let pt = |e: &Element, g: &Guides| -> (f64, f64) {
            let x = g.get(e.attr_ns(NONE, "x").unwrap_or("0"));
            let y = g.get(e.attr_ns(NONE, "y").unwrap_or("0"));
            if scaled {
                (x * sx, y * sy)
            } else {
                (x, y)
            }
        };
        let mut cmds = Vec::new();
        let mut cur = (0.0, 0.0);
        for c in p.elements() {
            match c.local() {
                "moveTo" => {
                    if let Some(q) = c.elements().next() {
                        cur = pt(q, &g);
                        cmds.push(Cmd::Move(cur.0, cur.1));
                    }
                }
                "lnTo" => {
                    if let Some(q) = c.elements().next() {
                        cur = pt(q, &g);
                        cmds.push(Cmd::Line(cur.0, cur.1));
                    }
                }
                "cubicBezTo" => {
                    let q: Vec<(f64, f64)> = c.elements().map(|e| pt(e, &g)).collect();
                    if q.len() == 3 {
                        cmds.push(Cmd::Cubic(q[0].0, q[0].1, q[1].0, q[1].1, q[2].0, q[2].1));
                        cur = q[2];
                    }
                }
                "quadBezTo" => {
                    let q: Vec<(f64, f64)> = c.elements().map(|e| pt(e, &g)).collect();
                    if q.len() == 2 {
                        cmds.push(Cmd::Quad(q[0].0, q[0].1, q[1].0, q[1].1));
                        cur = q[1];
                    }
                }
                "arcTo" => {
                    let gv = |a: &str| g.get(c.attr_ns(NONE, a).unwrap_or("0"));
                    let (mut wr, mut hr) = (gv("wR"), gv("hR"));
                    if scaled {
                        wr *= sx;
                        hr *= sy;
                    }
                    let st = gv("stAng") / 60000.0;
                    let sw = gv("swAng") / 60000.0;
                    cur = arc(&mut cmds, cur, wr, hr, st, sw);
                }
                "close" => cmds.push(Cmd::Close),
                _ => {}
            }
        }
        let fill = p.attr_ns(NONE, "fill").unwrap_or("norm").to_string();
        let stroke = p.attr_ns(NONE, "stroke").map(|v| v != "0" && v != "false").unwrap_or(true);
        paths.push(Path { cmds, fill, stroke });
    }
    Some(Shape { paths, text_rect })
}

/// An elliptical arc from `cur` (on the ellipse at angle `st`) sweeping `sw` degrees, as cubic
/// Béziers; returns the end point. DrawingML angles are of the ellipse's parametric form.
fn arc(cmds: &mut Vec<Cmd>, cur: (f64, f64), wr: f64, hr: f64, st: f64, sw: f64) -> (f64, f64) {
    if wr == 0.0 || hr == 0.0 || sw == 0.0 {
        return cur;
    }
    // DrawingML's angle is the visual angle: convert to the parametric one.
    let param = |deg: f64| {
        let r = deg.to_radians();
        (r.sin() * wr).atan2(r.cos() * hr)
    };
    let t0 = param(st);
    let cx = cur.0 - wr * t0.cos();
    let cy = cur.1 - hr * t0.sin();
    let end_vis = st + sw;
    let mut t1 = param(end_vis);
    // Keep the sweep's direction and size.
    let full = sw.abs() >= 360.0;
    if full {
        t1 = t0 + sw.signum() * std::f64::consts::TAU;
    } else if sw > 0.0 {
        while t1 <= t0 {
            t1 += std::f64::consts::TAU;
        }
        while t1 - t0 > std::f64::consts::TAU {
            t1 -= std::f64::consts::TAU;
        }
    } else {
        while t1 >= t0 {
            t1 -= std::f64::consts::TAU;
        }
        while t0 - t1 > std::f64::consts::TAU {
            t1 += std::f64::consts::TAU;
        }
    }
    let n = ((t1 - t0).abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = (t1 - t0) / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let mut a = t0;
    let mut p = cur;
    for _ in 0..n {
        let b = a + step;
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        let c1 = (p.0 - k * wr * sa, p.1 + k * hr * ca);
        let e = (cx + wr * cb, cy + hr * sb);
        let c2 = (e.0 + k * wr * sb, e.1 - k * hr * cb);
        cmds.push(Cmd::Cubic(c1.0, c1.1, c2.0, c2.1, e.0, e.1));
        p = e;
        a = b;
    }
    p
}

/// Typst curve components for a path.
pub fn curve_components(p: &Path) -> String {
    let f = |v: f64| format!("{}pt", (v * 100.0).round() / 100.0);
    let mut out = Vec::new();
    for c in &p.cmds {
        out.push(match c {
            Cmd::Move(x, y) => format!("curve.move(({}, {}))", f(*x), f(*y)),
            Cmd::Line(x, y) => format!("curve.line(({}, {}))", f(*x), f(*y)),
            Cmd::Cubic(a, b, c2, d, e, g) => format!("curve.cubic(({}, {}), ({}, {}), ({}, {}))", f(*a), f(*b), f(*c2), f(*d), f(*e), f(*g)),
            Cmd::Quad(a, b, c2, d) => format!("curve.quad(({}, {}), ({}, {}))", f(*a), f(*b), f(*c2), f(*d)),
            Cmd::Close => "curve.close(mode: \"straight\")".to_string(),
        });
    }
    out.join(", ")
}
