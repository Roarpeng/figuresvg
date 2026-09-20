"""Pipeline v4 (structure-first, per the user's architecture):

  1. Text Engine    : PaddleOCR 3.x -> text + bbox (+ sampled color)
  2. Geometry Engine: Rust palette engine, scene mode -> typed primitives
                      (rect/circle/line/polygon/path) + colors + coords
  3. Fusion         : remove traced-glyph geometry under OCR boxes,
                      build label relations
  4. Bitmap check   : per-text font metrics optimized against the bitmap
                      (targeted modification, never reversion to outlines);
                      final render verified, worst regions reported
  5. Output         : structured SVG -- every text a real <text>, every
                      shape a typed primitive
"""

import json
import os

from PIL import Image

from . import fuse, svggen, textloop, textmetrics, verify
from .recognizers import geometry, ocr


def convert(
    image_path: str,
    svg_out: str,
    scene_out: str | None = None,
    recolor: list | None = None,
    skip_ocr: bool = False,
) -> dict:
    stats = {}

    # 2. geometry: typed primitives
    scene_json = geometry.recognize_raw(image_path, recolor=recolor)
    scene = geometry.Scene.from_engine_json(scene_json)
    stats["scene_elements"] = len(scene.elements)
    from collections import Counter

    stats["primitive_types"] = dict(Counter(e.type for e in scene.elements))

    # 1. text
    texts = [] if skip_ocr else ocr.recognize(image_path)
    stats["ocr_texts"] = len(texts)

    # 3. fusion
    if texts:
        scene = fuse.fuse(scene, texts)
        from . import legend

        stats.update(legend.extract(scene))
        # 3a. font calibration: family/weight/size from ink-shape match
        from . import fontcal

        stats.update(fontcal.calibrate(scene, image_path))
        # 3b. ink-alignment placement: exact anchor from glyph ink box
        from . import placement

        stats.update(placement.place(scene, image_path))
    stats["elements_final"] = len(scene.elements)
    stats["relations"] = len(scene.relations)

    # 5. compile
    from . import compiler2

    tdicts = []
    for t in scene.elements:
        if t.type == "text":
            tdicts.append({k: getattr(t, k) for k in
                           ("id", "bbox", "text", "font_size", "font_family",
                            "font_weight", "fill", "dx", "dy", "baseline_dy",
                            "rotate", "text_anchor", "tags")})
    # stroke-group tags (errbar_*) live on Python scene elements; inject
    # them into the engine scene copy by id before compiling
    tag_by_id = {e.id: e.tags for e in scene.elements if e.tags}
    for el in scene_json.get("elements", []):
        if el.get("id") in tag_by_id:
            el["tags"] = tag_by_id[el["id"]]
    final = compiler2.generate(scene_json, texts=tdicts)

    # 4b. verification + targeted problem report
    with Image.open(image_path) as im:
        w, h = im.size
    vstats = verify.check(final, image_path, w, h, scene=scene)
    stats.update(vstats)

    with open(svg_out, "w") as f:
        f.write(final)
    if scene_out:
        with open(scene_out, "w") as f:
            json.dump(scene.to_json(), f, indent=1, ensure_ascii=False)
    stats["svg_bytes"] = os.path.getsize(svg_out)
    stats["svg_text_elements"] = final.count("<text")
    return stats
