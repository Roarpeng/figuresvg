//! figure2svg -- layered raster-figure -> SVG converter (Rust port of
//! scripts/figure2svg.py). Same CLI contract:
//!   figure2svg analyze INPUT [--proposal spec.json]
//!   figure2svg build INPUT --spec spec.json --out out.svg
//!           [--render check.png] [--strict]

mod cc;
mod geom;
mod img;
mod mask;
mod masks;
mod palquant;
mod rowbanded;
mod objects;
mod scene;
mod segment;
mod spec;
mod stats;
mod svg;
mod verify;

use img::Img;
use spec::Spec;
use std::process::exit;

/// near-black palette entries are TEXT: painted last (on top) so dilated
/// color layers can never cover glyphs.
fn is_text_color(c: [f32; 3]) -> i32 {
    (c[0] < 90.0 && c[1] < 90.0 && c[2] < 90.0) as i32
}

fn rgb_hex(c: [f32; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}",
        c[0].round().clamp(0.0, 255.0) as i32,
        c[1].round().clamp(0.0, 255.0) as i32,
        c[2].round().clamp(0.0, 255.0) as i32)
}

fn usage() -> ! {
    eprintln!(
        "usage: figure2svg analyze INPUT [--proposal spec.json]\n\
         \x20      figure2svg build INPUT --spec spec.json --out out.svg [--render check.png] [--strict]"
    );
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        usage();
    }
    let cmd = args[1].as_str();
    let input = args[2].clone();
    let mut rest = &args[3..];
    match cmd {
        "analyze" => {
            let mut proposal: Option<String> = None;
            while !rest.is_empty() {
                match rest[0].as_str() {
                    "--proposal" => {
                        proposal = Some(rest.get(1).expect("--proposal needs a value").clone());
                        rest = &rest[2..];
                    }
                    _ => usage(),
                }
            }
            cmd_analyze(&input, proposal.as_deref());
        }
        "build" => {
            let mut spec_path: Option<String> = None;
            let mut out = "out.svg".to_string();
            let mut render: Option<String> = None;
            let mut strict = false;
            let mut scene_out: Option<String> = None;
            while !rest.is_empty() {
                if rest[0] == "--scene" {
                    scene_out = Some(rest.get(1).expect("--scene needs a value").clone());
                    rest = &rest[2..];
                    continue;
                }
                match rest[0].as_str() {
                    "--spec" => {
                        spec_path = Some(rest.get(1).expect("--spec needs a value").clone());
                        rest = &rest[2..];
                    }
                    "--out" => {
                        out = rest.get(1).expect("--out needs a value").clone();
                        rest = &rest[2..];
                    }
                    "--render" => {
                        render = Some(rest.get(1).expect("--render needs a value").clone());
                        rest = &rest[2..];
                    }
                    "--strict" => {
                        strict = true;
                        rest = &rest[1..];
                    }
                    _ => usage(),
                }
            }
            let spec_path = match spec_path {
                Some(s) => s,
                None => usage(),
            };
                        cmd_build(&input, &spec_path, &out, render.as_deref(), strict, scene_out.as_deref());
        }
        _ => usage(),
    }
}

