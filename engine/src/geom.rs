//! Contour tracing (Moore), shoelace area, arc length, Douglas-Peucker,
//! shape emission (<rect>/<polygon> strings identical to the python side).

use crate::cc::{connected_components, Labels, CompStat};
use crate::img::Img;
use crate::mask::Mask;
use crate::stats::{dominant_color, Colors};

const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
]; // clockwise, y down

fn dir_index(dx: i32, dy: i32) -> usize {
    for (i, &(ex, ey)) in DIRS.iter().enumerate() {
        if ex == dx && ey == dy {
            return i;
        }
    }
    0
}

/// Moore-neighbor boundary trace of one blob; `fg(x,y)` membership test.
/// Returns the closed boundary ring of pixel coords (start not repeated).
fn trace_blob(fg: &impl Fn(i32, i32) -> bool, start: (i32, i32)) -> Vec<(i32, i32)> {
    let mut contour = vec![start];
    let mut p = start;
    let mut b = (start.0 - 1, start.1); // backtrack: west of topmost-leftmost
    let init = (p, b);
    let max_steps = 40_000_000usize;
    let mut steps = 0usize;
    loop {
        steps += 1;
        if steps > max_steps {
            break;
        }
        let db = dir_index(b.0 - p.0, b.1 - p.1);
        let mut next: Option<(i32, i32)> = None;
        for k in 1..=8 {
            let d = (db + k) % 8;
            let n = (p.0 + DIRS[d].0, p.1 + DIRS[d].1);
            if fg(n.0, n.1) {
                let dp = (db + k - 1 + 8) % 8;
                b = (p.0 + DIRS[dp].0, p.1 + DIRS[dp].1);
                next = Some(n);
                break;
            }
        }
        match next {
            None => break, // isolated single pixel
            Some(n) => {
                p = n;
                if p == init.0 && b == init.1 && contour.len() > 1 {
                    break; // Jacob's stopping criterion
                }
                contour.push(p);
            }
        }
    }
    contour
}

/// First (topmost, then leftmost) pixel of label `l`, or None if empty.
fn label_start(labels: &Labels, l: i32, st: &CompStat) -> Option<(i32, i32)> {
    for y in st.y..st.y + st.h {
        for x in st.x..st.x + st.w {
            if labels.lab[y as usize * labels.w + x as usize] == l {
                return Some((x, y));
            }
        }
    }
    None
}

/// External contours (one per 8-connected blob) of a mask.
pub fn external_contours(m: &Mask) -> Vec<Vec<(i32, i32)>> {
    let labels = connected_components(m);
    let mut out = Vec::with_capacity(labels.count);
    for (li, stat) in labels.stats.iter().enumerate().take(labels.count + 1).skip(1) {
        if let Some(start) = label_start(&labels, li as i32, stat) {
            let fg = |x: i32, y: i32| -> bool {
                x >= 0
                    && y >= 0
                    && (x as usize) < labels.w
                    && (y as usize) < labels.h
                    && labels.lab[y as usize * labels.w + x as usize] == li as i32
            };
            out.push(trace_blob(&fg, start));
        }
    }
    out
}

/// |shoelace area| of a closed polygon.
pub fn contour_area(pts: &[(i32, i32)]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0f64;
    for i in 0..n {
        let j = (i + 1) % n;
        a += pts[i].0 as f64 * pts[j].1 as f64 - pts[j].0 as f64 * pts[i].1 as f64;
    }
    (a / 2.0).abs()
}

/// Perimeter of a (closed by default) polyline.
pub fn arc_length(pts: &[(i32, i32)], closed: bool) -> f64 {
    let n = pts.len();
    if n < 2 {
        return 0.0;
    }
    let mut l = 0.0f64;
    for i in 0..n - 1 {
        l += dist(pts[i], pts[i + 1]);
    }
    if closed {
        l += dist(pts[n - 1], pts[0]);
    }
    l
}

fn dist(a: (i32, i32), b: (i32, i32)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    ((dx * dx + dy * dy) as f64).sqrt()
}

