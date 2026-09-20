"""Fusion (pure code, no AI): resolve OCR-vs-geometry overlaps and build
label relations.

A geometry element that is mostly inside a text box is a TRACED GLYPH of
that text -- it is not dropped but attached to the text element as the
fidelity candidate (`trace` field). The refine stage later decides, per
text, between the traced glyphs (pixel-faithful) and the re-rendered
<text> (editable) by comparing both renders against the source bitmap.
"""

from .scene import Scene, Relation

OVERLAP_FRAC = 0.55
SIZE_RATIO = 4.0
LABEL_RADIUS = 160.0


def _overlap(a, b) -> float:
    ax, ay, aw, ah = a
    bx, by, bw, bh = b
    ix = max(0, min(ax + aw, bx + bw) - max(ax, bx))
    iy = max(0, min(ay + ah, by + bh) - max(ay, by))
    return ix * iy


def fuse(scene: Scene, texts: list) -> Scene:
    """Attach glyph geometry to its text; keep non-text geometry; relate."""
    kept = []
    for e in scene.elements:
        e_area = max(1, e.bbox[2] * e.bbox[3])
        attached = False
        for t in texts:
            t_area = t.bbox[2] * t.bbox[3]
            if e_area > SIZE_RATIO * t_area:
                continue  # big region: text sits ON it, keep both
            if _overlap(e.bbox, t.bbox) >= OVERLAP_FRAC * e_area:
                # traced glyph of this text -> fidelity candidate
                t.trace.append(e)
                attached = True
                break
        if not attached:
            kept.append(e)
    scene.elements = kept + texts

    # labels relations: nearest decent-sized shape within radius
    shapes = [e for e in kept if e.bbox[2] * e.bbox[3] > 400]
    for t in texts:
        tx, ty = t.bbox[0] + t.bbox[2] / 2, t.bbox[1] + t.bbox[3] / 2
        best, bd = None, LABEL_RADIUS
        for s in shapes:
            sx = s.bbox[0] + s.bbox[2] / 2
            sy = s.bbox[1] + s.bbox[3] / 2
            d = ((tx - sx) ** 2 + (ty - sy) ** 2) ** 0.5
            if d < bd:
                best, bd = s, d
        if best:
            scene.relations.append(Relation(t.id, best.id, "labels"))
    return scene
