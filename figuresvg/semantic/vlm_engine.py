"""VLM Semantic Engine: interprets CV-extracted facts into a structured
Scene Understanding, then the SVG Generator draws from that understanding.

Architecture (user-defined):
  CV layer (measurements) → VLM (understanding) → Parametric SVG drawing

The VLM receives:
  1. The image (base64) — sees the overall structure
  2. CV facts (JSON) — exact colors, positions, text from the pipeline
It returns a structured understanding that the generator turns into SVG.
"""

import base64
import json
import os
import urllib.request

API_URL = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
MODEL = os.environ.get("FIGURESVG_VLM_MODEL", "qwen3.8-omni-flash")

SYSTEM_PROMPT = """You are a visual structure analyst. Given an image and its CV-extracted measurements, produce a JSON "scene understanding" that describes HOW to draw this image in SVG.

Rules:
1. Identify WHAT the image depicts (flower, face, pet, logo, chart, diagram, etc.)
2. List each visual component with its type, approximate position, size, and color
3. Use SVG primitive types: ellipse, circle, rect, polygon, line, path, text
4. Positions are in image pixel coordinates (origin top-left)
5. Colors as hex (#rrggbb)
6. For text elements, include the text content and estimated font size
7. Describe ARRANGEMENT patterns (e.g., "8 petals in radial arrangement at 45° intervals")
8. Output ONLY a JSON object, no markdown fences, no explanation

Schema:
{
  "interpretation": "one-line description of what this image depicts",
  "type": "flower|face|pet|logo|chart|illustration|diagram|other",
  "components": [
    {
      "id": "part_001",
      "role": "petal|eye|body|mark|stem|background|...",
      "shape": "ellipse|circle|rect|polygon|line|path|text",
      "position": [cx, cy],
      "size": [w, h] or {"r": radius} or [x1,y1,x2,y2] for line,
      "rotation": 0,
      "fill": "#hex",
      "text": "only for text shapes",
      "font_size": 0,
      "arrangement": "optional: describes pattern membership"
    }
  ],
  "arrangements": [
    {
      "type": "radial|linear|grid|concentric",
      "center": [cx, cy],
      "count": 8,
      "interval_degrees": 45,
      "member_roles": ["petal"]
    }
  ]
}"""


def _encode_image(path: str) -> str:
    with open(path, "rb") as f:
        return base64.b64encode(f.read()).decode()


def _call_vlm(image_path: str, cv_facts: dict) -> dict:
    """Send image + CV facts to Qwen VLM, get scene understanding."""
    key = os.environ.get("DASHSCOPE_API_KEY")
    if not key:
        raise RuntimeError("DASHSCOPE_API_KEY not set")

    img_b64 = _encode_image(image_path)
    # keep image small for API
    from PIL import Image
    import io
    img = Image.open(image_path)
    if max(img.size) > 800:
        ratio = 800 / max(img.size)
        img = img.resize((int(img.width * ratio), int(img.height * ratio)))
    buf = io.BytesIO()
    img.save(buf, format="JPEG", quality=85)
    img_b64 = base64.b64encode(buf.getvalue()).decode()

    user_msg = (
        f"Analyze this image.\n\n"
        f"CV measurements (from automated processing):\n"
        f"{json.dumps(cv_facts, indent=1, ensure_ascii=False)[:3000]}\n\n"
        f"Produce the scene understanding JSON."
    )

    body = json.dumps({
        "model": MODEL,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": [
                {"type": "image_url",
                 "image_url": {"url": f"data:image/jpeg;base64,{img_b64}"}},
                {"type": "text", "text": user_msg},
            ]},
        ],
        "max_tokens": 2000,
        "temperature": 0.1,
    }).encode()

    req = urllib.request.Request(API_URL, data=body, method="POST", headers={
        "Authorization": f"Bearer {key}",
        "Content-Type": "application/json",
    })
    with urllib.request.urlopen(req, timeout=60) as r:
        resp = json.load(r)

    content = resp["choices"][0]["message"]["content"]
    # strip markdown fences if present
    content = content.strip()
    if content.startswith("```"):
        content = content.split("\n", 1)[1].rsplit("```", 1)[0].strip()
    return json.loads(content)


def understand(image_path: str, cv_facts: dict) -> dict:
    """Main entry: image + facts → scene understanding."""
    return _call_vlm(image_path, cv_facts)
