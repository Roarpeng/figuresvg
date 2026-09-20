//! Black-layer binary tracing (native vtracer crate) and SVG rendering
//! for verification (resvg + tiny-skia, replaces cairosvg).

use crate::mask::Mask;
use visioncortex::{ColorImage, PathSimplifyMode};
use vtracer::{ColorMode, Config, Hierarchical};

/// Trace the binary black mask into `<path>` element strings (binary
/// colormode, spline mode -- same converter settings as the python side).
pub fn trace_black(clean: &Mask, w: usize, h: usize) -> Result<Vec<String>, String> {
    let mut pixels = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let v = if clean.bits[i] != 0 { 0u8 } else { 255u8 };
        pixels[i * 4] = v;
        pixels[i * 4 + 1] = v;
        pixels[i * 4 + 2] = v;
        pixels[i * 4 + 3] = 255;
    }
    let img = ColorImage { pixels, width: w, height: h };
    let config = Config {
        color_mode: ColorMode::Binary,
        hierarchical: Hierarchical::Stacked,
        filter_speckle: 1,
        color_precision: 8,
        layer_difference: 16,
        mode: PathSimplifyMode::Spline,
        corner_threshold: 60,
        length_threshold: 4.0,
        max_iterations: 10,
        splice_threshold: 45,
        path_precision: Some(1),
    };
    let svg = vtracer::convert(img, config)?;
    Ok(extract_path_elements(&svg.to_string()))
}

/// All `<path ... />` element strings of an SVG document.
pub fn extract_path_elements(svg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(rel) = svg[i..].find("<path") {
        let start = i + rel;
        let rest = &svg[start..];
        if let Some(endrel) = rest.find("/>") {
            // strip any fill attribute: the caller's <g fill=...> owns color
            let el = rest[..endrel + 2].to_string();
            let cleaned = strip_fill(&el);
            out.push(cleaned);
            i = start + endrel + 2;
        } else {
            break;
        }
    }
    out
}

fn strip_fill(el: &str) -> String {
    // remove fill="..." attributes so the wrapping group's fill applies
    let mut out = String::with_capacity(el.len());
    let b = el.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if el[i..].starts_with("fill=\"") {
            if let Some(qrel) = el[i + 6..].find('"') {
                i = i + 6 + qrel + 1;
                continue;
            }
        }
        let ch = el[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Render an SVG document to a WxH PNG (resvg, pure Rust).
pub fn render_svg_to_png(svg_str: &str, w: usize, h: usize, out_path: &str) -> Result<(), String> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(svg_str, &opts).map_err(|e| format!("svg parse: {e}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w as u32, h as u32)
        .ok_or_else(|| "pixmap alloc failed".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let png = pixmap.encode_png().map_err(|e| format!("png encode: {e}"))?;
    std::fs::write(out_path, png).map_err(|e| format!("write {out_path}: {e}"))
}
