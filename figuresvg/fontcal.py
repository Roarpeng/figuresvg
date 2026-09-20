"""Font calibration layer: pick the right font family / weight / size per
text by matching the rendered glyph INK SHAPE against the source bitmap.

Why: the closed-loop verifier showed rendered <text> was so far from the
original typography that even OCR misread it (match rate 0-29%). Root
causes: wrong family (everything was sans-serif) and imprecise size. This
module renders each OCR string in each candidate font with PIL (same
FreeType the renderers use), extracts the ink mask, aligns it to the
source text ink box, and keeps the (family, weight, size) with the best
IoU. Emits an editor-friendly font stack with metric-compatible
fallbacks (Liberation Sans ~ Arial, Liberation Serif ~ Times).
"""

import numpy as np
from PIL import Image, ImageDraw, ImageFont

FONT_DIRS = [
    "/usr/share/fonts/truetype/liberation2",
    "/usr/share/fonts/truetype/liberation",
    "/usr/share/fonts/truetype/dejavu",
    "/usr/share/fonts/truetype/wqy",
    "/usr/share/fonts/truetype/ubuntu",
]

# family -> (files normal/bold, SVG stack, cjk-capable)
CANDIDATES = {
    "Liberation Sans": ("LiberationSans-Regular.ttf",
                        "LiberationSans-Bold.ttf",
                        "'Liberation Sans', Arial, Helvetica, sans-serif",
                        False),
    "DejaVu Sans": ("DejaVuSans.ttf", "DejaVuSans-Bold.ttf",
                    "'DejaVu Sans', Verdana, sans-serif", False),
    "Liberation Serif": ("LiberationSerif-Regular.ttf",
                         "LiberationSerif-Bold.ttf",
                         "'Liberation Serif', 'Times New Roman', serif",
                         False),
    "DejaVu Serif": ("DejaVuSerif.ttf", "DejaVuSerif-Bold.ttf",
                     "'DejaVu Serif', Georgia, serif", False),
    "WenQuanYi Zen Hei": ("wqy-zenhei.ttc", "wqy-zenhei.ttc",
                          "'WenQuanYi Zen Hei', 'Noto Sans CJK SC', sans-serif",
                          True),
    "AR PL UMing CN": ("uming.ttc", "uming.ttc",
                       "'AR PL UMing CN', 'Noto Serif CJK SC', serif", True),
}

_font_cache = {}


def _load(family, bold):
    key = (family, bold)
    if key in _font_cache:
        return _font_cache[key]
    import os

    fname = CANDIDATES[family][1 if bold else 0]
    for d in FONT_DIRS:
        p = os.path.join(d, fname)
        if os.path.exists(p):
            _font_cache[key] = p
            return p
    _font_cache[key] = None
    return None


def _has_cjk(s):
    return any(ord(c) > 0x2E7F for c in (s or ""))


def _ink_mask(img_gray_or_rgba, thr=110):
    a = np.asarray(img_gray_or_rgba)
    if a.ndim == 3:
        if a.shape[2] == 4:
            return a[:, :, 3] > thr
        a = a.mean(axis=2)
    return a < thr


def _render_ink(text, font_path, size):
    font = ImageFont.truetype(font_path, int(size))
    # canvas with generous margin
    W = max(8, int(len(text) * size * 1.4) + 20)
    H = int(size * 2.6)
    img = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(img)
    d.text((6, int(size * 0.3)), text, font=font, fill=0)
    m = _ink_mask(img, 130)
    ys, xs = np.where(m)
    if len(ys) == 0:
        return None
    return m[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


def _source_ink(src, bbox, text_fill):
    x, y, w, h = bbox
    pad = 3
    crop = src[max(0, y - pad):y + h + pad, max(0, x - pad):x + w + pad]
    # text color known from OCR sampling: mask = pixels within 100 (max
    # channel) of the text color -- includes the anti-aliasing ring; the
    # previous "closer-to-text-than-bg" rule degenerated to 1px skeletons
    tf = np.array([int(text_fill[i:i + 2], 16) for i in (1, 3, 5)],
                  dtype=np.float32)
    d_text = np.abs(crop.astype(np.float32) - tf).max(axis=2)
    m = d_text < 100
    ys, xs = np.where(m)
    if len(ys) == 0:
        return None
    return m[ys.min():ys.max() + 1, xs.min():xs.max() + 1]


def _shape_iou(a, b):
    # compare at common shape (pad smaller onto larger), plus aspect penalty
    H = max(a.shape[0], b.shape[0])
    W = max(a.shape[1], b.shape[1])

    def pad(m):
        out = np.zeros((H, W), bool)
        y0 = (H - m.shape[0]) // 2
        x0 = (W - m.shape[1]) // 2
        out[y0:y0 + m.shape[0], x0:x0 + m.shape[1]] = m
        return out

    A, B = pad(a), pad(b)
    inter = (A & B).sum()
    union = (A | B).sum()
    iou = inter / union if union else 0.0
    ar = (a.shape[1] / max(1, a.shape[0])) / max(0.05, b.shape[1] / max(1, b.shape[0]))
    ar_pen = min(ar, 1 / ar)
    return iou * (0.6 + 0.4 * ar_pen)


def calibrate(scene, source_path):
    src = np.asarray(Image.open(source_path).convert("RGB"))
    texts = [e for e in scene.elements if e.type == "text"]
    n_cal = 0
    for t in texts:
        if getattr(t, "rotate", None):
            continue  # rotated text: ink metrics assume horizontal
        cjk = _has_cjk(t.text)
        fams = [f for f in CANDIDATES
                if CANDIDATES[f][3] == cjk and _load(f, False)]
        if not fams:
            continue
        target = _source_ink(src, t.bbox, t.fill or "#000000")
        if target is None or target.sum() < 8:
            continue
        base_fs = t.font_size or (t.bbox[3] / 0.8)
        best, bscore = None, -1.0
        for fam in fams:
            for bold in (False, True):
                path = _load(fam, bold)
                if not path:
                    continue
                for fs in (base_fs * s for s in (0.75, 0.85, 1.0, 1.15, 1.3)):
                    ink = _render_ink(t.text or "", path, max(6, fs))
                    if ink is None:
                        continue
                    sc = _shape_iou(target, ink)
                    if sc > bscore:
                        bscore, best = sc, (fam, bold, fs)
        if best:
            # ALWAYS apply the best-scoring font: an imperfect match still
            # beats the sans-serif default, which renders CJK garbled
            fam, bold, fs = best
            t.font_family = CANDIDATES[fam][2]
            t.font_weight = "bold" if bold else "normal"
            t.font_size = round(fs, 1)
            t.calib_score = round(max(0.0, bscore), 3)
            if bscore > 0.15:
                n_cal += 1
    return {"font_calibrated": n_cal, "fonts_total": len(texts),
            "calib_mean_score": round(float(np.mean(
                [t.calib_score for t in texts if t.calib_score])), 3)
            if any(t.calib_score for t in texts) else 0.0}
