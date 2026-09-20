"""figuresvg -- Raster Figure -> Semantic Scene Graph -> Editable SVG.

Layered recognizers (deterministic CV first, VLM optional later):
  OCR (PaddleOCR)  -> text elements with bbox/font-size/color
  Geometry (Rust engine, palette layers) -> rect/circle/line/polygon/path
  Fusion (pure code) -> overlap resolution + label relations
  SVG generator (pure code) -> editable SVG: <text>, <rect>, <circle>, ...
"""

from .scene import Scene, Element  # noqa: F401

__version__ = "0.1.0"
