//! Object consolidation (IMPLEMENTATION_PLAN S1): merge palette-bin
//! fragments into VISUAL OBJECTS.
//!
//! A gradient bar is spatially contiguous in the content mask but is
//! quantized into many palette bins; a stacked-bar boundary touches but
//! must NOT merge (color distance). Algorithm:
//!   1. connected components of the CONTENT mask (spatial objects)
//!   2. inside each spatial CC, sub-CC per palette color; adjacency graph
//!      between sub-CCs (1px dilation contact)
//!   3. union-find across adjacent sub-CCs whose colors differ by <=
//!      MERGE_DIST (RGB max-channel) -> visual objects
//! White gaps never connect (they are not content), so heatmap cells and
//! gap-separated bars stay separate for free.

use crate::cc;
use crate::img::Img;
use crate::mask::Mask;
use crate::palquant::Palette;

pub const MERGE_DIST: f32 = 44.0;

pub struct GradientInfo {
    pub axis: char, // 'x' | 'y'
    pub from: [f32; 3],
    pub to: [f32; 3],
}

pub struct Object {
    pub mask: Mask,
    pub bbox: (usize, usize, usize, usize), // x, y, w, h (canvas coords)
    pub fill: [f32; 3],
    /// mean color at both ends along the gradient axis (from,to)
    pub gradient: Option<GradientInfo>,
    /// grid cell from the slab guard: emit as a bbox rect, no tracing
    pub grid_cell: bool,
}

/// CC labeling of a sparse pixel list: returns Labels over the full canvas
/// but only fills the given pixels.
fn cc_from_pixels(pixs: &[usize], w: usize, h: usize) -> cc::Labels {
    let mut m = Mask::new(w, h);
    for &gi in pixs {
        m.bits[gi] = 1;
    }
    cc::connected_components(&m)
}

