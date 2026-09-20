"""Legend extraction: group small swatch shapes with their adjacent text
labels into <g id="legend-N"> entries -- the structure vector editors and
downstream tools expect."""

SWATCH_MAX_AREA = 4000
GAP = 46


def _vert_overlap(a, b):
    ay0, ay1 = a[1], a[1] + a[3]
    by0, by1 = b[1], b[1] + b[3]
    ov = min(ay1, by1) - max(ay0, by0)
    return ov / max(1, min(ay1 - ay0, by1 - by0))


def extract(scene):
    """Tag legend entries and legend groups on scene elements."""
    texts = [e for e in scene.elements if e.type == "text"]
    shapes = [e for e in scene.elements
              if e.type in ("rect", "circle", "line", "polygon")
              and e.bbox[2] * e.bbox[3] <= SWATCH_MAX_AREA
              and max(e.bbox[2], e.bbox[3]) <= 60]
    pairs = []
    used_s = set()
    for t in texts:
        best = None
        for i, s in enumerate(shapes):
            if i in used_s:
                continue
            # swatch immediately LEFT of the text (classic legend row)
            dx = t.bbox[0] - (s.bbox[0] + s.bbox[2])
            if -6 <= dx <= GAP and _vert_overlap(t.bbox, s.bbox) >= 0.55:
                if best is None or dx < best[1]:
                    best = (i, dx)
        if best is not None:
            used_s.add(best[0])
            pairs.append((shapes[best[0]], t))
    # cluster pairs sharing a y-band into legend groups (>=2 entries)
    pairs.sort(key=lambda p: p[1].bbox[1])
    groups, cur = [], []
    for p in pairs:
        if cur and abs(p[1].bbox[1] - cur[-1][1].bbox[1]) > 28:
            if len(cur) >= 2:
                groups.append(cur)
            cur = []
        cur.append(p)
    if len(cur) >= 2:
        groups.append(cur)
    n_entries = 0
    for gi, g in enumerate(groups, 1):
        gid = f"legend_{gi}"
        for s, t in g:
            s.tags = s.tags + ["legend-swatch", gid]
            t.tags = t.tags + ["legend-label", gid]
            n_entries += 1
    stats = {"legend_groups": len(groups), "legend_entries": n_entries}
    stats.update(group_error_bars(scene))
    return stats


def group_error_bars(scene):
    """S3: cluster thin dark stroke objects into error-bar groups.

    An error bar = a vertical stem plus cap strokes sharing the same
    x-center. Group dark line/path objects whose centers align within 3px
    horizontally and stack within 25px vertically (2-4 strokes per bar).
    """
    def _dark_small(e):
        f = e.fill or ""
        dark = f in ("#282828", "#000000") or (
            f.startswith("#") and len(f) == 7
            and 0 <= int(f[1:3], 16) < 70
            and 0 <= int(f[3:5], 16) < 70
            and 0 <= int(f[5:7], 16) < 70)
        return (e.type in ("line", "path") and dark
                and e.bbox[2] <= 30 and e.bbox[3] <= 40
                and e.bbox[2] * e.bbox[3] <= 900)

    strokes = [e for e in scene.elements if _dark_small(e)]
    strokes.sort(key=lambda e: (e.bbox[0], e.bbox[1]))
    used = set()
    groups = 0
    entries = 0
    for i, s in enumerate(strokes):
        if i in used:
            continue
        cx = s.bbox[0] + s.bbox[2] / 2
        members = [i]
        for j in range(i + 1, len(strokes)):
            if j in used:
                continue
            t = strokes[j]
            tcx = t.bbox[0] + t.bbox[2] / 2
            close_y = abs((t.bbox[1] + t.bbox[3] / 2)
                          - (s.bbox[1] + s.bbox[3] / 2)) <= 28
            if abs(tcx - cx) <= 3.5 and close_y:
                members.append(j)
        if 2 <= len(members) <= 5:
            groups += 1
            gid = f"errbar_{groups}"
            for j in members:
                used.add(j)
                strokes[j].tags = strokes[j].tags + ["errbar", gid]
                entries += 1
    return {"errbar_groups": groups, "errbar_entries": entries}
