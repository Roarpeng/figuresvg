//! Row-banded layout: detection (with continuity gate), band repainting
//! (safe denominators -- no NaN/uninitialized pixels), strip rects,
//! recolor donor rows, zone-guarded markers.

use crate::cc::connected_components;
use crate::img::{rgbf, Img};
use crate::mask::{runs, Mask};
use crate::masks::colored_mask;
use crate::spec::{Recolor, RowSpec, Spec};

/// Continuity gate (fix over the python detector): a real band row is ONE
/// horizontal colored run spanning >= 90% of the zone width. Heatmaps /
/// bar grids are discrete cell lattices -> their rows never reach 90% ->
/// generic path. Accept rowbanded only when >= half of the valid (colored)
/// rows are continuous bands.
const BAND_RUN_FRAC: f64 = 0.55;
const BAND_ROW_MIN_FRAC: f64 = 0.50;

pub struct RbDetect {
    pub rows: Vec<(i32, i32)>, // (top, bottom) pixel rows, bottom exclusive
    pub zone: (i32, i32),      // x extent
}

pub fn detect_rowbanded(a: &Img, spec: &Spec) -> Option<RbDetect> {
    let _ = spec;
    let (w, h) = (a.w as i64, a.h as i64);
    let cm = colored_mask(a);
    // column coverage of colored pixels
    let mut colcnt = vec![0i64; a.w];
    for (x, _y) in cm.pixels() {
        colcnt[x] += 1;
    }
    let cov_thr = (h as f64 * 0.10) as i64;
    let zones = runs(
        &(0..a.w).map(|x| colcnt[x] > cov_thr).collect::<Vec<_>>(),
        20usize.max(a.w / 200),
        20usize.max(a.w / 100),
    );
    if zones.is_empty() {
        return None;
    }
        // a band zone is WIDE: narrow tall zones are colorbars/axis strips
    let min_zone = ((w as f64 * 0.12) as i64).max(250);
    let (zx0, zx1) = match zones
        .iter()
        .filter(|z| (z.1 - z.0) as i64 >= min_zone)
        .max_by_key(|z| z.1 - z.0)
    {
        Some(z) => *z,
        None => return None, // no wide zone: colorbars/axis strips only
    };
    let (zx0, zx1) = (zx0 as i64, zx1 as i64);
    if zx1 - zx0 < (w as f64 * 0.08) as i64 {
        return None;
    }

    // ---- continuity gate (fix: reject discrete grids like heatmaps/bars)
    {
        let zw = (zx1 - zx0) as i64;
        let need = (BAND_RUN_FRAC * zw as f64) as i64;
        let mut valid_rows = 0i64;
        let mut band_rows = 0i64;
        for y in 0..a.h {
            let mut max_run = 0i64;
            let mut run = 0i64;
            for x in zx0 as usize..zx1 as usize {
                if cm.bits[y * a.w + x] != 0 {
                    run += 1;
                    max_run = max_run.max(run);
                } else {
                    run = 0;
                }
            }
            if max_run > 0 {
                valid_rows += 1;
                if max_run >= need {
                    band_rows += 1;
                }
            }
        }
        if valid_rows < 50 || (band_rows as f64) < BAND_ROW_MIN_FRAC * valid_rows as f64 {
            return None;
        }
    }

    // per-row median color of colored pixels inside the zone
    let mut rmc: Vec<[f64; 3]> = vec![[f64::NAN; 3]; a.h];
    let mut rchan: [Vec<f32>; 3];
    for y in 0..a.h {
        rchan = [Vec::new(), Vec::new(), Vec::new()];
        for x in zx0 as usize..zx1 as usize {
            if cm.bits[y * a.w + x] != 0 {
                let c = a.at(x, y);
                rchan[0].push(c[0]);
                rchan[1].push(c[1]);
                rchan[2].push(c[2]);
            }
        }
        if !rchan[0].is_empty() {
            rmc[y] = [
                crate::stats::median(&rchan[0]),
                crate::stats::median(&rchan[1]),
                crate::stats::median(&rchan[2]),
            ];
        }
    }
    let valid: Vec<usize> = (0..a.h).filter(|&y| !rmc[y][0].is_nan()).collect();
    if valid.len() < 50 {
        return None;
    }
    // color-jump detector between consecutive valid rows
    let mut jump = vec![0.0f64; a.h];
    for i in 1..a.h {
        let (v0, v1) = (!rmc[i - 1][0].is_nan(), !rmc[i][0].is_nan());
        if v0 && v1 {
            jump[i] = (rmc[i][0] - rmc[i - 1][0]).abs()
                + (rmc[i][1] - rmc[i - 1][1]).abs()
                + (rmc[i][2] - rmc[i - 1][2]).abs();
        }
    }
    // 3-tap mean, zero-padded edges (np.convolve 'same' semantics)
    let mut jump_s = vec![0.0f64; a.h];
    for i in 0..a.h {
        let s = jump[i]
            + if i > 0 { jump[i - 1] } else { 0.0 }
            + if i + 1 < a.h { jump[i + 1] } else { 0.0 };
        jump_s[i] = s / 3.0;
    }
    let cut_runs = runs(&jump_s.iter().map(|&v| v > 25.0).collect::<Vec<_>>(), 1, 6);
    let cuts: Vec<usize> = cut_runs.iter().map(|&(s, e)| (s + e) / 2).collect();
    let mut edges: Vec<usize> = vec![valid[0]];
    edges.extend(cuts);
    edges.push(valid[valid.len() - 1] + 1);
    let blocks: Vec<(usize, usize)> =
        edges.windows(2).map(|p| (p[0], p[1])).filter(|&(s, b)| b - s > 40).collect();
    if blocks.len() < 2 {
        return None;
    }
    let heights: Vec<usize> = blocks.iter().map(|&(s, b)| b - s).collect();
    let pitch = crate::stats::percentile(
        &heights.iter().map(|&v| v as f32).collect::<Vec<_>>(),
        10.0,
    );
    if pitch <= 0.0 {
        return None;
    }
    let mut rows: Vec<(i32, i32)> = Vec::new();
    for ((s, b), hh) in blocks.iter().zip(heights.iter()) {
        let mut k = (*hh as f64 / pitch).round() as i64;
        if k > 1 && (*hh as f64 / k as f64 - pitch).abs() > 0.2 * pitch {
            k = 1;
        }
        let k = k.max(1) as usize;
        let (s, b) = (*s as i64, *b as i64);
        let mut es: Vec<i64> = (0..=k).map(|i| (s as f64 + i as f64 * (b - s) as f64 / k as f64).round() as i64).collect();
        es[k] = b; // exact end anchor
        for j in 0..k {
            rows.push((es[j] as i32, es[j + 1] as i32));
        }
    }
    if rows.len() < 4 {
        return None;
    }
    Some(RbDetect { rows, zone: (zx0 as i32, zx1 as i32) })
}