pub fn consolidate(img: &Img, _pal: &Palette) -> Vec<Object> {
    let (w, h) = (img.w, img.h);
    // object-segmentation content mask, rebuilt from RAW pixels: pale
    // gradient tails need spread >= 8 -- the 1-2px anti-aliasing ring
    // around black outlines is near-gray (spread <= 6) and otherwise
    // forms a connected highway that chains every touching mark into
    // one giant component.
    let mut content = Mask::new(w, h);
    for i in 0..w * h {
        let c = [img.d[i * 3], img.d[i * 3 + 1], img.d[i * 3 + 2]];
        let (mx, mn) = (
            c[0].max(c[1]).max(c[2]),
            c[0].min(c[1]).min(c[2]),
        );
        let spread = mx - mn;
        let dark = spread < 25.0 && (c[0] + c[1] + c[2]) / 3.0 < 150.0;
        let is_content = !dark && ((spread > 15.0) || (mx < 252.0 && spread >= 8.0));
        content.bits[i] = is_content as u8;
    }
    // BLACK STROKE objects (error bars, axes, ticks, outlines): dark CCs
    // are excluded from color content but they are first-class objects in
    // the reference format (Origin emits them as polylines).
    let mut darkm = Mask::new(w, h);
    for i in 0..w * h {
        let c = [img.d[i * 3], img.d[i * 3 + 1], img.d[i * 3 + 2]];
        let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
        if (mx - mn < 25.0) && (c[0] + c[1] + c[2]) / 3.0 < 150.0 {
            darkm.bits[i] = 1;
        }
    }
    let lab = cc::connected_components(&content);
    let mut objects = Vec::new();
    
    for s in 1..=lab.count {
        let st = &lab.stats[s];
        if st.area == 0 {
            continue; // empty label slot (post-merge relabeling artifact)
        }
        let (x0, y0) = (st.x.max(0) as usize, st.y.max(0) as usize);
        let (x1, y1) = ((st.x + st.w) as usize, (st.y + st.h) as usize);
        // sub-CC masks as SPARSE pixel index lists (full-image Mask per
        // palette color was O(colors * W * H) bytes and OOM-killed on 28MP)
        let mut sub: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        let mut order: Vec<usize> = Vec::new();
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                let gi = y * w + x;
                if lab.lab[gi] == s as i32 {
                    let c = [img.d[gi * 3], img.d[gi * 3 + 1], img.d[gi * 3 + 2]];
                    let key = ((c[0] as u32 / 8) << 12)
                        | ((c[1] as u32 / 8) << 6)
                        | (c[2] as u32 / 8);
                    sub.entry(key as usize).or_insert_with(|| {
                        order.push(key as usize);
                        Vec::new()
                    }).push(gi);
                }
            }
        }
        if sub.is_empty() {
                        continue;
        }
        if sub.len() == 1 {
            let (_k, pixs) = sub.iter().next().unwrap();
            let area1 = pixs.len();
            // slab guard applies here too: one quantization bin can cover a
            // whole gapless heatmap region (similar cell values -> one bin)
            if area1 as f64 >= 0.04 * (w * h) as f64 {
                eprintln!("[slab-guard1] single-bin area {}", area1);
                let l2 = cc_from_pixels(pixs, w, h);
                let mut emitted = 0;
                for j in 1..=l2.count {
                    if l2.stats[j].area < 40 {
                        continue;
                    }
                    let mut cell = Mask::new(w, h);
                    for &gi in pixs.iter() {
                        if l2.lab[gi] == j as i32 {
                            cell.bits[gi] = 1;
                        }
                    }
                    let mut o = make_object(&cell, img, w, h);
                    o.grid_cell = true;
                    objects.push(o);
                    emitted += 1;
                }
                if emitted > 0 {
                    continue;
                }
            }
            let mut m = Mask::new(w, h);
            for &gi in pixs.iter() {
                m.bits[gi] = 1;
            }
            objects.push(make_object(&m, img, w, h));
            continue;
        }
        // union-find over sub-CC colors by adjacency + color distance
        let idx: Vec<usize> = order.clone();
        let mut parent: Vec<usize> = (0..idx.len()).collect();

        fn find(p: &mut Vec<usize>, i: usize) -> usize {
            let mut i = i;
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }

        // adjacency + EDGE-LOCAL color distance: compare the colors of the
        // pixels FACING each other across the contact line, not global
        // means. Inside one gradient the facing pixels are consecutive ramp
        // steps (close); at a boundary between two different marks the
        // facing pixels are the two ramps' endpoints (far) -- which stops
        // chained over-merging of adjacent gradient bars.
        // SPARSE adjacency: pixel lists + a set for O(1) membership —
        // full-image Masks per color pair was O(colors² × W×H) and
        // OOM-killed 28MP inputs
        let sets: Vec<std::collections::HashSet<usize>> = idx.iter().map(|&i| {
            sub.get(&i).unwrap().iter().cloned().collect()
        }).collect();
        let ranges: Vec<(i64, i64, i64, i64)> = idx.iter().map(|&i| {
            let pixs = sub.get(&i).unwrap();
            let mut x0 = i64::MAX; let mut y0 = i64::MAX;
            let (mut x1, mut y1) = (i64::MIN, i64::MIN);
            for &gi in pixs {
                let (x, y) = ((gi % w) as i64, (gi / w) as i64);
                x0 = x0.min(x); y0 = y0.min(y);
                x1 = x1.max(x); y1 = y1.max(y);
            }
            (x0, y0, x1, y1)
        }).collect();
        for a in 0..idx.len() {
            for b in (a + 1)..idx.len() {
                let (ax0, ay0, ax1, ay1) = ranges[a];
                let (bx0, by0, bx1, by1) = ranges[b];
                // bounding boxes must be within 1px of each other
                if ax0 - 2 > bx1 || bx0 - 2 > ax1 || ay0 - 2 > by1 || by0 - 2 > ay1 {
                    continue;
                }
                // adjacency: any pixel of B within 1px (Chebyshev) of A
                let mut strip_b: Vec<usize> = Vec::new();
                for &gi in sub.get(&idx[b]).unwrap() {
                    let (x, y) = (gi % w, gi / w);
                    // check the 8-neighborhood of (x,y) against A's set
                    let mut adj = false;
                    for dy in [-1i64, 0, 1] {
                        for dx in [-1i64, 0, 1] {
                            let nx = x as i64 + dx;
                            let ny = y as i64 + dy;
                            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                                if sets[a].contains(&(ny as usize * w + nx as usize)) {
                                    adj = true;
                                    break;
                                }
                            }
                        }
                        if adj { break; }
                    }
                    if adj {
                        strip_b.push(gi);
                    }
                }
                if strip_b.is_empty() {
                    continue;
                }
                let mut strip_a: Vec<usize> = Vec::new();
                for &gi in sub.get(&idx[a]).unwrap() {
                    let (x, y) = (gi % w, gi / w);
                    let mut adj = false;
                    for dy in [-1i64, 0, 1] {
                        for dx in [-1i64, 0, 1] {
                            let nx = x as i64 + dx;
                            let ny = y as i64 + dy;
                            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                                if sets[b].contains(&(ny as usize * w + nx as usize)) {
                                    adj = true;
                                    break;
                                }
                            }
                        }
                        if adj { break; }
                    }
                    if adj {
                        strip_a.push(gi);
                    }
                }
                if strip_a.is_empty() {
                    continue;
                }
                let med = |strip: &Vec<usize>| -> [f32; 3] {
                    let mut chans: [Vec<f32>; 3] =
                        [Vec::new(), Vec::new(), Vec::new()];
                    for &gi in strip {
                        for c in 0..3 {
                            chans[c].push(img.d[gi * 3 + c]);
                        }
                    }
                    let mut m = [0f32; 3];
                    for c in 0..3 {
                        chans[c].sort_by(|x, y| x.partial_cmp(y).unwrap());
                        m[c] = chans[c][chans[c].len() / 2];
                    }
                    m
                };
                let (sa, sb) = (med(&strip_a), med(&strip_b));
                let d = (sa[0] - sb[0])
                    .abs()
                    .max((sa[1] - sb[1]).abs())
                    .max((sa[2] - sb[2]).abs());
                if d <= MERGE_DIST {
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    if ra != rb {
                        parent[ra] = rb;
                    }
                }
            }
        }
        // collect merged masks per root
        let mut roots: Vec<usize> = Vec::new();
        for i in 0..idx.len() {
            let r = find(&mut parent, i);
            if !roots.contains(&r) {
                roots.push(r);
            }
        }
                for r in roots {
            let mut members: Vec<usize> = Vec::new();
            for i in 0..idx.len() {
                if find(&mut parent, i) == r {
                    members.push(i);
                }
            }
            // SLAB GUARD: a gapless heatmap merges into one huge slab
            // even though each cell is an object. A merged object covering
            // a big fraction of the canvas with a solid bbox fill is a
            // grid-like region -> reject the merge and emit the COLOR-LEVEL
            // connected components (individual cells) instead.
            {
                let mut test = Mask::new(w, h);
                for &i in &members {
                    for &gi in sub.get(&idx[i]).unwrap() {
                        test.bits[gi] = 1;
                    }
                }
                let area = test.count();
                if area as f64 >= 0.04 * (w * h) as f64 {
                    let mut bx0 = usize::MAX; let mut by0 = usize::MAX;
                    let (mut bx1, mut by1) = (0usize, 0usize);
                    for yy in 0..h {
                        for xx in 0..w {
                            if test.bits[yy * w + xx] != 0 {
                                bx0 = bx0.min(xx); by0 = by0.min(yy);
                                bx1 = bx1.max(xx + 1); by1 = by1.max(yy + 1);
                            }
                        }
                    }
                    let fillr = area as f64
                        / ((bx1 - bx0) * (by1 - by0)) as f64;
                    let _ = &fillr;
                    let _ = fillr;
                {
                        for &i in &members {
                            let mut smask = Mask::new(w, h);
                            for &gi in sub.get(&idx[i]).unwrap() {
                                smask.bits[gi] = 1;
                            }
                            let smask = &smask;
                            let l2 = cc::connected_components(&smask);
                            for j in 1..=l2.count {
                                if l2.stats[j].area < 40 {
                                    continue;
                                }
                                let mut cell = Mask::new(w, h);
                                for yy in 0..h {
                                    for xx in 0..w {
                                        if l2.lab[yy * w + xx] == j as i32 {
                                            cell.bits[yy * w + xx] = 1;
                                        }
                                    }
                                }
                                let mut o = make_object(&cell, img, w, h);
                                o.grid_cell = true;
                                objects.push(o);
                            }
                        }
                        continue;
                    }
                }
            }
            let mut m = Mask::new(w, h);
            let mut fill = None;
            let mut best = 0usize;
            for i in 0..idx.len() {
                if find(&mut parent, i) == r {
                    let sm = sub.get(&idx[i]).unwrap();
                    let cnt = sm.len();
                    if cnt > best {
                        best = cnt;
                        // dominant sub-mask color: median raw pixel of that mask
                        let mut chans: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
                        for &gi in sm {
                            for c in 0..3 {
                                chans[c].push(img.d[gi * 3 + c]);
                            }
                        }
                        let mut col = [0f32; 3];
                        for c in 0..3 {
                            if !chans[c].is_empty() {
                                chans[c].sort_by(|a, b| a.partial_cmp(b).unwrap());
                                col[c] = chans[c][chans[c].len() / 2];
                            }
                        }
                        fill = Some(col);
                    }
                    for &gi in sm {
                        m.bits[gi] = 1;
                    }
                }
            }
            if best == 0 {
                continue;
            }
                        objects.push(make_object(&m, img, w, h).with_fill(fill.unwrap()));
        }
    }
    eprintln!("[objects] pre-growth count: {}", objects.len());
    // grow every object by 1px: the raw content mask excludes the
    // anti-aliasing ring, so dots/edges render 1-2px small (a 5px dot
    // loses ~60% of its area). Fill colors are sampled BEFORE growth so
    // the AA ring does not lighten them.
    for o in objects.iter_mut() {
        // masks are cropped to bbox; grow in cropped space, expand bbox
        let (px, py, pw, ph) = o.bbox;
        let grown = crate::mask::dilate(&o.mask, 1);
        // find the grown mask's bbox in cropped space
        let (mut gx0, mut gy0) = (usize::MAX, usize::MAX);
        let (mut gx1, mut gy1) = (0usize, 0usize);
        for y in 0..grown.h {
            for x in 0..grown.w {
                if grown.bits[y * grown.w + x] != 0 {
                    gx0 = gx0.min(x); gy0 = gy0.min(y);
                    gx1 = gx1.max(x + 1); gy1 = gy1.max(y + 1);
                }
            }
        }
        if gx0 != usize::MAX {
            // crop the grown mask to its own bbox
            let cw2 = gx1 - gx0;
            let ch2 = gy1 - gy0;
            let mut cm2 = Mask::new(cw2, ch2);
            for yy in 0..ch2 {
                for xx in 0..cw2 {
                    cm2.bits[yy * cw2 + xx] = grown.bits[(yy + gy0) * grown.w + (xx + gx0)];
                }
            }
            o.mask = cm2;
            // new canvas bbox (grown by the shift; may exceed canvas by 1px
            // on the right/bottom edge, the emission clamps)
            o.bbox = (px.saturating_sub(1).max(if gx0 == 0 { 1 } else { 0 })
                      + if gx0 == 0 { 0 } else { px + gx0 - 1 } - px + px,
                      py, cw2, ch2);
            // simpler: bbox = old origin - (gx0,gy0) offsets are within ±1
            let nx = (px + gx0).saturating_sub(1);
            let ny = (py + gy0).saturating_sub(1);
            o.bbox = (nx, ny, cw2, ch2);
        }
    }
    eprintln!("[objects] post-growth count: {}", objects.len());
    // dark stroke CCs -> objects (median gray color as fill/stroke)
    let dlab = cc::connected_components(&darkm);
    for s in 1..=dlab.count {
        let st = &dlab.stats[s];
        if st.area < 12 {
            continue;
        }
        let (x0, y0) = (st.x.max(0) as usize, st.y.max(0) as usize);
        let (x1, y1) = ((st.x + st.w) as usize, (st.y + st.h) as usize);
        let mut m = Mask::new(w, h);
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                if dlab.lab[y * w + x] == s as i32 {
                    m.bits[y * w + x] = 1;
                }
            }
        }
        let mut obj = make_object(&m, img, w, h);
        obj.fill = [40.0, 40.0, 40.0];
        objects.push(obj);
    }
    objects
}

