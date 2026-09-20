//! Pixel classification masks + JPEG noise whitening (python `masks` section).

use crate::img::Img;
use crate::mask::{dilate, Mask};
use crate::spec::Spec;

#[inline]
fn mx(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2])
}
#[inline]
fn mn(c: [f32; 3]) -> f32 {
    c[0].min(c[1]).min(c[2])
}
#[inline]
fn mean(c: [f32; 3]) -> f32 {
    (c[0] + c[1] + c[2]) / 3.0
}

/// True black/gray content: low spread AND low luminance (python _dark,
/// fixed thresholds 25/150 -- dark saturated colors must NOT match).
pub fn dark_base(a: &Img) -> Mask {
    let mut m = Mask::new(a.w, a.h);
    for y in 0..a.h {
        for x in 0..a.w {
            let c = a.at(x, y);
            m.bits[y * a.w + x] = ((mx(c) - mn(c) < 25.0) && (mean(c) < 150.0)) as u8;
        }
    }
    m
}

/// Definitely-colored pixels (detection aid; excludes black and white).
pub fn colored_mask(a: &Img) -> Mask {
    let dark = dark_base(a);
    let mut m = Mask::new(a.w, a.h);
    for y in 0..a.h {
        for x in 0..a.w {
            let c = a.at(x, y);
            m.bits[y * a.w + x] =
                ((mx(c) - mn(c) > 15.0) && dark.bits[y * a.w + x] == 0 && (mn(c) > 8.0)) as u8;
        }
    }
    m
}

/// All content-colored pixels (python seg_mask).
pub fn seg_mask(a: &Img) -> Mask {
    let dark = dark_base(a);
    let mut m = Mask::new(a.w, a.h);
    for y in 0..a.h {
        for x in 0..a.w {
            let c = a.at(x, y);
            let is_dark = dark.bits[y * a.w + x] != 0;
            m.bits[y * a.w + x] =
                (((mx(c) - mn(c) > 15.0) || (mx(c) < 252.0)) && !is_dark && (mn(c) > 8.0)) as u8;
        }
    }
    m
}

/// Back-compat alias: content mask used for extent/verification.
pub fn tinted_mask(a: &Img) -> Mask {
    seg_mask(a)
}

/// Black layer mask (params: black_lum_max / black_spread_max).
pub fn dark_mask(a: &Img, spec: &Spec) -> Mask {
    let lum_max = spec.p("black_lum_max");
    let spread_max = spec.p("black_spread_max");
    let mut m = Mask::new(a.w, a.h);
    for y in 0..a.h {
        for x in 0..a.w {
            let c = a.at(x, y);
            m.bits[y * a.w + x] = ((mean(c) < lum_max as f32) && ((mx(c) - mn(c)) < spread_max as f32)) as u8;
        }
    }
    m
}

/// Push near-white JPEG chroma noise to pure white (away from content).
/// Returns (cleaned image, noise mask).
pub fn clean_noise(a: &Img, spec: &Spec) -> (Img, Mask) {
    let mut confident = colored_mask(a);
    let dm = dark_mask(a, spec);
    for i in 0..confident.bits.len() {
        confident.bits[i] = (confident.bits[i] != 0 || dm.bits[i] != 0) as u8;
    }
    let r = spec.p("noise_guard_radius") as i32;
    let near_content = dilate(&confident, r);
    let whiten_min = spec.p("noise_whiten_min") as f32;
    let whiten_spread = spec.p("noise_whiten_spread") as f32;
    let mut cand = Mask::new(a.w, a.h);
    for y in 0..a.h {
        for x in 0..a.w {
            let c = a.at(x, y);
            if mn(c) > whiten_min
                && (mx(c) - mn(c)) < whiten_spread
                && near_content.bits[y * a.w + x] == 0
            {
                cand.bits[y * a.w + x] = 1;
            }
        }
    }
    // Only SMALL isolated blobs are noise; a large connected near-white
    // area is a pale fill (pastel segments) and must survive (python parity).
    let lab = crate::cc::connected_components(&cand);
    let max_blob = (spec.p("noise_max_blob_frac") * (a.w * a.h) as f64) as i64;
    let mut keep = Mask::new(a.w, a.h);
    for j in 1..=lab.count {
        if lab.stats[j].area <= max_blob {
            let st = &lab.stats[j];
            let (x0, y0) = (st.x.max(0) as usize, st.y.max(0) as usize);
            let (x1, y1) = ((st.x + st.w) as usize, (st.y + st.h) as usize);
            for y in y0..y1.min(a.h) {
                for x in x0..x1.min(a.w) {
                    if lab.lab[y * a.w + x] == j as i32 {
                        keep.bits[y * a.w + x] = 1;
                    }
                }
            }
        }
    }
    let mut out = Img { w: a.w, h: a.h, d: a.d.clone() };
    for i in 0..keep.bits.len() {
        if keep.bits[i] != 0 {
            out.d[i * 3] = 255.0;
            out.d[i * 3 + 1] = 255.0;
            out.d[i * 3 + 2] = 255.0;
        }
    }
    (out, keep)
}
