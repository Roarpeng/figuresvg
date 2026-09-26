"""OCR recognizer: PaddleOCR -> text elements.

Prefers PaddleOCR 3.x (PP-OCRv5, much stronger, multilingual) with
`enable_mkldnn=False` -- the onednn/PIR execution path crashes on this
platform (ConvertPirAttribute2RuntimeAttribute NotImplementedError).
Falls back to the 2.7.x classic API if 3.x is unavailable.

Text COLOR is sampled as the darkest pixel inside each box on the source
(figure text is dark-on-light).
"""

import numpy as np
from PIL import Image

from ..scene import Element

_OCR = None
_API = None  # "3" or "2"


def _engine():
    global _OCR, _API
    if _OCR is not None:
        return _OCR
    import paddleocr
    from paddleocr import PaddleOCR

    ver = getattr(paddleocr, "__version__", "2")
    if ver.startswith("3"):
        # 3.x (PP-OCRv5/v6): disable onednn/PIR execution path that crashes
        # on this platform; predict() returns axis-aligned rec_boxes
        _OCR = PaddleOCR(lang="en", enable_mkldnn=False)
        _API = "3"
    else:
        _OCR = PaddleOCR(lang="en", use_angle_cls=False, show_log=False)
        _API = "2"
    return _OCR


import math


def _items_3(path):
    res = _engine().predict(path)[0]
    out = []
    texts = res.get("rec_texts", [])
    boxes = res.get("rec_boxes", [])
    polys = res.get("rec_polys", [])
    scores = res.get("rec_scores", [])
    for i, (t, s) in enumerate(zip(texts, scores)):
        rot = 0.0
        h_poly = None
        if i < len(polys) and polys[i] is not None and len(polys[i]) >= 4:
            p = polys[i]
            xs = [pt[0] for pt in p]
            ys = [pt[1] for pt in p]
            x0, y0, x1, y1 = int(min(xs)), int(min(ys)), int(max(xs)), int(max(ys))
            # baseline direction from the top edge (p0->p1): the quad is
            # ordered, so this is the text direction even for rotated lines
            dx = p[1][0] - p[0][0]
            dy = p[1][1] - p[0][1]
            ang = math.degrees(math.atan2(dy, dx))
            if abs(ang) > 180.0:
                ang += 360.0
            if abs(ang) > 90.0:
                ang = ang - 180.0 if ang > 0 else ang + 180.0
            if abs(ang) >= 6.0:
                rot = round(ang, 1)
            # perpendicular height of the quad (font-size basis)
            ex, ey = p[3][0] - p[0][0], p[3][1] - p[0][1]
            h_poly = math.hypot(ex, ey)
        else:
            x0, y0, x1, y1 = (int(v) for v in boxes[i][:4])
        out.append((x0, y0, x1, y1, t, float(s), rot, h_poly))
    return out


def _items_2(path):
    result = _engine().ocr(path, cls=False)
    out = []
    for it in (result[0] if result else []):
        box, (txt, score) = it
        xs = [p[0] for p in box]
        ys = [p[1] for p in box]
        out.append((int(min(xs)), int(min(ys)), int(max(xs)), int(max(ys)),
                    txt, float(score), 0.0, None))
    return out


def text_color(img: np.ndarray, x0, y0, x1, y1) -> str:
    crop = img[max(0, y0):y1, max(0, x0):x1]
    if crop.size == 0:
        return "#000000"
    lum = crop.mean(axis=2)
    i = int(np.argmin(lum))
    c = crop.reshape(-1, 3)[i]
    return "#{:02x}{:02x}{:02x}".format(*[int(v) for v in c])


def _is_plausible_text(bbox, text, W, H):
    """Reject OCR hallucinations: a lone glyph covering half the canvas is
    a misread shape (stem read as 'd', tail read as a CJK char), not text.
    Real figure text is small relative to the canvas and multi-glyph or
    plausibly placed."""
    x, y, w, h = bbox
    # giant single char = hallucination
    if h > H * 0.35 or w > W * 0.5:
        return False
    # single char with extreme aspect
    if len(text or "") <= 2 and (w / max(1, h) > 4 or h / max(1, w) > 4):
        return False
    # tiny fragment
    if w < 8 and h < 8:
        return False
    return True


def recognize(path: str) -> list:
    """Return Element list (type=text) for the image at `path`."""
    img = np.asarray(Image.open(path).convert("RGB"))
    H, W = img.shape[:2]
    _engine()  # set _API before dispatch
    items = _items_3(path) if _API == "3" else _items_2(path)
    # filter hallucinations before further processing
    items = [it for it in items if _is_plausible_text(it[:4], it[4], W, H)]
    out = []
    for n, item in enumerate(items, 1):
        x0, y0, x1, y1, txt, score, rot, h_poly = item
        h = y1 - y0
        fs_basis = h_poly if (h_poly and h_poly > 3) else h
        out.append(
            Element(
                id=f"text_{n:03d}",
                type="text",
                bbox=[x0, y0, x1 - x0, max(1, h)],
                source="ocr",
                confidence=score,
                fill=text_color(img, x0, y0, x1, y1),
                text=txt,
                # descent-aware: strings with descenders (g j p q y , ;)
                # occupy ~0.95 of the box, cap-only ~0.74
                font_size=round(fs_basis / (0.95 if any(c in "gjpqyQ,;" for c in (txt or "")) else 0.74), 1),
                font_family="sans-serif",
                rotate=rot if rot else None,
            )
        )
    return out