fn cmd_analyze(input: &str, proposal: Option<&str>) {
    let a = match Img::load(input) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    };
    let mut sp = Spec {
        spec_version: 2,
        canvas: [a.w as i64, a.h as i64],
        layout: String::new(),
        params: Default::default(),
        recolor: vec![],
        rows: vec![],
        out_of_scope: false,
        force: false,
    };
    // complexity note (NOT a refusal): when colors are spread over many
    // quantization bins with low top-30 coverage, the input is either a
    // photo/hand-drawn artwork (out of scope) or a dense dot plot (in
    // scope, converts fine) -- the two are not reliably separable by this
    // statistic, so warn only and let verification judge the result.
    {
        use std::collections::HashMap;
        let mut bins: HashMap<u32, usize> = HashMap::new();
        let mut total = 0usize;
        for i in 0..a.w * a.h {
            let c = [a.d[i * 3] as u32, a.d[i * 3 + 1] as u32, a.d[i * 3 + 2] as u32];
            if c[0] > 244 && c[1] > 244 && c[2] > 244 {
                continue;
            }
            *bins.entry((c[0] / 8) << 12 | (c[1] / 8) << 6 | (c[2] / 8)).or_insert(0) += 1;
            total += 1;
        }
        if total > 0 {
            let mut counts: Vec<usize> = bins.values().copied().collect();
            counts.sort_unstable_by(|x, y| y.cmp(x));
            let top30: usize = counts.iter().take(30).sum();
            let n_bins = counts.len();
            if n_bins > 1000 && (top30 as f64 / total as f64) < 0.65 {
                sp.out_of_scope = true; // recorded in spec as a note only
                println!("NOTE: dense/complex color structure ({n_bins} bins, top-30 coverage \
{:.0}%) -- typical of photos/artwork (out of scope) or dense dot plots (fine). \
Output quality will be judged by the verification checks.",
                    100.0 * top30 as f64 / total as f64);
            }
        }
    }
    let rb = rowbanded::detect_rowbanded(&a, &sp);
    if let Some(det) = rb.filter(|d| d.rows.len() >= 4) {
        sp.layout = "rowbanded".into();
        sp.rows = rowbanded::row_specs_from_detect(&a, &det);
        println!(
            "layout: rowbanded ({} rows, zone x {}-{})",
            sp.rows.len(),
            det.zone.0,
            det.zone.1
        );
        let exts: Vec<String> = sp.rows.iter().map(|r| format!("({},{})", r.top, r.bottom)).collect();
        println!("row y-extents: [{}]", exts.join(", "));
    } else {
        sp.layout = "generic".into();
        println!("layout: generic (regions + markers + black via layered engine)");
    }
    println!("params (defaults, override in spec['params']):");
    for (k, v) in spec::PARAMS_DEFAULTS {
        println!("  {k} = {v}");
    }
    if let Some(p) = proposal {
        std::fs::write(p, sp.to_json()).unwrap_or_else(|e| eprintln!("cannot write {p}: {e}"));
        println!("proposal written to {p}");
    }
}

