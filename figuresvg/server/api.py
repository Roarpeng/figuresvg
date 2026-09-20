"""HTTP API service.

Run:  uvicorn figuresvg.server.api:app --host 0.0.0.0 --port 8417
      (or: python -m figuresvg.server.api)

Endpoints:
  GET  /health            -> {"status": "ok"}
  POST /api/convert       -> multipart file upload (field "file")
                              ?recolor=<json> optional &skip_ocr=1 optional
                              returns {"svg": "...", "scene": {...}, "stats": {...}}
"""

import json
import os
import tempfile

from fastapi import FastAPI, File, Query, UploadFile
from fastapi.responses import JSONResponse

from ..pipeline import convert

app = FastAPI(
    title="figuresvg",
    description="Raster Figure -> Semantic Scene Graph -> Editable SVG",
    version="0.1.0",
)

_ENGINE_WARM = False


@app.on_event("startup")
def _warm():
    """Load the OCR model once at server start, not on first request."""
    global _ENGINE_WARM
    if os.environ.get("FIGURESVG_SKIP_WARM"):
        return
    from ..recognizers import ocr

    ocr._engine()
    _ENGINE_WARM = True


@app.get("/health")
def health():
    return {"status": "ok", "ocr_warm": _ENGINE_WARM}


@app.post("/api/convert")
async def api_convert(
    file: UploadFile = File(...),
    recolor: str = Query(default=""),
    skip_ocr: bool = Query(default=False),
):
    suffix = os.path.splitext(file.filename or "img.png")[1] or ".png"
    with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as td:
        td.write(await file.read())
        tmp_in = td.name
    try:
        with tempfile.TemporaryDirectory() as outdir:
            svg_path = os.path.join(outdir, "out.svg")
            scene_path = os.path.join(outdir, "scene.json")
            stats = convert(
                tmp_in, svg_path, scene_out=scene_path,
                recolor=json.loads(recolor) if recolor else None,
                skip_ocr=skip_ocr,
            )
            return JSONResponse({
                "filename": file.filename,
                "stats": stats,
                "scene": json.load(open(scene_path)),
                "svg": open(svg_path).read(),
            })
    finally:
        os.unlink(tmp_in)


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app, host="0.0.0.0", port=int(os.environ.get("PORT", 8417)))