fn point_segment_dist(p: (i32, i32), a: (i32, i32), b: (i32, i32)) -> f64 {
    let (ax, ay) = (a.0 as f64, a.1 as f64);
    let (bx, by) = (b.0 as f64, b.1 as f64);
    let (px, py) = (p.0 as f64, p.1 as f64);
    let (vx, vy) = (bx - ax, by - ay);
    let (wx, wy) = (px - ax, py - ay);
    let vv = vx * vx + vy * vy;
    let t = if vv == 0.0 { 0.0 } else { ((wx * vx + wy * vy) / vv).clamp(0.0, 1.0) };
    let (dx, dy) = (px - (ax + t * vx), py - (ay + t * vy));
    (dx * dx + dy * dy).sqrt()
}

fn dp_open(pts: &[(i32, i32)], eps: f64, out: &mut Vec<(i32, i32)>) {
    let n = pts.len();
    if n <= 2 {
        out.extend_from_slice(pts);
        return;
    }
    let (mut idx, mut dmax) = (0usize, 0.0f64);
    for (i, &p) in pts.iter().enumerate().take(n - 1).skip(1) {
        let d = point_segment_dist(p, pts[0], pts[n - 1]);
        if d > dmax {
            dmax = d;
            idx = i;
        }
    }
    if dmax > eps {
        dp_open(&pts[..=idx], eps, out);
        out.pop(); // shared point
        dp_open(&pts[idx..], eps, out);
    } else {
        out.push(pts[0]);
        out.push(pts[n - 1]);
    }
}

/// Douglas-Peucker. `closed`: ring input; result does not repeat the first pt.
pub fn approx_poly_dp(pts: &[(i32, i32)], eps: f64, closed: bool) -> Vec<(i32, i32)> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    if !closed {
        let mut out = Vec::new();
        dp_open(pts, eps, &mut out);
        return dedup_consecutive(out);
    }
    let (mut idx, mut dmax) = (0usize, 0.0f64);
    for (i, &p) in pts.iter().enumerate().skip(1) {
        let d = dist(p, pts[0]);
        if d > dmax {
            dmax = d;
            idx = i;
        }
    }
    if idx == 0 || dmax <= eps {
        return dedup_consecutive(vec![pts[0], *pts.last().unwrap()]);
    }
    let mut out = Vec::new();
    let mut chain2: Vec<(i32, i32)> = pts[idx..].to_vec();
    chain2.push(pts[0]);
    dp_open(&pts[..=idx], eps, &mut out);
    out.pop(); // shared anchor
    dp_open(&chain2, eps, &mut out);
    out.pop(); // closing anchor == pts[0]
    dedup_consecutive(out)
}

fn dedup_consecutive(mut pts: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    if pts.len() > 1 && pts[0] == pts[pts.len() - 1] {
        pts.pop();
    }
    pts.dedup();
    pts
}

/// Port of comp_contours: external contours, area>=12 kept, DP-simplified.
pub fn comp_contours(m: &Mask, eps: f64) -> Vec<Vec<(i32, i32)>> {
    external_contours(m)
        .into_iter()
        .filter(|c| contour_area(c) >= 12.0)
        .map(|c| approx_poly_dp(&c, eps, true))
        .filter(|c| c.len() >= 2)
        .collect()
}

const RECT_FILL_RATIO: f64 = 0.985; // PARAMS["rect_fill_ratio"]