fn cmd_build(input: &str, spec_path: &str, out_path: &str, render: Option<&str>, strict: bool, scene_out: Option<&str>) {
    let spec_str = std::fs::read_to_string(spec_path).unwrap_or_else(|e| {
        eprintln!("cannot read {spec_path}: {e}");
        exit(1);
    });
    let spec = Spec::from_json(&spec_str).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    if spec.out_of_scope && !spec.force {
        eprintln!("note: spec flags dense/complex color structure; proceeding -- \
the verification checks will decide pass/fail.");
    }
    let orig = Img::load(input).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    let (w, h) = (spec.canvas[0] as usize, spec.canvas[1] as usize);
    if w != orig.w || h != orig.h {
        eprintln!(
            "warning: spec canvas {w}x{h} != image {}x{} (continuing with spec canvas)",
            orig.w, orig.h
        );
    }
    let (cleaned, _noise) = masks::clean_noise(&orig, &spec);

    let mut svg_parts: Vec<String> = vec![
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#
        ),
        format!(r##"<rect width="{w}" height="{h}" fill="#ffffff"/>"##),
    ];

    let repainted: Img;
    let mut content: mask::Mask;
    let mut black_clean_for_verify: mask::Mask;
    let mut cc_src: usize;
    let engine = spec.p("engine"); // 0 = palette layers (default), 1 = legacy estimator
    if spec.layout == "rowbanded" && engine != 0.0 {
        let zone = rowbanded::detect_rowbanded(&cleaned, &spec)
            .map(|d| d.zone)
            .unwrap_or((0, w as i32));
        let rb = rowbanded::build_rowbanded(&cleaned, &orig, &spec, &spec.rows, zone, &spec.recolor);
        svg_parts.push("<g>".into());
        svg_parts.extend(rb.rects);
        svg_parts.push("</g><g>".into());
        for &(mx, my, mw, mh, _area) in &rb.markers {
            let (bx, by, bw, bh) = (mx as usize, my as usize, mw as usize, mh as usize);
            let x1 = (bx + bw).min(orig.w);
            let y1 = (by + bh).min(orig.h);
            if bx >= x1 || by >= y1 {
                continue;
            }
            // comp = spread > 60 within the marker bbox, on the ORIGINAL image
            let mut cmask = mask::Mask::new(x1 - bx, y1 - by);
            for yy in by..y1 {
                for xx in bx..x1 {
                    let c = orig.at(xx, yy);
                    let spd = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
                    if spd > 60.0 {
                        cmask.bits[(yy - by) * (x1 - bx) + (xx - bx)] = 1;
                    }
                }
            }
            let (els, _) = geom::marker_geometry(&orig, &cmask, by as i32, bx as i32, spec.p("circularity_min"));
            svg_parts.extend(els);
        }
        svg_parts.push("</g>".into());
        content = masks::tinted_mask(&rb.repainted);
        let dm = masks::dark_mask(&cleaned, &spec);
        for i in 0..content.bits.len() {
            content.bits[i] = (content.bits[i] != 0 || dm.bits[i] != 0) as u8;
        }
        repainted = rb.repainted;
        let (bl, cc_n, dropped) = segment::black_layer(&cleaned, &spec, None);
        let paths = svg::trace_black(&bl, w, h).unwrap_or_else(|e| {
            eprintln!("vtracer failed: {e}");
            exit(1);
        });
        println!("black: {cc_n} components kept, {dropped} specks dropped, {} paths", paths.len());
        for i in 0..content.bits.len() {
            content.bits[i] = (content.bits[i] != 0 || bl.bits[i] != 0) as u8;
        }
        svg_parts.push(r##"<g fill="#000000">"##.into());
        svg_parts.extend(paths);
        svg_parts.push("</g>".into());
        black_clean_for_verify = bl;
        cc_src = cc_n;
    } else {
        // palette-layers strategy: exact palette quantization, one binary-
        // traced layer per color. No fill color is ever estimated.
        // Recolor (rowbanded donor repaint) is a PREPROCESSING step so the
        // exact-color engine inherits targeted recolor for free.
        let mut base = cleaned.clone();
        if !spec.recolor.is_empty() && !spec.rows.is_empty() {
            let zone = rowbanded::detect_rowbanded(&cleaned, &spec)
                .map(|d| d.zone)
                .unwrap_or((0, w as i32));
            let rb = rowbanded::build_rowbanded(
                &cleaned, &orig, &spec, &spec.rows, zone, &spec.recolor,
            );
            base = rb.repainted;
            println!("recolor: {} row(s) repainted with donor gradients", spec.recolor.len());
        }
        let pal = palquant::quantize(&base, &spec);
        let use_objects = spec.p("objects") != 0.0;
        println!("palette: {} colors (step {})", pal.colors.len(), spec.p("palette_step") as i32);
        let mut layer_elems: Vec<String> = Vec::new();
        let mut black_union = mask::Mask::new(w, h);
        content = mask::Mask::new(w, h);
        let min_sub = spec.p("layer_min_subarea") as i64;
        // paint smallest layers first: larger layers then paint OVER the
        // seams and 1px bleed of smaller ones, so no white can show at
        // internal boundaries (bleed only remains at outer edges, which the
        // verification metrics exclude)
        let mut scene_crops: std::collections::HashMap<i32, mask::Mask> =
            std::collections::HashMap::new();
        let mut scene_stats: std::collections::HashMap<i32, (i64, i64)> =
            std::collections::HashMap::new();
        let mut scene_colors: std::collections::HashMap<i32, [f32; 3]> =
            std::collections::HashMap::new();
        let mut scene_labels: Vec<i32> = Vec::new();
        let mut order: Vec<usize> = (0..pal.colors.len()).collect();
        order.sort_by_key(|&i| (is_text_color(pal.colors[i]), pal.counts[i]));
        // all content pixels (any layer): seam-targeted growth may only
        // cover pixels that belong to SOME layer, never true white bg
        let mut content_map = mask::Mask::new(w, h);
        for i in 0..pal.assign.len() {
            if pal.assign[i] != usize::MAX {
                content_map.bits[i] = 1;
            }
        }
        for idx in order {
            let col = pal.colors[idx];
            let mut m = pal.layer_mask(idx, w, h);
            // seam-targeted 1px growth: adjacent quantization bins of one
            // ramp leave 1px slivers after tracing; growing INTO other
            // layers' pixels (never into white) lets neighbors overlap the
            // seam with a valid ramp color on both sides
            if is_text_color(col) == 0 {
                let grown = mask::dilate(&m, 1);
                for i in 0..m.bits.len() {
                    if grown.bits[i] != 0 && content_map.bits[i] != 0 && m.bits[i] == 0 {
                        m.bits[i] = 1;
                    }
                }
            }
            // drop tiny stray components (JPEG specks); keep glyph-sized CCs
            let lab = cc::connected_components(&m);
            let mut keep = mask::Mask::new(w, h);
            let mut kept_cc = 0usize;
            let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0usize, 0usize);
            for j in 1..=lab.count {
                if lab.stats[j].area >= min_sub {
                    kept_cc += 1;
                    let st = &lab.stats[j];
                    let (x0, y0) = (st.x.max(0) as usize, st.y.max(0) as usize);
                    let (x1, y1) = ((st.x + st.w) as usize, (st.y + st.h) as usize);
                    bx0 = bx0.min(x0);
                    by0 = by0.min(y0);
                    bx1 = bx1.max(x1.min(w));
                    by1 = by1.max(y1.min(h));
                    for y in y0..y1.min(h) {
                        for x in x0..x1.min(w) {
                            if lab.lab[y * w + x] == j as i32 {
                                keep.bits[y * w + x] = 1;
                            }
                        }
                    }
                }
            }
            if kept_cc == 0 {
                continue;
            }
            if scene_out.is_some() && use_objects {
                // object mode (v2): consolidation happens after the layer
                // loop, over the full palette assignment -- skip per-layer
                continue;
            }
            if scene_out.is_some() {
                // legacy per-CC scene mode: classify each kept CC
                for j in 1..=lab.count {
                    if lab.stats[j].area < min_sub {
                        continue;
                    }
                    let st = &lab.stats[j];
                    let (cx0, cy0) = (st.x.max(0) as usize, st.y.max(0) as usize);
                    let (cx1, cy1) =
                        ((st.x + st.w) as usize, (st.y + st.h) as usize);
                    let (cwj, chj) = (cx1.min(w) - cx0, cy1.min(h) - cy0);
                    let mut cmask = mask::Mask::new(cwj, chj);
                    for y in 0..chj {
                        for x in 0..cwj {
                            if lab.lab[(y + cy0) * w + (x + cx0)] == j as i32 {
                                cmask.bits[y * cwj + x] = 1;
                            }
                        }
                    }
                    // label ids are per-layer; make them globally unique
                    let gid = idx as i64 * 4_000_000 + j as i64;
                    scene_crops.insert(gid as i32, cmask);
                    scene_stats.insert(gid as i32, (cx0 as i64, cy0 as i64));
                    scene_colors.insert(gid as i32, col);
                    scene_labels.push(gid as i32);
                }
                continue;
            }
            // trace only the layer's bbox: vtracer cost scales with area and
            // most layers occupy a small fraction of the canvas
            let (cw, ch) = (bx1 - bx0, by1 - by0);
            let mut crop = mask::Mask::new(cw, ch);
            for y in 0..ch {
                for x in 0..cw {
                    crop.bits[y * cw + x] = keep.bits[(y + by0) * w + (x + bx0)];
                }
            }
            let paths = svg::trace_black(&crop, cw, ch).unwrap_or_else(|e| {
                eprintln!("vtracer failed on layer {idx}: {e}");
                exit(1);
            });
            if paths.is_empty() {
                continue;
            }
            layer_elems.push(format!(
                "<g fill=\"{}\" transform=\"translate({},{})\">",
                palquant::rgbf(col),
                bx0,
                by0
            ));
            layer_elems.extend(paths);
            layer_elems.push("</g>".into());
            // near-black layers double as the glyph mask for verification
            if col[0] < 90.0 && col[1] < 90.0 && col[2] < 90.0 {
                for i in 0..black_union.bits.len() {
                    black_union.bits[i] |= keep.bits[i];
                }
            }
            for i in 0..content.bits.len() {
                content.bits[i] |= keep.bits[i];
            }
        }
        if let Some(sp) = scene_out {
            if use_objects {
                let objs = objects::consolidate(&base, &pal);
                let mut elements = Vec::new();
                let mut sorted: Vec<&objects::Object> = objs.iter().collect();
                sorted.sort_by_key(|o| std::cmp::Reverse(o.mask.count()));
                let mut defs: Vec<serde_json::Value> = Vec::new();
                let mut emitted_count = 0;
                for (n, o) in sorted.iter().enumerate() {
                    let (mut ox, mut oy, mut ow, mut oh) = o.bbox;
                    if ow == 0 || oh == 0 {
                        continue;
                    }
                    // bbox may exceed the canvas after the +1px growth;
                    // clamp before cropping
                    if ox + ow > w {
                        ow = w - ox;
                    }
                    if oy + oh > h {
                        oh = h - oy;
                    }
                    if ox >= w || oy >= h || ow == 0 || oh == 0 {
                        continue;
                    }
                    // o.mask is already CROPPED to the object bbox --
                    // it IS the crop; use it directly (the old full-canvas
                    // indexing panicked since the mask is now bbox-sized)
                    let crop = &o.mask;
                    let el = if o.grid_cell {
                        serde_json::json!({
                            "id": format!("shape_{:03}", n + 1),
                            "type": "rect",
                            "bbox": [ox, oy, ow, oh],
                            "x": ox, "y": oy, "w": ow, "h": oh,
                            "fill": rgb_hex(o.fill),
                            "source": "grid-cell",
                            "confidence": 0.98,
                        })
                    } else {
                        scene::classify_cc(&crop, ox as i64, oy as i64, o.fill, n)
                    };
                    let mut el = el;
                    if let Some(g) = &o.gradient {
                        let gid = format!("grad_{:03}", n + 1);
                        if let serde_json::Value::Object(m) = &mut el {
                            m.insert("gradient".into(), serde_json::json!({
                                "id": gid, "axis": g.axis.to_string(),
                                "from": rgb_hex(g.from), "to": rgb_hex(g.to),
                            }));
                        }
                        defs.push(serde_json::json!({
                            "id": gid, "axis": g.axis.to_string(),
                            "from": rgb_hex(g.from), "to": rgb_hex(g.to),
                        }));
                    }
                    if let serde_json::Value::Object(m) = &mut el {
                        m.insert("id".into(), serde_json::json!(format!("shape_{:03}", n + 1)));
                    }
                    elements.push(el);
                    emitted_count += 1;
                }
                eprintln!("[emission] {} of {} objects emitted", emitted_count, sorted.len());
                let scene = serde_json::json!({
                    "schema": "figuresvg/scene@2",
                    "canvas": { "width": w, "height": h },
                    "background": "#ffffff",
                    "defs": defs,
                    "elements": elements,
                    "relations": [],
                    "notes": { "engine": "objects@2" }
                });
                std::fs::write(sp, serde_json::to_string_pretty(&scene).unwrap())
                    .unwrap_or_else(|e| { eprintln!("cannot write scene: {e}"); exit(1); });
                println!("wrote {sp} ({} objects, {} gradients)", elements.len(), defs.len());
                return;
            }
            let mut elements = Vec::new();
            let mut labels_sorted = scene_labels.clone();
            labels_sorted.sort_by_key(|&l| {
                -(scene_crops.get(&l).map(|m| m.count()).unwrap_or(0) as i64)
            });
            for (n, l) in labels_sorted.iter().enumerate() {
                let crop = &scene_crops[l];
                let (gx, gy) = scene_stats[l];
                let color = scene_colors[l];
                let mut el = scene::classify_cc(crop, gx, gy, color, n);
                if let serde_json::Value::Object(m) = &mut el {
                    m.insert("id".into(), serde_json::json!(format!("shape_{:03}", n + 1)));
                }
                elements.push(el);
            }
            let scene = serde_json::json!({
                "schema": "figuresvg/scene@1",
                "canvas": { "width": w, "height": h },
                "background": "#ffffff",
                "elements": elements,
                "relations": [],
                "notes": { "engine": "palette@1" }
            });
            std::fs::write(sp, serde_json::to_string_pretty(&scene).unwrap())
                .unwrap_or_else(|e| { eprintln!("cannot write scene: {e}"); exit(1); });
            println!("wrote {sp} ({} elements)", elements.len());
            return;
        }
        svg_parts.push("<g>".into());
        svg_parts.extend(layer_elems);
        svg_parts.push("</g>".into());
        // verification inputs: black layer = union of near-black palette layers
        let lab = cc::connected_components(&black_union);
        black_clean_for_verify = black_union;
        cc_src = lab.count;
        repainted = base;
    }

    svg_parts.push("</svg>".into());
    let svg_str = svg_parts.join("\n");
    std::fs::write(out_path, &svg_str).unwrap_or_else(|e| {
        eprintln!("cannot write {out_path}: {e}");
        exit(1);
    });
    println!("wrote {out_path} ({:.2} MB)", svg_str.len() as f64 / 1e6);

    repainted.save_png("_repainted.png").ok();

    if let Some(rp) = render {
        if let Err(e) = svg::render_svg_to_png(&svg_str, w, h, rp) {
            eprintln!("render failed: {e}");
            exit(1);
        }
        let results = verify::verify(rp, &repainted, &content, cc_src, Some(&black_clean_for_verify))
            .unwrap_or_else(|e| {
                eprintln!("verify failed: {e}");
                exit(1);
            });
        println!("\nverification:");
        let mut fails = 0usize;
        for (name, ok, detail) in &results {
            println!("  [{}] {}: {}", if *ok { "PASS" } else { "FAIL" }, name, detail);
            if !ok {
                fails += 1;
            }
        }
        // info: mean |diff| vs ORIGINAL incl. intentional edits
        let ren = Img::load(rp).unwrap_or_else(|e| {
            eprintln!("{e}");
            exit(1);
        });
        let mut omd = 0.0f64;
        for y in 0..ren.h.min(orig.h) {
            for x in 0..ren.w.min(orig.w) {
                let (rc, oc) = (ren.at(x, y), orig.at(x, y));
                omd += ((rc[0] - oc[0]).abs() + (rc[1] - oc[1]).abs() + (rc[2] - oc[2]).abs()) as f64;
            }
        }
        omd /= (ren.w.min(orig.w) * ren.h.min(orig.h) * 3) as f64;
        println!("  (info) mean |diff| vs ORIGINAL incl. intentional edits: {omd:.2}");
        if fails > 0 && strict {
            eprintln!("{fails} verification check(s) failed");
            exit(1);
        }
    }
}
