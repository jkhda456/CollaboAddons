//! Pictures for Typst: PNG, JPEG, GIF, WebP and SVG as they are; BMP and TIFF converted to PNG;
//! EMF and WMF drawn as SVG.

use super::typ::{pt, s};
use crate::xml::{Element, NONE};

pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    Svg,
    Bmp,
    Tiff,
    Emf,
    Wmf,
    Unknown,
}

pub fn kind(d: &[u8]) -> Kind {
    if d.starts_with(b"\x89PNG") {
        Kind::Png
    } else if d.starts_with(&[0xFF, 0xD8]) {
        Kind::Jpeg
    } else if d.starts_with(b"GIF8") {
        Kind::Gif
    } else if d.len() > 12 && &d[0..4] == b"RIFF" && &d[8..12] == b"WEBP" {
        Kind::Webp
    } else if d.starts_with(b"BM") {
        Kind::Bmp
    } else if d.starts_with(b"II*\0") || d.starts_with(b"MM\0*") {
        Kind::Tiff
    } else if d.len() > 44 && d[0..4] == [1, 0, 0, 0] && &d[40..44] == b" EMF" {
        Kind::Emf
    } else if d.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || d.starts_with(&[1, 0, 9, 0]) || d.starts_with(&[2, 0, 9, 0]) {
        Kind::Wmf
    } else {
        let head = String::from_utf8_lossy(&d[..d.len().min(512)]).to_ascii_lowercase();
        if head.contains("<svg") {
            Kind::Svg
        } else {
            Kind::Unknown
        }
    }
}

/// A picture as a file Typst reads: (virtual path, bytes), or None (with a warning).
pub fn prepare(part: &str, data: Vec<u8>, warnings: &mut Vec<String>) -> Option<(String, Vec<u8>)> {
    let base = part.replace('/', "_");
    match kind(&data) {
        Kind::Png => Some((format!("media/{base}.png"), data)),
        Kind::Jpeg => Some((format!("media/{base}.jpg"), data)),
        Kind::Gif => Some((format!("media/{base}.gif"), data)),
        Kind::Webp => Some((format!("media/{base}.webp"), data)),
        Kind::Svg => Some((format!("media/{base}.svg"), data)),
        Kind::Emf | Kind::Wmf => match crate::emf::to_svg(&data) {
            Ok(svg) => {
                // DOCU_DUMP_SVG=DIR keeps the metafiles' SVG, to look at.
                if let Ok(dir) = std::env::var("DOCU_DUMP_SVG") {
                    let _ = std::fs::write(format!("{dir}/{base}.svg"), &svg);
                }
                Some((format!("media/{base}.svg"), svg.into_bytes()))
            }
            Err(e) => {
                warnings.push(format!("{part}: a metafile picture that could not be drawn ({e})"));
                None
            }
        },
        Kind::Bmp | Kind::Tiff => match convert_raster(&data) {
            Some(png) => Some((format!("media/{base}.png"), png)),
            None => {
                warnings.push(format!("{part}: a picture in a format that could not be read"));
                None
            }
        },
        Kind::Unknown => {
            warnings.push(format!("{part}: a picture in an unknown format"));
            None
        }
    }
}

fn convert_raster(data: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// A picture of w × h points, cropped by an `a:srcRect` (fractions in 1/100000).
pub fn image_expr(file: &str, w: f64, h: f64, src: Option<&Element>) -> String {
    let f = |a: &str| src.and_then(|e| e.attr_ns(NONE, a)).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100000.0;
    let (l, t, r, b) = (f("l"), f("t"), f("r"), f("b"));
    let path = s(&format!("/{file}"));
    if l == 0.0 && t == 0.0 && r == 0.0 && b == 0.0 {
        return format!("image({path}, width: {}, height: {}, fit: \"stretch\")", pt(w), pt(h));
    }
    let fw = w / (1.0 - l - r).max(0.01);
    let fh = h / (1.0 - t - b).max(0.01);
    format!(
        "box(width: {}, height: {}, clip: true, place(top + left, dx: {}, dy: {}, image({path}, width: {}, height: {}, fit: \"stretch\")))",
        pt(w),
        pt(h),
        pt(-l * fw),
        pt(-t * fh),
        pt(fw),
        pt(fh)
    )
}
