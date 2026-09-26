"""Hybrid Fusion Pipeline: VLM provides SEMANTICS, CV provides MEASUREMENTS.

The correct division of labor:
  VLM (semantic layer):
    - WHAT text says (species names, labels, values) — even at tiny sizes
    - WHAT the image depicts (chart type, structural pattern)
    - Component ROLES (petal, eye, axis, legend, species-name)

  CV (measurement layer):
    - WHERE each text line sits (row analysis, exact y-coordinates)
    - WHAT exact colors each region has (per-pixel sampling)
    - WHAT geometry each shape has (connected components, contours)
    - HOW MANY objects there are and their exact positions

  Fusion:
    - Text: VLM content + CV positions → matched by ordinal/spatial logic
    - Shapes: VLM structure + CV geometry → role-labeled exact primitives
    - Colors: always CV (exact pixel sampling, never VLM estimate)
"""

import base64
import io
import json
import os
import urllib.request

import numpy as np
import cv2
from PIL import Image

API_URL = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"

SEMANTIC_PROMPT = """Analyze this image. I need you to extract SEMANTIC information that my CV system cannot:

1. TEXT CONTENT: List every piece of text you can read, in reading order (top to bottom, left to right). For each: the exact text string and what it labels (species name, axis value, legend, title, etc.)

2. STRUCTURE TYPE: What kind of figure/image is this? (phylogenetic tree, bar chart, logo, portrait, flower, diagram, etc.)

3. COMPONENT ROLES: For the main visual elements, describe what each IS (not position — my CV has that):
   - "colored horizontal bands" → species classification strips
   - "small circles at branch tips" → bootstrap markers
   - "gradient bar on right" → colorbar legend
   
4. RELATIONSHIPS: Which text labels which visual element?

Output ONLY this JSON (no fences):
{
  "image_type": "phylogenetic_tree|bar_chart|logo|portrait|flower|...",
  "texts": [
    {"content": "exact text string", "role": "species_name|axis_label|legend|title|value", "order": 1}
  ],
  "visual_components": [
    {"role": "species_bands|markers|tree_branches|colorbar|legend", 
     "description": "what this visual element is"}
  ],
  "text_to_visual": [
    {"text_order": 1, "visual_role": "species_bands", "relationship": "labels_row_1"}
  ]
}"""


