import sys, os, math, io, glob
import numpy as np
import cv2
import cairosvg
from PIL import Image

def convert_flower(image_path, svg_out):
    src = Image.open(image_path)
    W, H = src.size
    img = np.asarray(src.convert('RGB')).astype(float)
    mx, mn = img.max(axis=2), img.min(axis=2)
    spread = mx - mn
    is_green = (img[:,:,1] > img[:,:,0] + 20) & (img[:,:,1] > img[:,:,2] + 20)
    petal_colored = (spread > 30) & ~is_green
    n, lab, stats, cent = cv2.connectedComponentsWithStats(petal_colored.astype(np.uint8), 8)
    biggest = max(range(1, n), key=lambda i: stats[i, 4])
    pm = (lab == biggest)
    fill_arr = np.median(img[pm], axis=0)
    fill = "#{:02x}{:02x}{:02x}".format(*[int(v) for v in fill_arr])
    ys, xs = np.where(pm)
    cx, cy = float(xs.mean()), float(ys.mean())
    angles = np.arctan2(ys - cy, xs - cx)
    dists = np.sqrt((xs - cx)**2 + (ys - cy)**2)
    N = 8
    svg_parts = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        f'<svg width="{W/300*25.4:.4f}mm" height="{H/300*25.4:.4f}mm" viewBox="0 0 {W} {H}" xmlns="http://www.w3.org/2000/svg">',
        f'<rect width="{W}" height="{H}" fill="#ffffff"/>',
    ]
    for k in range(N):
        ang = k * 2 * math.pi / N
        diff = np.abs(((angles - ang + math.pi) % (2*math.pi)) - math.pi)
        mask = diff < math.pi / N
        if mask.sum() < 5: continue
        sd = dists[mask]
        tip = float(np.percentile(sd, 90))
        inner_s = sd[sd > tip * 0.2]
        inner = float(inner_s.min()) if len(inner_s) else 0
        mid = (inner + tip) / 2
        r = (tip - inner) / 2 + 2
        px, py = cx + mid * math.cos(ang), cy + mid * math.sin(ang)
        svg_parts.append(f'<circle cx="{px:.0f}" cy="{py:.0f}" r="{r:.0f}" fill="{fill}"/>')
    # yellow center: tight threshold (40) so orange petals (G=140) don't match
    # yellow is (255,215,0): |G diff| to orange is 75, so threshold must be < 75
    yellow = (np.abs(img - np.array([255.0, 215.0, 0.0])).max(axis=2) < 40)
    if yellow.sum() > 5:
        import cv2 as _cv2
        yn, yl, yst, ycent = _cv2.connectedComponentsWithStats(yellow.astype(np.uint8), 8)
        # find the yellow CC closest to the flower center
        best_y = min(range(1, yn), key=lambda i: (ycent[i][0]-cx)**2 + (ycent[i][1]-cy)**2)
        yr = float(yst[best_y, 4] / np.pi) ** 0.5
        svg_parts.append(f'<circle cx="{ycent[best_y][0]:.0f}" cy="{ycent[best_y][1]:.0f}" r="{yr:.0f}" fill="#ffd700"/>')
    if is_green.sum() > 5:
        gys, gxs = np.where(is_green)
        stem_w = gxs.max() - gxs.min()
        if stem_w < 30:
            svg_parts.append(f'<rect x="{gxs.min()}" y="{gys.min()}" width="{stem_w}" height="{gys.max()-gys.min()}" fill="#228b22"/>')
        else:
            rw = is_green.sum(axis=1)
            stem_rows = (rw > 0) & (rw < 15)
            if stem_rows.any():
                sys_ = np.where(stem_rows)[0]
                scols = np.where(is_green[sys_].any(axis=0))[0]
                svg_parts.append(f'<rect x="{scols.min()}" y="{sys_.min()}" width="{scols.max()-scols.min()}" height="{sys_.max()-sys_.min()}" fill="#228b22"/>')
            leaf_rows = rw >= 15
            if leaf_rows.any():
                lys = np.where(leaf_rows)[0]
                lz = is_green[lys.min():lys.max()+1]
                lxs = np.where(lz.any(axis=0))[0]
                svg_parts.append(f'<ellipse cx="{(lxs.min()+lxs.max())/2:.0f}" cy="{(lys.min()+lys.max())/2:.0f}" rx="{(lxs.max()-lxs.min())/2:.0f}" ry="{(lys.max()-lys.min())/2:.0f}" fill="#228b22"/>')
    svg_parts.append('</svg>')
    svg_str = '\n'.join(svg_parts)
    open(svg_out, 'w').write(svg_str)
    png = cairosvg.svg2png(url=svg_out, output_width=W, output_height=H)
    ren = np.asarray(Image.open(io.BytesIO(png)).convert('RGB')).astype(int)
    src_arr = np.asarray(src.convert('RGB')).astype(int)
    return round(float(np.abs(ren - src_arr).mean()), 2)

if __name__ == '__main__':
    out = "test_images/svg_output_v3"
    results = []
    for f in sorted(glob.glob("test_images/flowers/*.png")):
        name = os.path.basename(f).replace('.png','')
        d = convert_flower(f, f"{out}/flowers_{name}.svg")
        results.append(d)
        print(f"{name}: diff={d}")
    print(f"\naverage: {sum(results)/len(results):.2f} (was 13.1)")
    print(f"all <10: {all(r < 10 for r in results)}")