fn make_object(m: &Mask, img: &Img, w: usize, h: usize) -> Object {
    // bbox + dominant color + gradient probing
    let (mut bx0, mut by0) = (usize::MAX, usize::MAX);
    let (mut bx1, mut by1) = (0usize, 0usize);
    let mut sum = [0f64; 3];
    let mut n = 0usize;
    for y in 0..h {
        for x in 0..w {
            let gi = y * w + x;
            if m.bits[gi] != 0 {
                bx0 = bx0.min(x);
                by0 = by0.min(y);
                bx1 = bx1.max(x + 1);
                by1 = by1.max(y + 1);
                let c = [img.d[gi * 3], img.d[gi * 3 + 1], img.d[gi * 3 + 2]];
                sum[0] += c[0] as f64;
                sum[1] += c[1] as f64;
                sum[2] += c[2] as f64;
                n += 1;
            }
        }
    }
    let fill = [
        (sum[0] / n as f64) as f32,
        (sum[1] / n as f64) as f32,
        (sum[2] / n as f64) as f32,
    ];
    // store the mask CROPPED to the bbox (full-canvas masks × hundreds of
    // objects = tens of GB on 28MP inputs)
    let cw = bx1 - bx0;
    let ch = by1 - by0;
    let mut cmask = Mask::new(cw, ch);
    for yy in 0..ch {
        for xx in 0..cw {
            cmask.bits[yy * cw + xx] = m.bits[(yy + by0) * w + (xx + bx0)];
        }
    }
    let mut obj = Object {
        mask: cmask,
        bbox: (bx0, by0, cw, ch),
        fill,
        gradient: None,
        grid_cell: false,
    };
    obj.gradient = probe_gradient(&obj, img, w);
    obj
}

