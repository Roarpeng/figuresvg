"""Geometry recognizer: the Rust palette-layer engine, scene mode."""

import json
import os
import subprocess
import tempfile

from ..scene import Scene

ENGINE = os.environ.get(
    "FIGURESVG_ENGINE",
    os.path.join(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
        "engine",
        "target",
        "release",
        "figure2svg",
    ),
)


def recognize_raw(path: str, recolor: list | None = None) -> dict:
    """Run analyze + build --scene; return the RAW scene JSON (v2 objects)."""
    with tempfile.TemporaryDirectory() as td:
        spec_path = os.path.join(td, "spec.json")
        scene_path = os.path.join(td, "scene.json")
        subprocess.run(
            [ENGINE, "analyze", path, "--proposal", spec_path],
            check=True, capture_output=True,
        )
        if recolor:
            spec = json.load(open(spec_path))
            spec["recolor"] = recolor
            json.dump(spec, open(spec_path, "w"))
        subprocess.run(
            [ENGINE, "build", path, "--spec", spec_path,
             "--out", os.path.join(td, "out.svg"), "--scene", scene_path],
            check=True, capture_output=True,
        )
        return json.load(open(scene_path))


def recognize(path: str, recolor: list | None = None) -> Scene:
    return Scene.from_engine_json(recognize_raw(path, recolor=recolor))
