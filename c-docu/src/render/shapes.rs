//! DrawingML shapes as Typst: fills (solid, gradient, pattern), outlines (width, dash, color) and
//! geometry (presets and custom paths).

use super::geom;
use super::office::{self, Theme};
use super::typ::{pt, rgb, rgba};
use crate::xml::{Element, A, NONE};
use std::collections::HashMap;

/// A paint (Typst expression) or none.
pub fn paint_of_fill(fill: &Element, theme: &Theme, map: &HashMap<String, String>, ph: Option<&(String, f64)>) -> Option<String> {
    match fill.local() {
        "noFill" => None,
        "solidFill" => office::dml_color(fill, theme, map, ph).map(|(h, a)| if a < 1.0 { rgba(&h, a) } else { rgb(&h) }),
        "gradFill" => {
            let stops: Vec<(f64, String)> = fill
                .child(A, "gsLst")
                .map(|l| {
                    l.children_named(A, "gs")
                        .filter_map(|gs| {
                            let pos = gs.attr_ns(NONE, "pos").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100000.0;
                            office::dml_color(gs, theme, map, ph).map(|(h, a)| (pos, if a < 1.0 { rgba(&h, a) } else { rgb(&h) }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if stops.is_empty() {
                return None;
            }
            if stops.len() == 1 {
                return Some(stops[0].1.clone());
            }
            let mut stops = stops;
            stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let list: Vec<String> = stops.iter().map(|(p, c)| format!("({c}, {}%)", (p * 1000.0).round() / 10.0)).collect();
            if fill.child(A, "path").is_some() {
                Some(format!("gradient.radial({})", list.join(", ")))
            } else {
                let ang = fill.child(A, "lin").and_then(|l| l.attr_ns(NONE, "ang")).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 60000.0;
                Some(format!("gradient.linear({}, angle: {}deg)", list.join(", "), (ang * 100.0).round() / 100.0))
            }
        }
        "pattFill" => {
            // The foreground and background mixed: the pattern's average.
            let fg = fill.child(A, "fgClr").and_then(|c| office::dml_color(c, theme, map, ph));
            let bg = fill.child(A, "bgClr").and_then(|c| office::dml_color(c, theme, map, ph));
            match (fg, bg) {
                (Some((f, _)), Some((b, _))) => {
                    let (r1, g1, b1) = office::hex_rgb(&f);
                    let (r2, g2, b2) = office::hex_rgb(&b);
                    Some(rgb(&office::rgb_hex((r1 + r2 * 2.0) / 3.0, (g1 + g2 * 2.0) / 3.0, (b1 + b2 * 2.0) / 3.0)))
                }
                (Some((f, _)), None) => Some(rgb(&f)),
                _ => None,
            }
        }
        _ => None,
    }
}

const FILL_NAMES: &[&str] = &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];

/// The fill of a shape: its own (spPr), else the style's fillRef with the theme's fill style.
pub fn fill_of(sppr: Option<&Element>, style: Option<&Element>, theme: &Theme, map: &HashMap<String, String>) -> Option<String> {
    if let Some(sp) = sppr {
        if let Some(f) = sp.elements().find(|e| FILL_NAMES.contains(&e.local())) {
            if f.local() == "blipFill" {
                return Some("rgb(\"#D9D9D9\")".into());
            }
            return paint_of_fill(f, theme, map, None);
        }
    }
    let fr = style?.elements().find(|e| e.local() == "fillRef")?;
    let idx: usize = fr.attr_ns(NONE, "idx").and_then(|v| v.parse().ok()).unwrap_or(0);
    if idx == 0 {
        return None;
    }
    let ph = office::dml_color(fr, theme, map, None);
    // Theme fill styles are almost always solid placeholder fills.
    ph.map(|(h, a)| if a < 1.0 { rgba(&h, a) } else { rgb(&h) })
}

/// The outline: (Typst stroke or "none").
pub fn stroke_of(sppr: Option<&Element>, style: Option<&Element>, theme: &Theme, map: &HashMap<String, String>) -> String {
    let ln = sppr.and_then(|s| s.child(A, "ln"));
    let lnref = style.and_then(|s| s.elements().find(|e| e.local() == "lnRef"));
    let ref_idx: usize = lnref.and_then(|r| r.attr_ns(NONE, "idx")).and_then(|v| v.parse().ok()).unwrap_or(0);
    let ref_color = lnref.and_then(|r| office::dml_color(r, theme, map, None));
    let width = ln.and_then(|l| l.attr_ns(NONE, "w")).and_then(|v| v.parse::<f64>().ok()).map(|v| v / 12700.0).unwrap_or(match ref_idx {
        0 => 0.0,
        1 => 0.75,
        2 => 1.0,
        _ => 1.5,
    });
    let paint = match ln.and_then(|l| l.elements().find(|e| FILL_NAMES.contains(&e.local()))) {
        Some(f) => paint_of_fill(f, theme, map, None),
        None if ln.is_some() && ref_idx == 0 => None,
        None => ref_color.map(|(h, a)| if a < 1.0 { rgba(&h, a) } else { rgb(&h) }),
    };
    let Some(paint) = paint else { return "none".into() };
    if width <= 0.0 && ref_idx == 0 && ln.and_then(|l| l.attr_ns(NONE, "w")).is_none() {
        return "none".into();
    }
    let dash = ln.and_then(|l| l.child(A, "prstDash")).and_then(|d| d.attr_ns(NONE, "val")).map(|d| match d {
        "dot" | "sysDot" => "dotted",
        "dash" | "sysDash" | "lgDash" => "dashed",
        "dashDot" | "lgDashDot" | "sysDashDot" => "dash-dotted",
        "lgDashDotDot" | "sysDashDotDot" => "dash-dotted",
        _ => "solid",
    });
    let mut s = format!("(paint: {paint}, thickness: {}", pt(width.max(0.25)));
    if let Some(d) = dash {
        if d != "solid" {
            s.push_str(&format!(", dash: \"{d}\""));
        }
    }
    s.push(')');
    s
}

/// A shape's drawing at w × h (from its top-left corner): its preset (or custom) geometry with
/// fill and stroke.
pub fn preset(name: &str, w: f64, h: f64, fill: &Option<String>, stroke: &str, geom_el: Option<&Element>) -> String {
    let fill_s = fill.clone().unwrap_or_else(|| "none".into());
    match name {
        "rect" | "flowChartProcess" => return format!("rect(width: {}, height: {}, fill: {fill_s}, stroke: {stroke})", pt(w), pt(h)),
        "ellipse" | "flowChartConnector" => return format!("ellipse(width: {}, height: {}, fill: {fill_s}, stroke: {stroke})", pt(w), pt(h)),
        _ => {}
    }
    let av = geom_el.and_then(|g| g.child(A, "avLst"));
    match geom::shape(Some(name), None, av, w, h) {
        Some(s) => draw(&s, w, h, fill, stroke),
        None => format!("rect(width: {}, height: {}, fill: {fill_s}, stroke: {stroke})", pt(w), pt(h)),
    }
}

/// Custom geometry (`a:custGeom`).
pub fn custom(cg: &Element, w: f64, h: f64, fill: &Option<String>, stroke: &str) -> String {
    match geom::shape(None, Some(cg), None, w, h) {
        Some(s) => draw(&s, w, h, fill, stroke),
        None => String::from("none"),
    }
}

fn draw(s: &geom::Shape, w: f64, h: f64, fill: &Option<String>, stroke: &str) -> String {
    let mut layers = Vec::new();
    for p in &s.paths {
        let comps = geom::curve_components(p);
        if comps.is_empty() {
            continue;
        }
        let f = if p.fill == "none" {
            "none".to_string()
        } else {
            match (fill, p.fill.as_str()) {
                (Some(f), "darken" | "darkenLess") if f.starts_with("rgb(") => format!("{f}.darken({})", if p.fill == "darken" { "40%" } else { "20%" }),
                (Some(f), "lighten" | "lightenLess") if f.starts_with("rgb(") => format!("{f}.lighten({})", if p.fill == "lighten" { "40%" } else { "20%" }),
                (Some(f), _) => f.clone(),
                (None, _) => "none".into(),
            }
        };
        let st = if p.stroke { stroke.to_string() } else { "none".into() };
        layers.push(format!("place(top + left, curve(fill: {f}, stroke: {st}, {comps}))"));
    }
    format!("box(width: {}, height: {})[#{}]", pt(w), pt(h), layers.join("#"))
}
