//! Scene Graph export: classify every palette-layer connected component
//! into the most specific editable primitive (circle > rect > line >
//! polygon > path). The scene JSON is the project's intermediate contract
//! (see schema/SCENE_GRAPH.md); text elements come from the OCR layer,
//! never from here.

use crate::cc::Labels;
use crate::geom;
use crate::img::Img;
use crate::mask::Mask;
use crate::svg;
use serde_json::{json, Map, Value};

fn hex(c: [f32; 3]) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        c[0].round().clamp(0.0, 255.0) as i32,
        c[1].round().clamp(0.0, 255.0) as i32,
        c[2].round().clamp(0.0, 255.0) as i32
    )
}

/// Classify one connected component (crop of the layer mask at its bbox)
/// into a Scene element. `gid` is the global element counter; returns the
/// element and how many ids were consumed (path elements stay one id).
pub fn classify_cc(
    crop: &Mask,
    gx: i64,
    gy: i64,
    fill: [f32; 3],
    st_index: usize,
) -> Value {
    let (w, h) = (crop.w as f64, crop.h as f64);
    let area = crop.count() as f64;
    let mut el = Map::new();
    el.insert("fill".into(), json!(hex(fill)));
    el.insert("source".into(), json!("geometry"));
    el.insert("confidence".into(), json!(0.95));
    el.insert(
        "bbox".into(),
        json!([gx, gy, crop.w as i64, crop.h as i64]),
    );
    el.insert("stroke".into(), Value::Null);

    let contours = geom::external_contours(crop);
    if contours.is_empty() {
        el.insert("type".into(), json!("rect"));
        el.insert("x".into(), json!(gx));
        el.insert("y".into(), json!(gy));
        el.insert("w".into(), json!(crop.w as i64));
        el.insert("h".into(), json!(crop.h as i64));
        return Value::Object(el);
    }
    let contour = &contours[0];
    let per = geom::arc_length(contour, true).max(1.0);
    let ca = geom::contour_area(contour).max(1.0);
    let circ = 4.0 * std::f64::consts::PI * ca / (per * per);

    // circle: high circularity and near-square bbox
    let dbg = std::env::var("FIGURESVG_DEBUG").is_ok();
    if dbg {
        let ap = geom::approx_poly_dp(contour, (0.004 * per).max(1.0), true);
        eprintln!(
            "branches: circ={circ:.2} fillr={:.3} approx={} area={} w={w} h={h}",
            geom::contour_area(contour) / (w * h),
            ap.len(),
            area,
        );
    }
    let aspect = w.max(h) / w.min(h).max(1.0);

    // ellipse: round-ish but elongated (aspect >= 1.3:1). Flower petals,
    // anime eyes, pet bodies are ellipses, not circles.
    if circ >= 0.55 && aspect >= 1.3 {
        let (cx, cy) = (
            gx + crop.w as i64 / 2,
            gy + crop.h as i64 / 2,
        );
        el.insert("type".into(), json!("ellipse"));
        el.insert("cx".into(), json!(cx));
        el.insert("cy".into(), json!(cy));
        el.insert("rx".into(), json!((crop.w as f64 / 2.0).round() as i64));
        el.insert("ry".into(), json!((crop.h as f64 / 2.0).round() as i64));
        return Value::Object(el);
    }

    // circle: high circularity and near-square bbox
    if circ >= 0.75 && aspect < 1.3 {
        let (cx, cy) = (
            gx + crop.w as i64 / 2,
            gy + crop.h as i64 / 2,
        );
        el.insert("type".into(), json!("circle"));
        el.insert("cx".into(), json!(cx));
        el.insert("cy".into(), json!(cy));
        el.insert("r".into(), json!(((area / std::f64::consts::PI).sqrt().round()) as i64));
        return Value::Object(el);
    }

    // thin elongated run: an axis-aligned line segment
    if w.min(h) <= 3.0 && w.max(h) >= 8.0 {
        el.insert("type".into(), json!("line"));
        if w >= h {
            el.insert("x1".into(), json!(gx));
            el.insert("y1".into(), json!(gy + crop.h as i64 / 2));
            el.insert("x2".into(), json!(gx + crop.w as i64 - 1));
            el.insert("y2".into(), json!(gy + crop.h as i64 / 2));
        } else {
            el.insert("x1".into(), json!(gx + crop.w as i64 / 2));
            el.insert("y1".into(), json!(gy));
            el.insert("x2".into(), json!(gx + crop.w as i64 / 2));
            el.insert("y2".into(), json!(gy + crop.h as i64 - 1));
        }
        return Value::Object(el);
    }

    let approx = geom::approx_poly_dp(contour, (0.004 * per).max(1.0), true);
    let bbox_area = w * h;
    // rect/polygon-ness is how much of the bbox the MASK FILLS (pixel
    // count), not the contour-enclosed area: a frame's outer contour
    // encloses the whole bbox (ratio ~1) while the mask fills ~0.7% of it
    let fill_ratio = area / bbox_area;

    // rectangle: 4-5 corners. Interior holes (text drawn on the shape)
    // lower the pixel fill ratio without changing the rect nature -- a
    // rect with 4 corners stays a rect down to 0.55 fill; the OCR text
    // layer re-draws the glyphs on top.
    if approx.len() <= 5 && fill_ratio >= 0.55 {
        el.insert("type".into(), json!("rect"));
        el.insert("x".into(), json!(gx));
        el.insert("y".into(), json!(gy));
        el.insert("w".into(), json!(crop.w as i64));
        el.insert("h".into(), json!(crop.h as i64));
        return Value::Object(el);
    }

    // polygon: few vertices, decent fill (holes tolerated similarly)
    if approx.len() <= 14 && fill_ratio >= 0.55 {
        let pts: Vec<String> = approx
            .iter()
            .map(|&(x, y)| format!("{},{}", x + gx as i32, y + gy as i32))
            .collect();
        el.insert("type".into(), json!("polygon"));
        el.insert("points".into(), json!(pts.join(" ")));
        el.insert("closed".into(), json!(true));
        return Value::Object(el);
    }

    // path: trace this component alone (complex outline)
    let paths = svg::trace_black(crop, crop.w, crop.h).unwrap_or_default();
    let d: Vec<String> = paths
        .iter()
        .filter_map(|p| extract_d(p))
        .filter(|s| !s.trim().is_empty())
        .collect();
    if !d.is_empty() {
        el.insert("type".into(), json!("path"));
        el.insert("d".into(), json!(d.join(" ")));
        el.insert("transform".into(), json!(format!("translate({gx},{gy})")));
        return Value::Object(el);
    }
    // stroke-outline fallback: sparse components (frames, grids, connectors)
    // must NEVER become a filled bbox rect -- that paints whole regions
    // solid. Their external contour as a stroked outline is semantically
    // right and visually close. Degenerate contours (a 2-pixel blob whose
    // trace oscillates up to the step cap, producing 1e8-point paths and
    // 100s of MB of d-data) fall back to a bbox rect.
    if contour.len() > 2000 {
        el.insert("type".into(), json!("rect"));
        el.insert("x".into(), json!(gx));
        el.insert("y".into(), json!(gy));
        el.insert("w".into(), json!(crop.w as i64));
        el.insert("h".into(), json!(crop.h as i64));
        return Value::Object(el);
    }
    let pts: Vec<String> = contour
        .iter()
        .map(|&(x, y)| format!("{},{}", x + gx as i32, y + gy as i32))
        .collect();
    let mut d = String::from("M ");
    d.push_str(&pts.join(" L "));
    d.push_str(" Z");
    el.insert("type".into(), json!("path"));
    el.insert("d".into(), json!(d));
    el.insert("fill".into(), json!("none"));
    el.insert("stroke".into(), json!(hex(fill)));
    el.insert("stroke_width".into(), json!(2.0));
    Value::Object(el)
}