/// <rect> when the contour fills its bbox, else <polygon> (python emit_shapes).
pub fn emit_shapes(contours: &[Vec<(i32, i32)>], fill: &str, x_off: i32, y_off: i32) -> Vec<String> {
    let mut parts = Vec::new();
    for pts in contours {
        if pts.len() < 2 {
            continue;
        }
        let mut x0 = i32::MAX;
        let mut y0 = i32::MAX;
        let mut x1 = i32::MIN;
        let mut y1 = i32::MIN;
        for &(x, y) in pts {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        if pts.len() == 4 && contour_area(pts) / (w * h) as f64 > RECT_FILL_RATIO {
            parts.push(format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                x0 + x_off,
                y0 + y_off,
                w,
                h,
                fill
            ));
        } else {
            let pstr: Vec<String> = pts
                .iter()
                .map(|&(x, y)| format!("{},{}", x + x_off, y + y_off))
                .collect();
            parts.push(format!(r#"<polygon points="{}" fill="{}"/>"#, pstr.join(" "), fill));
        }
    }
    parts
}

/// Marker blob geometry: circle when 4*pi*A/P^2 >= circularity_min, else
/// polygon. Color/position from the largest sub-component of `comp_mask_yx`
/// (crop-local mask). (x, y) = crop origin in full-image coords.
pub fn marker_geometry(
    a: &Img,
    comp_mask_yx: &Mask,
    y: i32,
    x: i32,
    spec_circularity_min: f64,
) -> (Vec<String>, Option<[f32; 3]>) {
    let labels = connected_components(comp_mask_yx);
    if labels.count < 1 {
        return (vec![], None);
    }
    let mut j = 1usize;
    let mut best: i64 = -1;
    for (i, s) in labels.stats.iter().enumerate().skip(1) {
        if s.area > best {
            best = s.area;
            j = i;
        }
    }
    let st: CompStat = labels.stats[j];
    let (jx, jy, jw, jh, jarea) = (st.x as usize, st.y as usize, st.w as usize, st.h as usize, st.area);
    // local mask of sub-CC j inside its bbox
    let mut jcomp = Mask::new(jw, jh);
    for yy in 0..jh {
        for xx in 0..jw {
            if labels.lab[(jy + yy) * labels.w + jx + xx] == j as i32 {
                jcomp.bits[yy * jw + xx] = 1;
            }
        }
    }
    // color from 6px-eroded core when the blob is big enough
    let mut use_core = false;
    if jw > 14 && jh > 14 {
        let mut core_cnt = 0usize;
        for yy in 6..jh - 6 {
            for xx in 6..jw - 6 {
                if jcomp.bits[yy * jw + xx] != 0 {
                    core_cnt += 1;
                }
            }
        }
        use_core = core_cnt >= 5;
    }
    let (ys0, xs0, ys1, xs1) = if use_core {
        (6usize, 6usize, jh - 6, jw - 6)
    } else {
        (0usize, 0usize, jh, jw)
    };
    let mut col_colors = Colors::default();
    for yy in ys0..ys1 {
        for xx in xs0..xs1 {
            if jcomp.bits[yy * jw + xx] != 0 {
                let fx = (x + jx as i32 + xx as i32) as usize;
                let fy = (y + jy as i32 + yy as i32) as usize;
                if fx < a.w && fy < a.h {
                    col_colors.push(a.at(fx, fy));
                }
            }
        }
    }
    if col_colors.len() == 0 {
        return (vec![], None);
    }
    let col = dominant_color(&col_colors);

    let contours = external_contours(&jcomp);
    let per = contours.first().map(|c| arc_length(c, true)).unwrap_or(0.0).max(1.0);
    let circ = 4.0 * std::f64::consts::PI * jarea as f64 / (per * per);
    if circ >= spec_circularity_min {
        let r = (jarea as f64 / std::f64::consts::PI).sqrt();
        let (cx, cy) = (x as f64 + st.cx, y as f64 + st.cy);
        return (
            vec![format!(
                r#"<circle cx="{:.1}" cy="{:.1}" r="{:.1}" fill="{}"/>"#,
                cx,
                cy,
                r,
                crate::img::rgbf(col)
            )],
            Some(col),
        );
    }
    let approx: Vec<Vec<(i32, i32)>> =
        contours.first().map(|c| vec![approx_poly_dp(c, 1.0, true)]).unwrap_or_default();
    (emit_shapes(&approx, &crate::img::rgbf(col), y + jy as i32, x + jx as i32), Some(col))
}