def vlm_semantic(image_path: str) -> dict:
    """Get semantic understanding from VLM."""
    key = os.environ.get("DASHSCOPE_API_KEY")
    model = os.environ.get("FIGURESVG_VLM_MODEL", "qwen3.8-omni-flash")

    img = Image.open(image_path)
    if max(img.size) > 1000:
        ratio = 1000 / max(img.size)
        img = img.resize((int(img.width * ratio), int(img.height * ratio)))
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=85)
    b64 = base64.b64encode(buf.getvalue()).decode()

    body = json.dumps({
        "model": model,
        "messages": [
            {"role": "user", "content": [
                {"type": "image_url",
                 "image_url": {"url": f"data:image/jpeg;base64,{b64}"}},
                {"type": "text", "text": SEMANTIC_PROMPT},
            ]},
        ],
        "max_tokens": 3000,
        "temperature": 0.1,
    }).encode()

    req = urllib.request.Request(API_URL, data=body, method="POST",
                                 headers={
                                     "Authorization": f"Bearer {key}",
                                     "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as r:
        resp = json.load(r)

    content = resp["choices"][0]["message"]["content"].strip()
    if content.startswith("```"):
        content = content.split("\n", 1)[1].rsplit("```", 1)[0].strip()
    return json.loads(content)


def cv_text_lines(image_path: str) -> list:
    """CV measurement: find text line positions (y-coordinates) even when
    OCR can't read the content. Uses row-profile analysis on dark pixels."""
    img = np.asarray(Image.open(image_path).convert("RGB"))
    H, W = img.shape[:2]
    lum = img.mean(axis=2)

    # text = dark pixels that are NOT part of large shapes
    dark = lum < 128
    # remove large connected shapes (tree lines, bars, etc.)
    n, lab, stats, _ = cv2.connectedComponentsWithStats(
        dark.astype(np.uint8), 8)
    text_mask = np.zeros_like(dark)
    for i in range(1, n):
        x, y, w, h, area = stats[i]
        # text characters: small height (5-50px), not too wide (< 60% canvas)
        if h <= 60 and w < W * 0.6 and area >= 10:
            text_mask |= (lab == i)

    # row profile: find text lines
    row_profile = text_mask.sum(axis=1)
    threshold = 2
    in_line = row_profile > threshold
    lines = []
    start = None
    for y in range(H):
        if in_line[y] and start is None:
            start = y
        elif not in_line[y] and start is not None:
            if y - start >= 4:  # minimum line height
                # get x extent
                seg = text_mask[start:y]
                cols = np.where(seg.any(axis=0))[0]
                if len(cols) > 3:
                    lines.append({
                        "y": start, "h": y - start,
                        "x": int(cols.min()), "w": int(cols.max() - cols.min()),
                        "dark_px": int(seg.sum()),
                    })
            start = None
    if start is not None and H - start >= 4:
        seg = text_mask[start:H]
        cols = np.where(seg.any(axis=0))[0]
        if len(cols) > 3:
            lines.append({
                "y": start, "h": H - start,
                "x": int(cols.min()), "w": int(cols.max() - cols.min()),
                "dark_px": int(seg.sum()),
            })
    return lines


def cv_color_regions(image_path: str, max_regions=20) -> list:
    """CV measurement: dominant color regions with exact positions."""
    img = np.asarray(Image.open(image_path).convert("RGB")).astype(float)
    H, W = img.shape[:2]
    lum = img.mean(axis=2)
    mx, mn = img.max(axis=2), img.min(axis=2)
    spread = mx - mn

    colored = (spread > 20) & ~((spread < 25) & (lum < 150))
    n, lab, stats, cent = cv2.connectedComponentsWithStats(
        colored.astype(np.uint8), 8)
    regions = []
    for i in range(1, n):
        x, y, w, h, area = stats[i]
        if area < H * W * 0.001:
            continue
        mask = lab == i
        pixels = img[mask]
        med = np.median(pixels, axis=0)
        regions.append({
            "bbox": [int(x), int(y), int(w), int(h)],
            "area": int(area),
            "color": "#{:02x}{:02x}{:02x}".format(
                int(med[0]), int(med[1]), int(med[2])),
            "centroid": [round(float(cent[i][0]), 1), round(float(cent[i][1]), 1)],
        })
    regions.sort(key=lambda r: -r["area"])
    return regions[:max_regions]


def fuse(vlm_result: dict, cv_lines: list, cv_regions: list,
         image_size: tuple) -> dict:
    """Combine VLM semantics with CV measurements."""
    W, H = image_size
    fused = {
        "image_type": vlm_result.get("image_type", "unknown"),
        "texts": [],
        "regions": cv_regions,
        "source": "hybrid_v1",
    }

    # 1. Match VLM text content to CV text line positions
    vlm_texts = vlm_result.get("texts", [])
    # sort both by reading order (top-to-bottom)
    vlm_texts.sort(key=lambda t: t.get("order", 0))
    cv_lines_sorted = sorted(cv_lines, key=lambda l: l["y"])

    # If counts roughly match, pair by order
    if len(vlm_texts) > 0 and len(cv_lines_sorted) > 0:
        if abs(len(vlm_texts) - len(cv_lines_sorted)) <= max(
                len(vlm_texts), len(cv_lines_sorted)) * 0.3:
            # direct pairing by order
            for i, vt in enumerate(vlm_texts):
                if i < len(cv_lines_sorted):
                    cl = cv_lines_sorted[i]
                    fused["texts"].append({
                        "content": vt.get("content", ""),
                        "role": vt.get("role", "unknown"),
                        "bbox": [cl["x"], cl["y"], cl["w"], cl["h"]],
                        "source": "vlm_content+cv_position",
                    })
        else:
            # mismatch: attach VLM texts with approximate positions
            for vt in vlm_texts:
                fused["texts"].append({
                    "content": vt.get("content", ""),
                    "role": vt.get("role", "unknown"),
                    "bbox": None,
                    "source": "vlm_only",
                })
            # attach unmatched CV lines as "unreadable text regions"
            for cl in cv_lines_sorted:
                fused.setdefault("unmatched_text_regions", []).append(cl)

    # 2. Attach VLM role descriptions to CV color regions
    for region in fused["regions"]:
        region["role"] = "color_region"  # default
    vlm_comps = vlm_result.get("visual_components", [])
    for i, vc in enumerate(vlm_comps):
        if i < len(fused["regions"]):
            fused["regions"][i]["role"] = vc.get("role", "color_region")
            fused["regions"][i]["description"] = vc.get("description", "")

    return fused


def convert(image_path: str, svg_out: str) -> dict:
    """Full hybrid pipeline: VLM semantic + CV measurement → fused → SVG."""
    img = Image.open(image_path)
    W, H = img.size

    # Phase 1: VLM semantics (what is it, what does the text say)
    print("  [VLM] extracting semantics...")
    try:
        vlm_result = vlm_semantic(image_path)
        print(f"  [VLM] type={vlm_result.get('image_type')}, "
              f"{len(vlm_result.get('texts', []))} texts, "
              f"{len(vlm_result.get('visual_components', []))} components")
    except Exception as e:
        print(f"  [VLM] failed: {e}")
        vlm_result = {"image_type": "unknown", "texts": [],
                      "visual_components": []}

    # Phase 2: CV measurements (exact positions, colors, text lines)
    print("  [CV] measuring text lines...")
    cv_lines = cv_text_lines(image_path)
    print(f"  [CV] {len(cv_lines)} text lines detected")

    print("  [CV] measuring color regions...")
    cv_regions = cv_color_regions(image_path)
    print(f"  [CV] {len(cv_regions)} color regions")

    # Phase 3: Fusion
    print("  [FUSE] combining...")
    fused = fuse(vlm_result, cv_lines, cv_regions, (W, H))
    matched = sum(1 for t in fused["texts"] if t.get("bbox"))
    print(f"  [FUSE] {matched}/{len(fused['texts'])} texts matched to positions")

    # Phase 4: Generate SVG
    svg = generate_svg(fused, W, H)
    with open(svg_out, "w") as f:
        f.write(svg)

    # Phase 5: Verify
    stats = _verify(svg, image_path, W, H)
    return {**stats,
            "type": fused["image_type"],
            "texts_matched": matched,
            "texts_total": len(fused["texts"]),
            "regions": len(fused["regions"])}


def generate_svg(fused: dict, W: int, H: int) -> str:
    """Generate structured SVG from fused data."""
    from xml.sax.saxutils import escape
    out = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<svg width="{W / 300 * 25.4:.4f}mm" height="{H / 300 * 25.4:.4f}mm" '
        f'viewBox="0 0 {W} {H}" xmlns="http://www.w3.org/2000/svg">',
        f'<rect id="background" width="{W}" height="{H}" fill="#ffffff"/>',
        '<g id="layer1_objects">',
    ]

    # color regions (from CV, with VLM role labels)
    for r in fused.get("regions", []):
        x, y, w, h = r["bbox"]
        fill = r["color"]
        role = r.get("role", "region")
        out.append(
            f'<rect id="{role}_{x}_{y}" x="{x}" y="{y}" width="{w}" '
            f'height="{h}" fill="{fill}" data-role="{role}"/>')

    out.append('</g>')
    out.append('<g id="layer2_texts">')

    # texts (VLM content + CV positions)
    for t in fused.get("texts", []):
        content = t.get("content", "")
        if not content or not t.get("bbox"):
            continue
        x, y, w, h = t["bbox"]
        fs = int(h * 1.3)  # approximate from line height
        role = t.get("role", "text")
        out.append(
            f'<text id="text_{y}" x="{x}" y="{y + h}" '
            f'font-size="{fs}" font-family="serif" fill="#333" '
            f'data-role="{role}" data-source="{t.get("source", "")}">'
            f'<tspan>{escape(content)}</tspan></text>')

    out.append('</g>')
    out.append('</svg>')
    return "\n".join(out)


def _verify(svg: str, source_path: str, W: int, H: int) -> dict:
    try:
        import cairosvg
        png = cairosvg.svg2png(bytestring=svg.encode(),
                               output_width=W, output_height=H)
        ren = np.asarray(Image.open(io.BytesIO(png)).convert("RGB")).astype(int)
        src = np.asarray(Image.open(source_path).convert("RGB")).astype(int)
        diff = float(np.abs(ren - src).mean())
        coverage = float(
            ((src.min(axis=2) < 235) & (ren.min(axis=2) < 235)).sum() /
            max(1, (src.min(axis=2) < 235).sum()))
        return {"pixel_diff": round(diff, 2), "coverage": round(coverage, 4)}
    except Exception as e:
        return {"verify_error": str(e)[:80]}