fn extract_d(path_el: &str) -> Option<String> {
    let start = path_el.find("d=\"")? + 3;
    let rest = &path_el[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Build the full Scene document from labeled kept components.
pub fn build_scene(
    img: &Img,
    lab: &Labels,
    keep_label: &[i32], // labels to include (post area filter), per CC
    layer_color: &std::collections::HashMap<i32, [f32; 3]>,
    stats_of: &std::collections::HashMap<i32, (i64, i64)>, // label -> (gx, gy)
    crops: &std::collections::HashMap<i32, Mask>,
) -> Value {
    let mut elements = Vec::new();
    let mut n = 0usize;
    // stable order: by area descending (largest first, like painting order)
    let mut labels: Vec<i32> = keep_label.to_vec();
    labels.sort_by_key(|&l| -lab.stats[l.max(0) as usize].area);
    for l in labels {
        let color = match layer_color.get(&l) {
            Some(c) => *c,
            None => continue,
        };
        let (gx, gy) = stats_of[&l];
        let crop = &crops[&l];
        let mut el = classify_cc(crop, gx, gy, color, n);
        n += 1;
        if let Value::Object(m) = &mut el {
            m.insert("id".into(), json!(format!("shape_{n:03}")));
        }
        elements.push(el);
    }
    json!({
        "schema": "figuresvg/scene@1",
        "canvas": { "width": img.w, "height": img.h },
        "background": "#ffffff",
        "elements": elements,
        "relations": [],
        "notes": { "engine": "palette@1", "out_of_scope": false }
    })
}
