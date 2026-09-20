//! Binary masks, separable rectangular dilate/erode, 1-D run extraction.

use std::collections::VecDeque;

#[derive(Clone)]
pub struct Mask {
    pub w: usize,
    pub h: usize,
    pub bits: Vec<u8>, // 0/1
}

impl Mask {
    pub fn new(w: usize, h: usize) -> Mask {
        Mask { w, h, bits: vec![0; w * h] }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
            && self.bits[y as usize * self.w + x as usize] != 0
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, v: bool) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.bits[y as usize * self.w + x as usize] = v as u8;
        }
    }

    pub fn count(&self) -> usize {
        self.bits.iter().filter(|&&b| b != 0).count()
    }

    /// Iterate set pixels as (x, y).
    pub fn pixels(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let w = self.w;
        self.bits.iter().enumerate().filter_map(move |(i, &b)| {
            if b != 0 { Some((i % w, i / w)) } else { None }
        })
    }
}

/// One sliding-window pass over a strided sequence of u8 values.
/// Window for output element i is [i-r, i+r] clipped to [0, n); monotone
/// deque, O(n). is_max=false -> min filter (erode).
fn sliding_pass(
    n: usize,
    r: i32,
    is_max: bool,
    load: impl Fn(usize) -> u8,
    mut store: impl FnMut(usize, u8),
) {
    let mut dq: VecDeque<(usize, u8)> = VecDeque::new();
    for i in 0..n {
        let v = load(i);
        while let Some(&(_, bv)) = dq.back() {
            let drop = if is_max { bv <= v } else { bv >= v };
            if drop {
                dq.pop_back();
            } else {
                break;
            }
        }
        dq.push_back((i, v));
        let lo = i as i64 - r as i64;
        while let Some(&(f, _)) = dq.front() {
            if (f as i64) < lo {
                dq.pop_front();
            } else {
                break;
            }
        }
        store(i, dq.front().unwrap().1);
    }
}

/// Rectangular (2r+1)^2 dilation (max filter), separable.
pub fn dilate(m: &Mask, r: i32) -> Mask {
    if r <= 0 {
        return m.clone();
    }
    let (w, h) = (m.w, m.h);
    let mut tmp = vec![0u8; w * h];
    for y in 0..h {
        let row = y * w;
        sliding_pass(
            w,
            r,
            true,
            |x| m.bits[row + x],
            |x, v| tmp[row + x] = v,
        );
    }
    let mut out = Mask::new(w, h);
    for x in 0..w {
        sliding_pass(
            h,
            r,
            true,
            |y| tmp[y * w + x],
            |y, v| out.bits[y * w + x] = v,
        );
    }
    out
}

/// Rectangular (2r+1)^2 erosion (min filter), separable. Out-of-image
/// treated as +inf (cv2 erode default border), i.e. clipped windows.
pub fn erode(m: &Mask, r: i32) -> Mask {
    if r <= 0 {
        return m.clone();
    }
    let (w, h) = (m.w, m.h);
    let mut tmp = vec![1u8; w * h];
    for y in 0..h {
        let row = y * w;
        sliding_pass(
            w,
            r,
            false,
            |x| m.bits[row + x],
            |x, v| tmp[row + x] = v,
        );
    }
    let mut out = Mask::new(w, h);
    for x in 0..w {
        sliding_pass(
            h,
            r,
            false,
            |y| tmp[y * w + x],
            |y, v| out.bits[y * w + x] = v,
        );
    }
    out
}

/// Port of python runs(): true-intervals of `flags` merged by gaps smaller
/// than `merge_gap`, keeping intervals of length >= min_len.
pub fn runs(flags: &[bool], min_len: usize, merge_gap: usize) -> Vec<(usize, usize)> {
    let idx: Vec<usize> = flags.iter().enumerate().filter(|&(_, &f)| f).map(|(i, _)| i).collect();
    if idx.is_empty() {
        return vec![];
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut s = idx[0];
    let mut p = idx[0];
    for &i in &idx[1..] {
        if i - p > merge_gap {
            merged.push((s, p + 1));
            s = i;
        }
        p = i;
    }
    merged.push((s, p + 1));
    merged.into_iter().filter(|&(a, b)| b - a >= min_len).collect()
}
