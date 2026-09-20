# Scene Graph JSON — the intermediate contract

Every recognizer (OCR, geometry, VLM, future ones) emits Scene Graph
elements; the SVG generator consumes ONLY the Scene Graph. This file is
normative.

## Top level

```json
{
  "schema": "figuresvg/scene@1",
  "canvas": { "width": 1920, "height": 1080 },
  "background": "#ffffff",
  "elements": [ ... ],
  "relations": [ ... ],
  "notes": { "out_of_scope": false, "engine": "palette@1" }
}
```

## Element

Common fields (all elements):

| field | type | meaning |
| --- | --- | --- |
| `id` | string | unique, stable (`text_001`, `shape_012`) |
| `type` | enum | `text` `rect` `circle` `ellipse` `line` `polyline` `polygon` `path` `group` |
| `bbox` | [x,y,w,h] | axis-aligned bounding box, pixels |
| `fill` | color? | `#rrggbb` or `none` |
| `stroke` | color? | stroke color, if any |
| `stroke_width` | number? | px |
| `confidence` | 0..1 | recognizer confidence |
| `source` | string | which recognizer produced it (`ocr`, `geometry`, `vlm`, `fusion`) |
| `tags` | string[] | semantic tags (`axis-label`, `legend`, `title`, ...) |

Type-specific fields:

- `text`: `text` (string), `font_size` (px, estimated from bbox height),
  `color`, optional `font_family`
- `rect`: `x` `y` `w` `h`, optional `rx` (rounded corners)
- `circle`: `cx` `cy` `r`
- `ellipse`: `cx` `cy` `rx` `ry`
- `line`: `x1` `y1` `x2` `y2`
- `polyline` / `polygon`: `points` (`"x,y x,y ..."`), `closed` (bool)
- `path`: `d` (SVG path data — last resort for complex outlines)

## Relation

```json
{ "from": "text_001", "to": "shape_003", "type": "labels" }
```

Relation types: `labels` (text labels a shape), `inside` (element contained
in another), `connects` (line/arrow endpoints), `same_group` (visually
clustered, e.g. legend entries).

## Rules

1. The SVG generator must not need the source image: the Scene Graph is
   complete and self-contained.
2. Recognizers never edit each other's elements; the fusion stage
   (pure code) resolves overlaps and builds relations.
3. Text is ALWAYS real text (`type: "text"`) — never traced outlines.
4. Geometry prefers the most specific primitive that fits within
   tolerance: rect > polygon > path.
5. Unknown VLM semantics attach as `tags` / relations, never as new
   geometry (VLM never invents coordinates).
```
