//! RGB float image buffer + load/save (replaces numpy/PIL arrays).

#[derive(Clone)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub d: Vec<f32>, // len = w*h*3, row-major RGB
}

impl Img {
    pub fn new(w: usize, h: usize) -> Img {
        Img { w, h, d: vec![255.0; w * h * 3] }
    }

    pub fn load(path: &str) -> Result<Img, String> {
        // sniff content, never trust the extension: real inputs include
        // PNG payloads named .jpeg (PIL tolerates this; extension-based
        // format guessing does not)
        let bytes = std::fs::read(path).map_err(|e| format!("cannot open {path}: {e}"))?;
        let dyn_img = image::load_from_memory(&bytes)
            .map_err(|e| format!("cannot decode {path}: {e}"))?;
        let rgb = dyn_img.to_rgb8();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let raw = rgb.into_raw();
        let mut d = vec![0.0f32; w * h * 3];
        for (o, v) in d.iter_mut().zip(raw.iter()) {
            *o = *v as f32;
        }
        Ok(Img { w, h, d })
    }

    pub fn save_png(&self, path: &str) -> Result<(), String> {
        let mut buf = vec![0u8; self.w * self.h * 3];
        for (o, v) in buf.iter_mut().zip(self.d.iter()) {
            *o = v.round().clamp(0.0, 255.0) as u8;
        }
        let img = image::RgbImage::from_raw(self.w as u32, self.h as u32, buf)
            .ok_or_else(|| "rgb buffer size mismatch".to_string())?;
        img.save(path).map_err(|e| format!("cannot save {path}: {e}"))
    }

    #[inline]
    pub fn at(&self, x: usize, y: usize) -> [f32; 3] {
        let i = (y * self.w + x) * 3;
        [self.d[i], self.d[i + 1], self.d[i + 2]]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, c: [f32; 3]) {
        let i = (y * self.w + x) * 3;
        self.d[i] = c[0];
        self.d[i + 1] = c[1];
        self.d[i + 2] = c[2];
    }
}

/// "rgb(r,g,b)" with per-channel rounding, like the python rgbf().
pub fn rgbf(c: [f32; 3]) -> String {
    format!("rgb({},{},{})", c[0].round() as i32, c[1].round() as i32, c[2].round() as i32)
}