/// Repaint one band preserving text (t), soft edges (w), saturated px.
///
/// FIX over the python reference: no NaN and no uninitialized pixels. The
/// python version divides by a NaN-masked reference (all channels <=120 ->
/// t = NaN -> the pixel falls into NO branch of an np.empty output). Here
/// reference channels are clamped to >= 1.0 and text depth t is computed
/// only over channels with ref > 120 (0.0 when none qualify), so every
/// pixel lands in exactly one branch.
pub fn paint_band(
    src: &Img,
    dst: &mut Img,
    top: i32,
    bot: i32,
    x0: i32,
    x1: i32,
    ref_col: &[[f32; 3]], // per-column reference colors (length x1-x0)
    strip_col: &[[f32; 3]], // per-column band/strip output colors
) {
    let white = 255.0f32;
    for y in top.max(0) as usize..bot.min(src.h as i32) as usize {
        for x in x0.max(0) as usize..x1.min(src.w as i32) as usize {
            let region = src.at(x, y);
            let pr = ref_col[x - x0 as usize];
            // t: text darkening, mean over channels with ref > 120
            let mut tsum = 0.0f32;
            let mut tn = 0usize;
            for c in 0..3 {
                if pr[c] > 120.0 {
                    let den = pr[c].max(1.0);
                    tsum += (1.0 - region[c] / den).clamp(0.0, 1.0);
                    tn += 1;
                }
            }
            let t = if tn > 0 { tsum / tn as f32 } else { 0.0 };
            // w: edge whitening, max over channels with headroom >= 8
            let mut w = 0.0f32;
            for c in 0..3 {
                let den2 = 255.0 - pr[c];
                if den2 >= 8.0 {
                    w = w.max(((region[c] - pr[c]) / den2).clamp(0.0, 1.0));
                }
            }
            let spread = region[0].max(region[1]).max(region[2]) - region[0].min(region[1]).min(region[2]);
            let bluish = region[2] - region[0];
            let reddish = region[0] - region[2];
            let keep = spread > 60.0 || bluish > 28.0 || reddish > 28.0;
            let grb = strip_col[x - x0 as usize];
            let out = if keep {
                region
            } else if t > 0.02 {
                [
                    (1.0 - t) * grb[0],
                    (1.0 - t) * grb[1],
                    (1.0 - t) * grb[2],
                ]
            } else if w > 0.002 {
                [
                    (1.0 - w) * grb[0] + w * white,
                    (1.0 - w) * grb[1] + w * white,
                    (1.0 - w) * grb[2] + w * white,
                ]
            } else {
                grb
            };
            dst.set(x, y, [out[0].clamp(0.0, 255.0), out[1].clamp(0.0, 255.0), out[2].clamp(0.0, 255.0)]);
        }
    }
}

