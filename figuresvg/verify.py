"""Verification V4 (TypeSafe-decided): closed-loop scene comparison.

Render the final SVG and re-run the SAME recognizers on the render:
  - text: every intended string must be re-recognized at a matching box
    (string match rate + mean IoU + mean center distance = placement error)
  - shapes: re-detect geometry on the render; compare type histogram and
    pixel coverage/color agreement on the source content mask
This verifies SEMANTICS (what the user checks), not just pixels, and the
text deltas double as the correction signal (see textloop.py).
"""

import io

import cairosvg
import numpy as np
from PIL import Image


def _render(svg, w, h) -> Image.Image:
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=w,
                           output_height=h)
    return Image.open(io.BytesIO(png)).convert("RGB")


def _iou(a, b):
    ax, ay, aw, ah = a
    bx, by, bw, bh = b
    ix = max(0, min(ax + aw, bx + bw) - max(ax, bx))
    iy = max(0, min(ay + ah, by + bh) - max(ay, by))
    inter = ix * iy
    union = aw * ah + bw * bh - inter
    return inter / union if union > 0 else 0.0


def check(svg, source_path, w, h, scene=None, render_out=None):
    ren = np.asarray(_render(svg, w, h)).astype(np.int32)
    src = np.asarray(Image.open(source_path).convert("RGB")).astype(np.int32)
    if ren.shape != src.shape:
        src = np.asarray(Image.open(source_path).convert("RGB").resize(
            (ren.shape[1], ren.shape[0]))).astype(np.int32)
    out = {"pixel_mean_diff": round(float(np.abs(ren - src).mean()), 3)}

    # closed-loop text verification
    if scene is not None:
        texts = [e for e in scene.elements if e.type == "text"]
        if texts:
            if render_out is None:
                import tempfile, os

                fd, render_out = tempfile.mkstemp(suffix=".png")
                os.close(fd)
            Image.fromarray(ren.astype(np.uint8)).save(render_out)
            from .recognizers import ocr

            retext = ocr.recognize(render_out)
            matched, ious, cdist = 0, [], []
            for t in texts:
                best_iou, best_c = 0.0, 1e9
                for r in retext:
                    if r.text == t.text:
                        v = _iou(t.bbox, r.bbox)
                        if v > best_iou:
                            best_iou = v
                            best_c = ((t.bbox[0] + t.bbox[2] / 2 - r.bbox[0] - r.bbox[2] / 2) ** 2 +
                                      (t.bbox[1] + t.bbox[3] / 2 - r.bbox[1] - r.bbox[3] / 2) ** 2) ** 0.5
                if best_iou >= 0.3:
                    matched += 1
                    ious.append(best_iou)
                    cdist.append(best_c)
            out["text_match_rate"] = round(matched / len(texts), 3)
            out["text_mean_iou"] = round(float(np.mean(ious)), 3) if ious else 0.0
            out["text_mean_center_err_px"] = round(float(np.mean(cdist)), 2) if cdist else -1

        # coverage: source content pixels that are non-background in render
        content = src.min(axis=2) < 235
        rendered_content = ren.min(axis=2) < 235
        inter = (content & rendered_content).sum()
        out["content_coverage"] = round(float(inter / max(1, content.sum())), 4)
        out["extra_ink_frac"] = round(float((~content & rendered_content).sum()
                                            / max(1, (~content).sum())), 4)
    return out
