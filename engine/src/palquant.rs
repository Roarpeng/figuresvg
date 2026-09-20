//! Palette-layers strategy (TypeSafe-decided 2026-09-20, p=0.99):
//! quantize the cleaned image to its EXACT dominant palette (uniform bins;
//! gradient ramp steps become palette entries), then binary-trace one layer
//! per palette color with its exact fill. No color is ever estimated --
//! fidelity is bounded only by the quantization step and per-layer mask
//! smoothing.

use crate::img::Img;
use crate::mask::Mask;
use crate::spec::Spec;
use std::collections::HashMap;

pub struct Palette {
    /// exact mean color of each surviving bin
    pub colors: Vec<[f32; 3]>,
    /// palette index per pixel; usize::MAX = background (not content)
    pub assign: Vec<usize>,
    /// pixel count per entry
    pub counts: Vec<usize>,
}

/// Uniform-quantize content pixels into palette bins; bin color = exact mean
/// of the bin's pixels; bins below min_count are dropped and their pixels
/// snap to the nearest surviving color (thin AA fringes).
pub fn quantize(img: &Img, spec: &Spec) -> Palette {
    let step = spec.p("palette_step").max(1.0) as i32;
    let min_count = spec.p("palette_min_count").max(1.0) as usize;
    let n = img.w * img.h;

    // content = not near-white (background after noise cleaning is pure white)
    let key = |c: [f32; 3]| -> u64 {
        ((c[0] as i32 / step) as u64) << 32 | ((c[1] as i32 / step) as u64) << 16
            | (c[2] as i32 / step) as u64
    };
    let mut bins: HashMap<u64, (u64, u64, u64, usize)> = HashMap::new();
    let mut content = vec![false; n];
    for i in 0..n {
        let c = [img.d[i * 3], img.d[i * 3 + 1], img.d[i * 3 + 2]];
        if c[0] > 250.5 && c[1] > 250.5 && c[2] > 250.5 {
            continue;
        }
        content[i] = true;
        let e = bins.entry(key(c)).or_insert((0, 0, 0, 0));
        e.0 += c[0] as u64;
        e.1 += c[1] as u64;
        e.2 += c[2] as u64;
        e.3 += 1;
    }
    let mut kept: Vec<(u64, [f32; 3], usize)> = bins
        .into_iter()
        .filter(|(_, v)| v.3 >= min_count)
        .map(|(k, (sr, sg, sb, cnt))| {
            (k, [sr as f32 / cnt as f32, sg as f32 / cnt as f32, sb as f32 / cnt as f32], cnt)
        })
        .collect();
    kept.sort_unstable_by(|a, b| b.2.cmp(&a.2));

    let colors: Vec<[f32; 3]> = kept.iter().map(|k| k.1).collect();
    let mut bin_index: HashMap<u64, usize> = HashMap::new();
    for (i, k) in kept.iter().enumerate() {
        bin_index.insert(k.0, i);
    }
    // assign: kept bin -> its index; dropped-bin pixel -> nearest kept color
    let mut assign = vec![usize::MAX; n];
    for i in 0..n {
        if !content[i] {
            continue;
        }
        let c = [img.d[i * 3], img.d[i * 3 + 1], img.d[i * 3 + 2]];
        if let Some(&idx) = bin_index.get(&key(c)) {
            assign[i] = idx;
        }
    }
    for i in 0..n {
        if content[i] && assign[i] == usize::MAX {
            let c = [img.d[i * 3], img.d[i * 3 + 1], img.d[i * 3 + 2]];
            let (best, _) = colors
                .iter()
                .enumerate()
                .map(|(j, &k)| {
                    (j, (c[0] - k[0]).abs().max((c[1] - k[1]).abs()).max((c[2] - k[2]).abs()))
                })
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .unwrap();
            assign[i] = best;
        }
    }
    let counts = kept.iter().map(|k| k.2).collect();
    Palette { colors, assign, counts }
}

impl Palette {
    pub fn layer_mask(&self, idx: usize, w: usize, h: usize) -> Mask {
        let mut m = Mask::new(w, h);
        for i in 0..self.assign.len() {
            if self.assign[i] == idx {
                m.bits[i] = 1;
            }
        }
        m
    }
}

pub fn rgbf(c: [f32; 3]) -> String {
    format!("rgb({},{},{})", c[0].round() as i32, c[1].round() as i32, c[2].round() as i32)
}