pub struct RowbandedOut {
    pub rects: Vec<String>,
    pub markers: Vec<(i32, i32, i32, i32, i64)>, // x,y,w,h,area
    pub repainted: Img,
}

/// Rowbanded build. `cleaned` is the noise-whitened source, `orig` the
/// original (markers are read pre-whitening, zone-guarded).
pub fn build_rowbanded(
    cleaned: &Img,
    orig: &Img,
    spec: &Spec,
    rows: &[RowSpec],
    zone: (i32, i32),
    recolor: &[Recolor],
) -> RowbandedOut {
    let px0 = rows.iter().map(|r| r.x0).min().unwrap_or(zone.0 as i64) as i32;
    let px1 = rows.iter().map(|r| r.x1).max().unwrap_or(zone.1 as i64) as i32;
    let ns = spec.p("gradient_strips") as usize;
    let zw = (px1 - px0).max(0) as usize;

    // per-column median reference color per row (from the ref_slice band)
    let mut refs: Vec<Vec<[f32; 3]>> = Vec::with_capacity(rows.len());
    for r in rows {
        let mut colref: Vec<[f32; 3]> = Vec::with_capacity(zw);
        let (s, e) = ((r.top + r.ref_slice[0]) as usize, (r.top + r.ref_slice[1]) as usize);
        let e = e.min(cleaned.h);
        let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for x in px0.max(0) as usize..px1.min(cleaned.w as i32) as usize {
            chans[0].clear();
            chans[1].clear();
            chans[2].clear();
            for y in s..e {
                let c = cleaned.at(x, y);
                chans[0].push(c[0]);
                chans[1].push(c[1]);
                chans[2].push(c[2]);
            }
            if chans[0].is_empty() {
                colref.push([255.0, 255.0, 255.0]);
            } else {
                colref.push([
                    crate::stats::median(&chans[0]) as f32,
                    crate::stats::median(&chans[1]) as f32,
                    crate::stats::median(&chans[2]) as f32,
                ]);
            }
        }
        refs.push(colref);
    }
    // donor rows: per-column median over donor references
    let mut donor: std::collections::HashMap<usize, Vec<[f32; 3]>> = std::collections::HashMap::new();
    for rc in recolor {
        let donors: Vec<&Vec<[f32; 3]>> = rc.donor_rows.iter().filter_map(|&d| refs.get(d)).collect();
        if donors.is_empty() {
            continue;
        }
        let mut merged: Vec<[f32; 3]> = Vec::with_capacity(zw);
        for xi in 0..zw {
            let r: Vec<f32> = donors.iter().map(|dv| dv[xi][0]).collect();
            let g: Vec<f32> = donors.iter().map(|dv| dv[xi][1]).collect();
            let b: Vec<f32> = donors.iter().map(|dv| dv[xi][2]).collect();
            merged.push([
                crate::stats::median(&r) as f32,
                crate::stats::median(&g) as f32,
                crate::stats::median(&b) as f32,
            ]);
        }
        donor.insert(rc.row, merged);
    }

    let mut new = Img { w: cleaned.w, h: cleaned.h, d: cleaned.d.clone() };
    let mut rects: Vec<String> = Vec::with_capacity(rows.len() * ns);
    let wx = (px1 - px0) as f64 / ns as f64;
    for (i, r) in rows.iter().enumerate() {
        let (top, bot) = (r.top as i32, r.bottom as i32);
        let src_cols = donor.get(&i).unwrap_or(&refs[i]);
        // per-column strip index
        let mut idx: Vec<usize> = Vec::with_capacity(zw);
        for x in px0..px1 {
            let k = (((x - px0) as f64) / wx) as usize;
            idx.push(k.min(ns - 1));
        }
        // strip colors = mean of donor/ref per-column colors inside strip
        let mut cols: Vec<[f32; 3]> = vec![[0.0, 0.0, 0.0]; ns];
        let mut cnt = vec![0usize; ns];
        for (xi, &k) in idx.iter().enumerate() {
            let c = src_cols[xi];
            cols[k][0] += c[0];
            cols[k][1] += c[1];
            cols[k][2] += c[2];
            cnt[k] += 1;
        }
        for s in 0..ns {
            if cnt[s] > 0 {
                for c in 0..3 {
                    cols[s][c] /= cnt[s] as f32;
                }
            }
        }
        let per_column: Vec<[f32; 3]> = idx.iter().map(|&k| cols[k]).collect();
        paint_band(cleaned, &mut new, top, bot, px0, px1, &refs[i], &per_column);
        for s in 0..ns {
            let sx0 = px0 as f64 + s as f64 * wx;
            let sx1 = px0 as f64 + (s + 1) as f64 * wx;
            let (sx0, sx1) = (sx0.round() as i32, sx1.round() as i32);
            rects.push(format!(
                r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                sx0,
                top,
                sx1 - sx0 + 1,
                bot - top,
                rgbf(cols[s])
            ));
        }
    }

    // markers on the original (pre-whiten) image, zone-guarded
    let mut mmask = Mask::new(orig.w, orig.h);
    let x_edge0 = px0 - 7;
    for y in 0..orig.h {
        for x in 0..orig.w {
            let (xi, c) = (x as i32, orig.at(x, y));
            let spread = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]);
            let on = if xi < x_edge0 {
                spread > 12.0
            } else if xi < px0 {
                spread > 60.0
            } else if xi < px1 {
                (c[2] - c[0] > 60.0) || ((c[0] - c[2] > 60.0) && (c[1] < 150.0))
            } else {
                false
            };
            if on {
                mmask.bits[y * orig.w + x] = 1;
            }
        }
    }
    let labels = connected_components(&mmask);
    let mut markers = Vec::new();
    for st in labels.stats.iter().take(labels.count + 1).skip(1) {
        if st.area >= 150 && (40..=60).contains(&st.w) {
            markers.push((st.x, st.y, st.w, st.h, st.area));
        }
    }

    RowbandedOut { rects, markers, repainted: new }
}

