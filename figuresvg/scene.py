"""Scene Graph data model (see schema/SCENE_GRAPH.md)."""

from dataclasses import dataclass, field, asdict
from typing import List, Optional


@dataclass
class Element:
    id: str
    type: str  # text rect circle ellipse line polyline polygon path
    bbox: List[int]  # x, y, w, h
    source: str = "geometry"
    confidence: float = 0.95
    fill: Optional[str] = None
    stroke: Optional[str] = None
    stroke_width: Optional[float] = None
    tags: List[str] = field(default_factory=list)
    rotate: Optional[float] = None
    text_anchor: Optional[str] = None
    # type-specific
    text: Optional[str] = None
    font_size: Optional[float] = None
    font_family: Optional[str] = None
    x: Optional[int] = None
    y: Optional[int] = None
    w: Optional[int] = None
    h: Optional[int] = None
    rx: Optional[float] = None
    cx: Optional[int] = None
    cy: Optional[int] = None
    r: Optional[float] = None
    x1: Optional[int] = None
    y1: Optional[int] = None
    x2: Optional[int] = None
    y2: Optional[int] = None
    points: Optional[str] = None
    d: Optional[str] = None
    transform: Optional[str] = None
    # fidelity candidates: traced-glyph geometry elements of a text element
    # (runtime only; deliberately not serialized into the scene JSON)
    trace: List[object] = field(default_factory=list)
    # refine decision: True -> <text>, False -> traced glyphs, None -> auto
    editable: Optional[bool] = None
    # bitmap-optimized text metrics (runtime)
    dx: Optional[float] = None
    dy: Optional[float] = None
    baseline_dy: Optional[int] = None
    font_weight: Optional[str] = None
    box_diff: Optional[float] = None
    calib_score: Optional[float] = None
    gradient: Optional[dict] = None


@dataclass
class Relation:
    frm: str
    to: str
    type: str  # labels / inside / connects / same_group


@dataclass
class Scene:
    schema: str = "figuresvg/scene@1"
    width: int = 0
    height: int = 0
    background: str = "#ffffff"
    elements: List[Element] = field(default_factory=list)
    relations: List[Relation] = field(default_factory=list)
    notes: dict = field(default_factory=dict)
    defs: list = field(default_factory=list)

    @classmethod
    def from_engine_json(cls, data: dict) -> "Scene":
        sc = cls(
            width=data["canvas"]["width"],
            height=data["canvas"]["height"],
            notes=data.get("notes", {}),
        )
        for e in data.get("elements", []):
            known = {f for f in Element.__dataclass_fields__}
            e = dict(e)
            e["id"] = e.get("id") or f"shape_{len(sc.elements) + 1:03d}"
            sc.elements.append(Element(**{k: v for k, v in e.items() if k in known}))
        return sc

    def to_json(self) -> dict:
        return {
            "schema": self.schema,
            "canvas": {"width": self.width, "height": self.height},
            "background": self.background,
            "elements": [
                {k: v for k, v in asdict(e).items() if v is not None and k not in ("trace", "editable", "baseline_dy", "font_weight", "box_diff", "dx", "dy", "calib_score", "gradient")}
                for e in self.elements
            ],
            "relations": [asdict(r) for r in self.relations],
            "notes": self.notes,
        }
