//! Percentiles, medians, dominant-color (numpy-equivalent, linear interp).

/// numpy-style percentile with linear interpolation (clones + sorts).
pub fn percentile(v: &[f32], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f32> = v.to_vec();
    s.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    let idx = p * (n as f64 - 1.0) / 100.0; // numpy percentile: p in 0..100
    let lo = idx.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    let frac = idx - lo as f64;
    s[lo] as f64 + frac * (s[hi] as f64 - s[lo] as f64)
}

pub fn median(v: &[f32]) -> f64 {
    percentile(v, 50.0)
}

/// Colors stored channel-wise (avoids [f32;3] Vec overhead for millions of px).
#[derive(Default)]
pub struct Colors {
    pub r: Vec<f32>,
    pub g: Vec<f32>,
    pub b: Vec<f32>,
}

impl Colors {
    pub fn with_capacity(n: usize) -> Colors {
        Colors {
            r: Vec::with_capacity(n),
            g: Vec::with_capacity(n),
            b: Vec::with_capacity(n),
        }
    }
    #[inline]
    pub fn push(&mut self, c: [f32; 3]) {
        self.r.push(c[0]);
        self.g.push(c[1]);
        self.b.push(c[2]);
    }
    pub fn len(&self) -> usize {
        self.r.len()
    }
    pub fn at(&self, i: usize) -> [f32; 3] {
        [self.r[i], self.g[i], self.b[i]]
    }
    pub fn max_chan(&self, i: usize) -> f32 {
        self.r[i].max(self.g[i]).max(self.b[i])
    }
    pub fn min_chan(&self, i: usize) -> f32 {
        self.r[i].min(self.g[i]).min(self.b[i])
    }
    pub fn mean(&self) -> [f32; 3] {
        let n = self.len().max(1);
        let mut s = [0.0f64; 3];
        for i in 0..self.len() {
            s[0] += self.r[i] as f64;
            s[1] += self.g[i] as f64;
            s[2] += self.b[i] as f64;
        }
        [(s[0] / n as f64) as f32, (s[1] / n as f64) as f32, (s[2] / n as f64) as f32]
    }
    /// Per-channel median.
    pub fn median3(&self) -> [f32; 3] {
        [
            median(&self.r) as f32,
            median(&self.g) as f32,
            median(&self.b) as f32,
        ]
    }
    /// Per-channel numpy-percentile p.
    pub fn percentile3(&self, p: f64) -> [f64; 3] {
        [
            percentile(&self.r, p),
            percentile(&self.g, p),
            percentile(&self.b, p),
        ]
    }
}

/// Median re-centered on the dominant cluster (python dominant_color).
pub fn dominant_color(colors: &Colors) -> [f32; 3] {
    let n = colors.len();
    if n == 0 {
        return [255.0, 255.0, 255.0];
    }
    let mut c = colors.median3();
    for _ in 0..2 {
        let mut near = Colors::with_capacity(n);
        for i in 0..n {
            let d = (colors.r[i] - c[0])
                .abs()
                .max((colors.g[i] - c[1]).abs())
                .max((colors.b[i] - c[2]).abs());
            if d < 30.0 {
                near.push(colors.at(i));
            }
        }
        if near.len() < n / 5 {
            break;
        }
        c = near.median3();
    }
    c
}