/// Analyze-time row descriptors from detected row pairs.
pub fn row_specs_from_detect(a: &Img, det: &RbDetect) -> Vec<RowSpec> {
    let cm = colored_mask(a);
    let tm = crate::masks::tinted_mask(a);
    let (zx0, zx1) = det.zone;
    let mut out = Vec::new();
    for &(s, b) in &det.rows {
        let (s2, b2) = ((s + 10) as usize, (b - 10) as usize);
        if s2 >= b2 || b2 > a.h {
            continue;
        }
        let n_rows = (b2 - s2) as f32;
        let mut frac_col = vec![0.0f32; a.w];
        let mut frac_tin = vec![0.0f32; a.w];
        for y in s2..b2 {
            for x in 0..a.w {
                if cm.bits[y * a.w + x] != 0 {
                    frac_col[x] += 1.0;
                }
                if tm.bits[y * a.w + x] != 0 {
                    frac_tin[x] += 1.0;
                }
            }
        }
        for x in 0..a.w {
            frac_col[x] /= n_rows;
            frac_tin[x] /= n_rows;
        }
        let min_len = ((b - s) as usize) / 3;
        let body = runs(&frac_col.iter().map(|&v| v > 0.5).collect::<Vec<_>>(), min_len, 4);
        if body.is_empty() {
            continue;
        }
        let x0 = zx0.max(body.iter().map(|&(bx, _)| bx as i32).min().unwrap());
        let tin_runs = runs(&frac_tin.iter().map(|&v| v > 0.3).collect::<Vec<_>>(), 20, 6);
        let mut x1 = zx1;
        for &(rs, re) in &tin_runs {
            if rs as i32 <= x0 + 50 && re as i32 > x0 {
                x1 = re as i32;
                break;
            }
        }
        out.push(RowSpec {
            top: s as i64,
            bottom: b as i64,
            x0: x0 as i64,
            x1: x1 as i64,
            ref_slice: [10, 45],
        });
    }
    out
}
