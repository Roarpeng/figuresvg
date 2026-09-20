//! Generic layered engine: component classification (marker / flat /
//! axial-gradient / multicolor-split), white glyph holes, black layer.

use crate::cc::{connected_components, fill_holes, merge_across_gaps};
use crate::geom::{comp_contours, emit_shapes, marker_geometry};
use crate::img::{rgbf, Img};
use crate::mask::{dilate, erode, Mask};
use crate::masks::{dark_mask, seg_mask};
use crate::spec::Spec;
use crate::stats::{dominant_color, Colors};
use std::collections::HashMap;

// ------------------------------------------------------ gradient analysis

/// Power iteration for the top eigenvector of a symmetric 3x3 matrix.
fn top_eigenvector(cov: &[[f64; 3]; 3]) -> [f64; 3] {
    let mut v = [1.0f64, 1.0, 1.0];
    let n0 = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v[0] /= n0;
    v[1] /= n0;
    v[2] /= n0;
    for _ in 0..128 {
        let mut nv = [
            cov[0][0] * v[0] + cov[0][1] * v[1] + cov[0][2] * v[2],
            cov[1][0] * v[0] + cov[1][1] * v[1] + cov[1][2] * v[2],
            cov[2][0] * v[0] + cov[2][1] * v[1] + cov[2][2] * v[2],
        ];
        let n = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
        if n < 1e-12 {
            break;
        }
        nv[0] /= n;
        nv[1] /= n;
        nv[2] /= n;
        if (nv[0] - v[0]).abs() + (nv[1] - v[1]).abs() + (nv[2] - v[2]).abs() < 1e-12 {
            v = nv;
            break;
        }
        v = nv;
    }
    v
}