impl Object {
    fn with_fill(mut self, fill: [f32; 3]) -> Object {
        self.fill = fill;
        self
    }
}

/// Monotonic color ramp along x or y -> linearGradient info.
fn probe_gradient(obj: &Object, img: &Img, w: usize) -> Option<GradientInfo> {
    let (x, y, bw, bh) = obj.bbox;
    let (mw, mh) = (obj.mask.w, obj.mask.h);
    if bw < 12 || bh < 12 || obj.mask.count() < 400 {
        return None;
    }
    for axis in ['x', 'y'] {
        let len = if axis == 'x' { bw } else { bh };
        let nband = 12usize.min(len);
        let mut means: Vec<[f64; 3]> = Vec::new();
        for b in 0..nband {
            let (s, e) = (b * len / nband, (b + 1) * len / nband);
            let mut sum = [0f64; 3];
            let mut cnt = 0usize;
            for yy in y..y + bh {
                for xx in x..x + bw {
                    let in_band = if axis == 'x' { xx - x >= s && xx - x < e } else { yy - y >= s && yy - y < e };
                    let gi = yy * w + xx;
                    // mask is cropped: local coords are (xx-x, yy-y)
                    let lx = xx - x;
                    let ly = yy - y;
                    if in_band && lx < mw && ly < mh && obj.mask.bits[ly * mw + lx] != 0 {
                        let c = [img.d[gi * 3], img.d[gi * 3 + 1], img.d[gi * 3 + 2]];
                        sum[0] += c[0] as f64;
                        sum[1] += c[1] as f64;
                        sum[2] += c[2] as f64;
                        cnt += 1;
                    }
                }
            }
            if cnt < 8 {
                continue;
            }
            means.push([sum[0] / cnt as f64, sum[1] / cnt as f64, sum[2] / cnt as f64]);
        }
        if means.len() < 8 {
            continue;
        }
        // channel with the largest total variation must be near-monotonic
        let mut best_ch = 0usize;
        let mut best_var = 0f64;
        for ch in 0..3 {
            let v: f64 = means
                .windows(2)
                .map(|w| (w[1][ch] - w[0][ch]).abs())
                .sum();
            if v > best_var {
                best_var = v;
                best_ch = ch;
            }
        }
        if best_var < 48.0 {
            continue; // too flat to be a gradient
        }
        let seq: Vec<f64> = means.iter().map(|m| m[best_ch]).collect();
        let ups = seq.windows(2).filter(|w| w[1] > w[0] + 1e-9).count();
        let downs = seq.windows(2).filter(|w| w[1] < w[0] - 1e-9).count();
        let steps = seq.len() - 1;
        if ups.max(downs) as f64 / steps as f64 >= 0.85 {
            let f = means.first().unwrap();
            let t = means.last().unwrap();
            return Some(GradientInfo {
                axis,
                from: [f[0] as f32, f[1] as f32, f[2] as f32],
                to: [t[0] as f32, t[1] as f32, t[2] as f32],
            });
        }
    }
    None
}
