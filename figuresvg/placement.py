"""Ink-alignment placement (S5): compute each text's exact SVG anchor from
the glyph ink box, replacing iterative OCR-loop positioning.

The font is already chosen by fontcal. PIL renders the string at the chosen
family/size with a known anchor; the ink offset from the anchor tells us
exactly where the glyphs sit relative to (x, baseline). Matching the
rendered ink box onto the SOURCE ink box gives x/y directly -- one shot,
no iteration.
"""

import numpy as np
from PIL import Image, ImageDraw, ImageFont

from .fontcal import CANDIDATES, _load


def _family_file(family_stack: str, bold: bool):
    """Map an emitted font stack back to a loadable font file."""
    for fam, (_n, _b, stack, _c) in CANDIDATES.items():
        if stack == family_stack or fam in (family_stack or ""):
            p = _load(fam, bold)
            if p:
                return p
    return None


def place(scene, source_path: str) -> dict:
    src = np.asarray(Image.open(source_path).convert("RGB"))
    texts = [e for e in scene.elements if e.type == "text"]
    n = 0
    for t in texts:
        if getattr(t, "rotate", None):
            continue  # rotated text placed from its quad directly
        path = _family_file(t.font_family, (t.font_weight or "normal") == "bold")
        if not path:
            continue
        fs = int(max(6, t.font_size or 12))
        try:
            font = ImageFont.truetype(path, fs)
        except Exception:
            continue
        # ink box of the rendered string, anchored at left-baseline (ls)
        W = max(16, int(len(t.text or "") * fs * 1.6) + 20)
        H = int(fs * 3)
        img = Image.new("L", (W, H), 255)
        d = ImageDraw.Draw(img)
        d.text((4, int(H * 0.66)), t.text or "", font=font, fill=0,
               anchor="ls")
        m = np.asarray(img) < 130
        ys, xs = np.where(m)
        if len(ys) == 0:
            continue
        ax, ay = 4, int(H * 0.66)  # anchor position in canvas
        ink_left = xs.min() - ax      # >=0: ink starts right of anchor
        ink_bottom = ys.max() - ay    # >0: ink descends below baseline

        # source ink box (absolute coords), same rule as fontcal
        from .fontcal import _source_ink

        sink = _source_ink(src, t.bbox, t.fill or "#000000")
        if sink is None:
            continue
        x0, y0, w0, h0 = t.bbox
        pad = 3
        # absolute ink box of source = bbox crop origin + ink min within crop
        crop = src[max(0, y0 - pad):y0 + h0 + pad, max(0, x0 - pad):x0 + w0 + pad]
        # recompute mins absolutely
        tf = np.array([int((t.fill or "#000000")[i:i + 2], 16) for i in (1, 3, 5)],
                      dtype=np.float32)
        d_text = np.abs(crop.astype(np.float32) - tf).max(axis=2)
        mm = d_text < 100
        ys2, xs2 = np.where(mm)
        if len(ys2) == 0:
            continue
        abs_ink_left = max(0, x0 - pad) + xs2.min()
        abs_ink_bottom = max(0, y0 - pad) + ys2.max()

        # place anchor so rendered ink lands exactly on source ink
        t.dx = abs_ink_left - ink_left - x0
        t.dy = abs_ink_bottom - ink_bottom - int(y0 + h0 * 0.78)
        n += 1
    return {"ink_placed": n, "texts_total": len(texts)}
