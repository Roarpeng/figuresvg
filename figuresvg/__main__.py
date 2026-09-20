"""CLI: python -m figuresvg convert IN.png OUT.svg [--scene s.json] [--recolor-json ...]"""

import argparse
import json
import sys


def main(argv=None):
    p = argparse.ArgumentParser(prog="figuresvg")
    sub = p.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("convert", help="image -> editable SVG")
    c.add_argument("input")
    c.add_argument("output")
    c.add_argument("--scene", help="also write the Scene Graph JSON")
    c.add_argument("--recolor-json", help='recolor spec, e.g. \'[{"row":9,"donor_rows":[7,8,12]}]\'')
    c.add_argument("--skip-ocr", action="store_true")
    a = p.parse_args(argv)

    from .pipeline import convert

    recolor = json.loads(a.recolor_json) if a.recolor_json else None
    stats = convert(a.input, a.output, scene_out=a.scene,
                    recolor=recolor, skip_ocr=a.skip_ocr)
    for k, v in stats.items():
        print(f"{k}: {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