fn pearson(xs: &[f64], ys: &[f64]) -> f64 {
    let n = xs.len() as f64;
    if n == 0.0 {
        return 0.0;
    }
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    for i in 0..xs.len() {
        sx += xs[i];
        sy += ys[i];
    }
    let (mx, my) = (sx / n, sy / n);
    let (mut sxy, mut sxx, mut syy) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..xs.len() {
        let (dx, dy) = (xs[i] - mx, ys[i] - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 1e-12 || syy <= 1e-12 {
        return 0.0;
    }
    sxy / (sxx.sqrt() * syy.sqrt())
}

/// (axis, r) where r = |corr(position, PC1-of-color)|, or (None, r).
/// `coords` are full-image pixel coordinates of the component's pixels.
pub fn axial_gradient(a: &Img, coords: &[(usize, usize)], spec: &Spec) -> (Option<char>, f64) {
    let n = coords.len();
    if n < 8 {
        return (None, 0.0);
    }
    let mut colors = Colors::with_capacity(n);
    let mut xs = Vec::with_capacity(n);
    let mut ys = Vec::with_capacity(n);
    for &(x, y) in coords {
        colors.push(a.at(x, y));
        xs.push(x as f64);
        ys.push(y as f64);
    }
    let mc = colors.mean();
    let mut cov = [[0.0f64; 3]; 3];
    let chans = [&colors.r, &colors.g, &colors.b];
    let cm = [mc[0] as f64, mc[1] as f64, mc[2] as f64];
    for i in 0..3 {
        for j in 0..3 {
            let mut s = 0.0;
            for k in 0..n {
                s += (chans[i][k] as f64 - cm[i]) * (chans[j][k] as f64 - cm[j]);
            }
            cov[i][j] = s / n as f64;
        }
    }
    let v = top_eigenvector(&cov);
    let mut c = vec![0.0f64; n];
    let mut mc2 = 0.0f64;
    for k in 0..n {
        c[k] = (chans[0][k] as f64 - cm[0]) * v[0]
            + (chans[1][k] as f64 - cm[1]) * v[1]
            + (chans[2][k] as f64 - cm[2]) * v[2];
        mc2 += c[k];
    }
    mc2 /= n as f64;
    let mut sc = 0.0f64;
    for k in 0..n {
        sc += (c[k] - mc2) * (c[k] - mc2);
    }
    let std = (sc / n as f64).sqrt();
    if std < 1e-6 {
        return (None, 0.0);
    }
    let rx = pearson(&xs, &c).abs();
    let ry = pearson(&ys, &c).abs();
    let thr = spec.p("gradient_axiality");
    if rx >= ry {
        if rx >= thr {
            (Some('x'), rx)
        } else {
            (None, rx)
        }
    } else if ry >= thr {
        (Some('y'), ry)
    } else {
        (None, ry)
    }
}

/// Split a gradient component into strip polygons along its axis.
pub fn gradient_strip_shapes(
    a: &Img,
    comp_mask: &Mask, // crop-local
    axis: char,
    y0: i32,
    x0: i32,
    spec: &Spec,
) -> Vec<String> {
    let ns = spec.p("gradient_strips") as usize;
    let coords: Vec<(usize, usize)> = comp_mask.pixels().collect();
    if coords.is_empty() {
        return vec![];
    }
    let pos: Vec<i64> = coords
        .iter()
        .map(|&(x, y)| if axis == 'x' { x as i64 } else { y as i64 })
        .collect();
    let lo = *pos.iter().min().unwrap() as f64;
    let hi = *pos.iter().max().unwrap() as f64 + 1.0;
    let (w, h) = (comp_mask.w, comp_mask.h);
    let mut out = Vec::new();
    for i in 0..ns {
        let e0 = lo + (hi - lo) * i as f64 / ns as f64;
        let e1 = lo + (hi - lo) * (i + 1) as f64 / ns as f64;
        let sel: Vec<usize> = (0..coords.len())
            .filter(|&k| (pos[k] as f64) >= e0 && (pos[k] as f64) < e1)
            .collect();
        if sel.len() < 6 {
            continue;
        }
        let mut strip = Mask::new(w, h);
        for &k in &sel {
            strip.bits[coords[k].1 * w + coords[k].0] = 1;
        }
        let mut cols = Colors::with_capacity(sel.len());
        for &k in &sel {
            let fx = (coords[k].0 as i32 + x0) as usize;
            let fy = (coords[k].1 as i32 + y0) as usize;
            if fx < a.w && fy < a.h {
                cols.push(a.at(fx, fy));
            }
        }
        let col = cols.median3();
        out.extend(emit_shapes(&comp_contours(&strip, 1.0), &rgbf(col), y0, x0));
    }
    out
}

// -------------------------------------------------- multicolor splitting

/// Top-k color modes of a pixel set (python dominant_color_clusters).
pub fn dominant_color_clusters(colors: &Colors) -> (Vec<[f32; 3]>, Vec<i32>, Vec<f32>) {
    let n = colors.len();
    let coarse: i64 = 6;
    let min_count = (30.0f64).max(n as f64 * 0.004) as i64;
    // coarse histogram: key -> (count, channel sums)
    let mut hist: HashMap<i64, (i64, [f64; 3])> = HashMap::new();
    for i in 0..n {
        let q = [
            (colors.r[i] / (256.0 / coarse as f32)).floor() as i64,
            (colors.g[i] / (256.0 / coarse as f32)).floor() as i64,
            (colors.b[i] / (256.0 / coarse as f32)).floor() as i64,
        ];
        let key = q[0] * coarse * coarse + q[1] * coarse + q[2];
        let e = hist.entry(key).or_insert((0, [0.0; 3]));
        e.0 += 1;
        e.1[0] += colors.r[i] as f64;
        e.1[1] += colors.g[i] as f64;
        e.1[2] += colors.b[i] as f64;
    }
    let mut entries: Vec<(i64, i64, [f64; 3])> =
        hist.into_iter().map(|(k, (c, s))| (k, c, s)).collect();
    // order by count desc, key asc (numpy unique + argsort(-counts) semantics)
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut centers: Vec<[f32; 3]> = Vec::new();
    for (_, cnt, sum) in entries {
        if cnt < min_count {
            continue;
        }
        let c = [
            (sum[0] / cnt as f64) as f32,
            (sum[1] / cnt as f64) as f32,
            (sum[2] / cnt as f64) as f32,
        ];
        if centers
            .iter()
            .all(|e| (e[0] - c[0]).abs().max((e[1] - c[1]).abs()).max((e[2] - c[2]).abs()) > 30.0)
        {
            centers.push(c);
        }
        if centers.len() >= 16 {
            break;
        }
    }
    if centers.is_empty() {
        centers.push(colors.median3());
    }
    let mut assign = vec![0i32; n];
    let mut mind = vec![0.0f32; n];
    for i in 0..n {
        let c = colors.at(i);
        let mut best = 0usize;
        let mut bd = f32::MAX;
        for (ci, e) in centers.iter().enumerate() {
            let d = (e[0] - c[0]).abs().max((e[1] - c[1]).abs()).max((e[2] - c[2]).abs());
            if d < bd {
                bd = d;
                best = ci;
            }
        }
        assign[i] = best as i32;
        mind[i] = bd;
    }
    (centers, assign, mind)
}

/// A component with mixed colors and no single gradient axis: split by
/// dominant color clusters, emit flat polygons per cluster.
pub fn split_multicolor(a: &Img, comp_mask: &Mask, y0: i32, x0: i32, spec: &Spec) -> Vec<String> {
    let coords: Vec<(usize, usize)> = comp_mask.pixels().collect();
    let mut colors = Colors::with_capacity(coords.len());
    for &(x, y) in &coords {
        let (fx, fy) = ((x as i32 + x0) as usize, (y as i32 + y0) as usize);
        if fx < a.w && fy < a.h {
            colors.push(a.at(fx, fy));
        }
    }
    let (centers, assign0, mind) = dominant_color_clusters(&colors);
    // white pseudo-cluster for pixels far from every center and near-white
    let mut assign = assign0;
    for i in 0..coords.len() {
        if mind[i] > 60.0 && colors.max_chan(i) > 190.0 {
            assign[i] = -1;
        }
    }
    let (w, h) = (comp_mask.w, comp_mask.h);
    let mut emitted: Vec<(i64, Vec<String>, i32)> = Vec::new();
    let mut cluster_ids: Vec<i32> = (0..centers.len() as i32).collect();
    cluster_ids.push(-1);
    for ci in cluster_ids {
        let sel: Vec<usize> = (0..coords.len()).filter(|&i| assign[i] == ci).collect();
        if sel.len() < 60 {
            continue;
        }
        let mut m = Mask::new(w, h);
        for &k in &sel {
            m.bits[coords[k].1 * w + coords[k].0] = 1;
        }
        // open (3x3) removes 1px JPEG serrations along cluster boundaries
        let m = dilate(&erode(&m, 1), 1);
        let sub = connected_components(&m);
        for si in 1..=sub.count {
            let st = sub.stats[si];
            let (jx, jy, jw, jh, jarea) = (st.x as usize, st.y as usize, st.w as usize, st.h as usize, st.area);
            if ci == -1 {
                if (jarea as f64) < spec.p("black_min_component_area") {
                    continue; // white glyphs only; specks skipped
                }
            } else if jarea < 60 {
                continue;
            }
            let mut sm = Mask::new(jw, jh);
            let mut sub_colors = Colors::with_capacity(jarea as usize);
            for yy in 0..jh {
                for xx in 0..jw {
                    if sub.lab[(jy + yy) * sub.w + jx + xx] == si as i32 {
                        sm.bits[yy * jw + xx] = 1;
                        let (fx, fy) = ((xx as i32 + jx as i32 + x0) as usize, (yy as i32 + jy as i32 + y0) as usize);
                        if fx < a.w && fy < a.h {
                            sub_colors.push(a.at(fx, fy));
                        }
                    }
                }
            }
            if ci == -1 {
                emitted.push((
                    jarea,
                    emit_shapes(&comp_contours(&sm, 1.0), "#ffffff", y0 + jy as i32, x0 + jx as i32),
                    1,
                ));
            } else {
                let col = dominant_color(&sub_colors);
                emitted.push((
                    jarea,
                    emit_shapes(&comp_contours(&sm, 1.2), &rgbf(col), y0 + jy as i32, x0 + jx as i32),
                    0,
                ));
            }
        }
    }

    // holes: content enclosed by the component but not part of it
    let filled = fill_holes(comp_mask);
    let mut holes = Mask::new(w, h);
    let mut any_hole = false;
    for i in 0..w * h {
        if filled.bits[i] != 0 && comp_mask.bits[i] == 0 {
            holes.bits[i] = 1;
            any_hole = true;
        }
    }
    if any_hole {
        let hl = connected_components(&holes);
        for hi in 1..=hl.count {
            let st = hl.stats[hi];
            let (jx, jy, jw, jh, jarea) = (st.x as usize, st.y as usize, st.w as usize, st.h as usize, st.area);
            if (jarea as f64) < spec.p("black_min_component_area") {
                continue;
            }
            let mut sm = Mask::new(jw, jh);
            let mut hcolors = Colors::with_capacity(jarea as usize);
            for yy in 0..jh {
                for xx in 0..jw {
                    if hl.lab[(jy + yy) * hl.w + jx + xx] == hi as i32 {
                        sm.bits[yy * jw + xx] = 1;
                        let (fx, fy) = ((xx as i32 + jx as i32 + x0) as usize, (yy as i32 + jy as i32 + y0) as usize);
                        if fx < a.w && fy < a.h {
                            hcolors.push(a.at(fx, fy));
                        }
                    }
                }
            }
            // white when bright and uniform, else dominant color
            let hmean = hcolors.mean();
            let mut uniform = true;
            for i in 0..hcolors.len() {
                let c = hcolors.at(i);
                if (c[0] - hmean[0]).abs().max((c[1] - hmean[1]).abs()).max((c[2] - hmean[2]).abs()) >= 40.0 {
                    uniform = false;
                    break;
                }
            }
            let bright = {
                let mut b = false;
                for i in 0..hcolors.len() {
                    if hcolors.max_chan(i) > 190.0 {
                        b = true;
                        break;
                    }
                }
                b
            };
            let fill = if bright && uniform {
                "#ffffff".to_string()
            } else {
                rgbf(dominant_color(&hcolors))
            };
            emitted.push((jarea, emit_shapes(&comp_contours(&sm, 1.0), &fill, y0 + jy as i32, x0 + jx as i32), 1));
        }
    }
    emitted.sort_by(|a, b| a.2.cmp(&b.2).then(b.0.cmp(&a.0)));
    let mut parts = Vec::new();
    for (_, els, _) in emitted {
        parts.extend(els);
    }
    parts
}

// ------------------------------------------------------- generic pipeline

/// White text/shapes drawn ON a flat colored region.
pub fn white_glyph_holes(
    a: &Img,
    comp: &Mask, // crop-local
    col: [f32; 3],
    y: i32,
    x: i32,
    spec: &Spec,
) -> Vec<String> {
    let (w, h) = (comp.w, comp.h);
    let mut holes = Mask::new(w, h);
    for yy in 0..h {
        for xx in 0..w {
            if comp.bits[yy * w + xx] == 0 {
                continue;
            }
            let c = a.at((xx as i32 + x) as usize, (yy as i32 + y) as usize);
            let diff = (c[0] - col[0]).abs().max((c[1] - col[1]).abs()).max((c[2] - col[2]).abs());
            let mx = c[0].max(c[1]).max(c[2]);
            if diff > 60.0 && mx > 190.0 {
                holes.bits[yy * w + xx] = 1;
            }
        }
    }
    let hl = connected_components(&holes);
    let mut out = Vec::new();
    for hi in 1..=hl.count {
        let st = hl.stats[hi];
        let (jx, jy, jw, jh, jarea) = (st.x as usize, st.y as usize, st.w as usize, st.h as usize, st.area);
        if (jarea as f64) < spec.p("black_min_component_area") {
            continue;
        }
        let mut sm = Mask::new(jw, jh);
        for yy in 0..jh {
            for xx in 0..jw {
                if hl.lab[(jy + yy) * hl.w + jx + xx] == hi as i32 {
                    sm.bits[yy * jw + xx] = 1;
                }
            }
        }
        out.extend(emit_shapes(&comp_contours(&sm, 1.0), "#ffffff", y + jy as i32, x + jx as i32));
    }
    out
}

/// Segment the cleaned image into region/marker SVG elements.
/// Returns (elements, content_mask).
pub fn generic_elements(a: &Img, spec: &Spec, marker_zone_excl: Option<&Mask>) -> (Vec<String>, Mask) {
    let seg = seg_mask(a);
    let labels0 = connected_components(&seg);
    let (lab, count) = merge_across_gaps(&labels0, spec.p("gap_merge_radius") as i32);
    // recompute per-label stats on merged labels
    let mut stats_by: Vec<(i32, i32, i32, i32, i64)> = vec![(0, 0, 0, 0, 0); count + 1]; // x,y,w,h,area
    {
        let mut acc: Vec<(i32, i32, i32, i32, i64)> =
            vec![(i32::MAX, i32::MAX, i32::MIN, i32::MIN, 0); count + 1];
        for i in 0..a.w * a.h {
            let l = lab[i];
            if l == 0 {
                continue;
            }
            let (x, y) = ((i % a.w) as i32, (i / a.w) as i32);
            let s = &mut acc[l as usize];
            if x < s.0 { s.0 = x; }
            if y < s.1 { s.1 = y; }
            if x > s.2 { s.2 = x; }
            if y > s.3 { s.3 = y; }
            s.4 += 1;
        }
        for l in 1..=count {
            let s = acc[l];
            if s.4 > 0 {
                stats_by[l] = (s.0, s.1, s.2 - s.0 + 1, s.3 - s.1 + 1, s.4);
            }
        }
    }

    let min_region = (spec.p("region_min_area_frac") * (a.w * a.h) as f64) as i64;
    let max_marker = (spec.p("marker_max_area_frac") * (a.w * a.h) as f64) as i64;

    let mut elements = Vec::new();
    for l in 1..=count {
        let (x, y, w, h, area) = stats_by[l];
        if area == 0 {
            continue;
        }
        let mut comp = Mask::new(w as usize, h as usize);
        let mut coords: Vec<(usize, usize)> = Vec::with_capacity(area as usize);
        let mut excl_hits = 0i64;
        for yy in y..y + h {
            for xx in x..x + w {
                if lab[yy as usize * a.w + xx as usize] == l as i32 {
                    comp.bits[(yy - y) as usize * w as usize + (xx - x) as usize] = 1;
                    coords.push((xx as usize, yy as usize));
                    if let Some(ex) = marker_zone_excl {
                        if ex.get(xx, yy) {
                            excl_hits += 1;
                        }
                    }
                }
            }
        }
        if excl_hits as f64 / area as f64 > 0.5 {
            continue;
        }
        if area < min_region {
            continue;
        }
        let mut colors = Colors::with_capacity(coords.len());
        for &(cx, cy) in &coords {
            colors.push(a.at(cx, cy));
        }
        let p10 = colors.percentile3(10.0);
        let p90 = colors.percentile3(90.0);
        let spread = ((p90[0] - p10[0]).max(p90[1] - p10[1]).max(p90[2] - p10[2])) as f32;
        if area <= max_marker {
            let (els, _) = marker_geometry(a, &comp, y, x, spec.p("circularity_min"));
            elements.extend(els);
        } else if spread < spec.p("flat_spread") as f32 {
            let col = dominant_color(&colors);
            elements.extend(emit_shapes(&comp_contours(&comp, 1.2), &rgbf(col), y, x));
            elements.extend(white_glyph_holes(a, &comp, col, y, x, spec));
        } else {
            let (axis, _r) = axial_gradient(a, &coords, spec);
            match axis {
                Some(ax) => elements.extend(gradient_strip_shapes(a, &comp, ax, y, x, spec)),
                None => elements.extend(split_multicolor(a, &comp, y, x, spec)),
            }
        }
    }
    (elements, seg)
}

// ---------------------------------------------------------- black layer

/// Dark mask components; drop CCs smaller than black_min_component_area.
pub fn black_layer(a: &Img, spec: &Spec, extra_excl: Option<&Mask>) -> (Mask, usize, usize) {
    let mut bm = dark_mask(a, spec);
    if let Some(ex) = extra_excl {
        for i in 0..bm.bits.len() {
            if ex.bits[i] != 0 {
                bm.bits[i] = 0;
            }
        }
    }
    let labels = connected_components(&bm);
    let min_area = spec.p("black_min_component_area") as i64;
    let mut keep = vec![false; labels.count + 1];
    for li in 1..=labels.count {
        keep[li] = labels.stats[li].area >= min_area;
    }
    let mut clean = Mask::new(a.w, a.h);
    for i in 0..a.w * a.h {
        let l = labels.lab[i];
        clean.bits[i] = (l > 0 && keep[l as usize]) as u8;
    }
    let n_keep = keep.iter().filter(|&&k| k).count();
    (clean, n_keep, labels.count - n_keep)
}
