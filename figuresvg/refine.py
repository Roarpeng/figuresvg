"""Refine stage (the bitmap-comparison step): for every text element,
render BOTH candidates -- the traced glyphs (fidelity) and the re-rendered
<text> (editable) -- against the source bitmap inside the element's box,
and keep whichever is closer. Elements passing the threshold as <text>
become editable; failing ones fall back to their traced glyphs. Everything
else of the SVG is untouched, so overall fidelity tracks the pixel engine.
"""

import io

import numpy as np
import cairosvg
from PIL import Image

from . import svggen
from .scene import Scene

# a text box is accepted as editable <text> if its mean deviation from the
# source stays under this; above it the traced glyphs win
TEXT_ACCEPT_DIFF = 10.0
PAD = 4


def _render(svg: str, w: int, h: int) -> np.ndarray:
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=w, output_height=h)
    return np.asarray(Image.open(io.BytesIO(png)).convert("RGB")).astype(int)


def _box_diff(a: np.ndarray, b: np.ndarray, bbox) -> float:
    x, y, w, h = bbox
    x0, y0 = max(0, x - PAD), max(0, y - PAD)
    x1, y1 = min(a.shape[1], x + w + PAD), min(a.shape[0], y + h + PAD)
    if x1 <= x0 or y1 <= y0:
        return 0.0
    return float(np.abs(a[y0:y1, x0:x1] - b[y0:y1, x0:x1]).mean())


def refine(scene: Scene, source_path: str) -> tuple[Scene, dict]:
    """Return (scene with per-text choices applied, stats)."""
    src = np.asarray(Image.open(source_path).convert("RGB")).astype(np.int32)
    h, w = src.shape[:2]

    texts = [e for e in scene.elements if e.type == "text"]
    # texts without engine-traced candidates get one synthesized straight
    # from the source bitmap (binarized crop, binary trace) so EVERY text
    # has both variants and the bitmap comparison can always choose
    for t in texts:
        if not t.trace:
            cand = _synth_trace(src, t)
            if cand is not None:
                t.trace = [cand]

    stats = {"texts_total": len(texts), "texts_editable": 0,
             "texts_traced": 0}
    if not texts:
        return scene, stats

    # variant A: every text -> traced glyphs (pixel mode)
    scene_a = _with_variant(scene, "trace")
    # variant B: every text -> <text> (fully editable mode)
    scene_b = _with_variant(scene, "text")
    ra = _render(svggen.generate(scene_a), w, h)
    rb = _render(svggen.generate(scene_b), w, h)

    for t in texts:
        if not t.trace:
            t.editable = True  # no glyph candidate exists at all
            stats["texts_editable"] += 1
            continue
        da = _box_diff(ra, src, t.bbox)
        db = _box_diff(rb, src, t.bbox)
        if db <= TEXT_ACCEPT_DIFF and db <= da + 6.0:
            t.editable = True
            stats["texts_editable"] += 1
        else:
            t.editable = False
            stats["texts_traced"] += 1
    return scene, stats


def _synth_trace(src: np.ndarray, t):
    """Trace the text's own pixels from the source as a path candidate."""
    import vtracer
    import tempfile
    import os

    x, y, bw, bh = t.bbox
    pad = 6
    x0, y0 = max(0, x - pad), max(0, y - pad)
    x1, y1 = min(src.shape[1], x + bw + pad), min(src.shape[0], y + bh + pad)
    if x1 <= x0 or y1 <= y0:
        return None
    crop = src[y0:y1, x0:x1]
    lum = crop.mean(axis=2)
    # dark text on light bg; keep AA soft via 160 threshold
    mask = (lum < 160).astype(np.uint8) * 255
    if mask.sum() < 20:
        return None
    img = Image.fromarray(np.stack([255 - mask] * 3, axis=2))
    with tempfile.TemporaryDirectory() as td:
        pin, pout = os.path.join(td, "in.png"), os.path.join(td, "out.svg")
        img.save(pin)
        vtracer.convert_image_to_svg_py(pin, pout, colormode="binary",
                                        mode="spline", filter_speckle=1,
                                        color_precision=8,
                                        layer_difference=16,
                                        corner_threshold=60,
                                        length_threshold=4.0,
                                        max_iterations=10,
                                        splice_threshold=45,
                                        path_precision=1)
        d_parts = [m for m in __import__("re").findall(r' d="([^"]+)"', open(pout).read())]
    if not d_parts:
        return None
    from .scene import Element

    fill = t.fill or "#000000"
    return Element(
        id=t.id + "_glyph", type="path", bbox=t.bbox,
        source="synth-trace", fill=fill,
        d=" ".join(d_parts),
        transform=f"translate({x0},{y0})",
    )


def _with_variant(scene: Scene, variant: str) -> Scene:
    """Copy of the scene where every traced-backed text uses `variant`."""
    import copy

    sc = copy.deepcopy(scene)
    for e in sc.elements:
        if e.type == "text" and e.trace:
            e.editable = (variant == "text")
    return sc
