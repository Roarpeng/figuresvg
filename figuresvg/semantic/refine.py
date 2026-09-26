"""CV Refinement: takes the VLM's structural understanding and replaces
its approximate coordinates/colors with exact CV measurements.

VLM says: "8 petals, radial, center ~(110,90)"
CV provides: exact center (110,95), exact petal positions (measured from
the bitmap), exact colors (sampled per-region)

The refined output has VLM's UNDERSTANDING + CV's PRECISION.
"""

import json
import math

import numpy as np
from PIL import Image
import cv2


def refine(understanding: dict, image_path: str) -> dict:
    """Replace VLM's approximate values with CV-measured exact values."""
    img = np.asarray(Image.open(image_path).convert("RGB")).astype(float)
    H, W = img.shape[:2]

    refined = dict(understanding)
    comps = refined.get("components", [])

    for comp in comps:
        pos = comp.get("position", [0, 0])
        size = comp.get("size", [10, 10])

        # skip text (OCR already provides exact)
        if comp.get("shape") == "text":
            continue

        # 1. refine color: sample the image at the component's position
        cx, cy = int(pos[0]), int(pos[1])
        cx = max(0, min(W - 1, cx))
        cy = max(0, min(H - 1, cy))
        # sample a small region
        sval = size[0] if isinstance(size, list) else (size.get("r", 10) if isinstance(size, dict) else size)
        r = max(3, min(15, int(sval / 4)))
        y0, y1 = max(0, cy - r), min(H, cy + r)
        x0, x1 = max(0, cx - r), min(W, cx + r)
        region = img[y0:y1, x0:x1]
        if region.size > 0:
            # median color (robust to AA/edges)
            med = np.median(region.reshape(-1, 3), axis=0)
            comp["fill"] = "#{:02x}{:02x}{:02x}".format(
                int(med[0]), int(med[1]), int(med[2]))

        # 2. refine position/size: find the connected region matching
        # the component's color near its stated position
        _refine_geometry(comp, img, W, H)

    # 3. refine arrangements: snap radial centers to measured centroid
    for arr in refined.get("arrangements", []):
        if arr.get("type") == "radial" and arr.get("center"):
            center = arr["center"]
            # find the color centroid of the largest colored region
            members = [c for c in comps
                       if c.get("role") in arr.get("member_roles", [])]
            if members:
                fill = members[0].get("fill", "#ff69b4")
                mask = _color_mask(img, fill)
                if mask is not None:
                    ys, xs = np.where(mask)
                    if len(ys) > 10:
                        # weighted centroid
                        arr["center"] = [
                            round(float(xs.mean()), 1),
                            round(float(ys.mean()), 1),
                        ]

    return refined


def _color_mask(img, hex_color):
    """Binary mask of pixels close to the given hex color."""
    try:
        r = int(hex_color[1:3], 16)
        g = int(hex_color[3:5], 16)
        b = int(hex_color[5:7], 16)
    except (ValueError, IndexError):
        return None
    target = np.array([r, g, b], dtype=float)
    dist = np.abs(img - target).max(axis=2)
    return dist < 40


def _refine_geometry(comp, img, W, H):
    """Snap a component's position/size to the actual colored region."""
    fill = comp.get("fill", "")
    if not fill.startswith("#"):
        return

    mask = _color_mask(img, fill)
    if mask is None:
        return

    # find the connected region nearest to the component's stated position
    pos = comp.get("position", [0, 0])
    cx, cy = int(pos[0]), int(pos[1])

    # search in a window around the stated position
    win = 50
    y0, y1 = max(0, cy - win), min(H, cy + win)
    x0, x1 = max(0, cx - win), min(W, cx + win)
    local = mask[y0:y1, x0:x1]

    # find largest CC in the window
    n, lab, stats, cent = cv2.connectedComponentsWithStats(
        local.astype(np.uint8), 8)
    if n <= 1:
        return

    best = max(range(1, n), key=lambda i: stats[i, 4])
    st = stats[best]
    if st[4] < 4:  # too small
        return

    # convert back to global coords
    gx = x0 + st[0]
    gy = y0 + st[1]
    gw = st[2]
    gh = st[3]

    shape = comp.get("shape", "")
    if shape in ("circle", "ellipse"):
        comp["position"] = [round(gx + gw / 2, 1), round(gy + gh / 2, 1)]
        if shape == "circle":
            r = max(gw, gh) / 2
            if isinstance(comp.get("size"), dict):
                comp["size"]["r"] = round(r, 1)
            else:
                comp["size"] = [round(gw, 1), round(gh, 1)]
        else:
            comp["size"] = [round(gw, 1), round(gh, 1)]
    elif shape == "rect":
        comp["position"] = [round(gx + gw / 2, 1), round(gy + gh / 2, 1)]
        comp["size"] = [round(gw, 1), round(gh, 1)]
