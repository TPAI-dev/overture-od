#!/usr/bin/env python3
"""Generate the OVERTURE app icon set with no third-party deps.

The mark: a matte-enamel rounded square, a rising terracotta land-curve, and a
parchment playhead scrubbing it (triangle cap + node) — the product's signature
"a build is a timeline you scrub" compressed to an icon. Rendered per-size with
signed-distance shapes so edges stay clean from 16px to 1024px.
"""
import os, struct, zlib, subprocess, shutil, math

OUT = os.path.join(os.path.dirname(__file__), "icons")
ISET = os.path.join(OUT, "icon.iconset")

INK_TOP = (0x16, 0x18, 0x1d)
INK_BOT = (0x0a, 0x0b, 0x0e)
TERRA   = (0xcf, 0x8a, 0x63)
PARCH   = (0xe9, 0xe5, 0xd8)

# normalized land-curve control points (x, y) — y down, lower = higher up
CURVE = [(0.15, 0.63), (0.30, 0.575), (0.44, 0.50),
         (0.58, 0.41), (0.72, 0.335), (0.85, 0.285)]
BASE_Y = 0.70
PH_X = 0.42  # playhead x


def curve_y(x):
    pts = CURVE
    if x <= pts[0][0]:
        return pts[0][1]
    if x >= pts[-1][0]:
        return pts[-1][1]
    for i in range(len(pts) - 1):
        x0, y0 = pts[i]
        x1, y1 = pts[i + 1]
        if x0 <= x <= x1:
            t = (x - x0) / (x1 - x0)
            return y0 + (y1 - y0) * t
    return pts[-1][1]


def seg_dist(px, py, ax, ay, bx, by):
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    L = vx * vx + vy * vy
    t = 0.0 if L == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / L))
    dx, dy = px - (ax + vx * t), py - (ay + vy * t)
    return math.hypot(dx, dy)


def cover(d, half, aa):
    return max(0.0, min(1.0, 0.5 - (d - half) / aa))


def over(dst, src, a):
    return (src[0] * a + dst[0] * (1 - a),
            src[1] * a + dst[1] * (1 - a),
            src[2] * a + dst[2] * (1 - a))


def rrect_sdf(px, py, cx, cy, hw, hh, r):
    qx, qy = abs(px - cx) - (hw - r), abs(py - cy) - (hh - r)
    return math.hypot(max(qx, 0), max(qy, 0)) + min(max(qx, qy), 0) - r


def render(S):
    aa = 1.4
    px_w = max(1.0, S * 0.020)            # curve stroke half-width
    ph_w = max(0.8, S * 0.0125)           # playhead half-width
    buf = bytearray(S * S * 4)
    # precompute triangle (down-pointing cap)
    tw, ty0, ty1 = S * 0.052, S * 0.135, S * 0.225
    Ax, Ay = PH_X * S - tw, ty0
    Bx, By = PH_X * S + tw, ty0
    Cx, Cy = PH_X * S, ty1
    node_x, node_y = PH_X * S, curve_y(PH_X) * S
    for j in range(S):
        for i in range(S):
            x, y = i + 0.5, j + 0.5
            # background rounded rect (transparent outside)
            sdf = rrect_sdf(x, y, S / 2, S / 2, S / 2 - S * 0.012,
                            S / 2 - S * 0.012, S * 0.205)
            bg_a = max(0.0, min(1.0, 0.5 - sdf / aa))
            if bg_a <= 0.0:
                continue
            g = y / S
            col = (INK_TOP[0] + (INK_BOT[0] - INK_TOP[0]) * g,
                   INK_TOP[1] + (INK_BOT[1] - INK_TOP[1]) * g,
                   INK_TOP[2] + (INK_BOT[2] - INK_TOP[2]) * g)
            xn = x / S
            cy = curve_y(xn) * S
            # under-curve fill (terracotta, soft)
            if x >= CURVE[0][0] * S and x <= CURVE[-1][0] * S:
                fa = (max(0.0, min(1.0, (y - cy) / aa))
                      * max(0.0, min(1.0, (BASE_Y * S - y) / (aa * 2))))
                if fa > 0:
                    col = over(col, TERRA, 0.16 * fa)
            # baseline (faint parchment)
            col = over(col, PARCH, 0.10 * cover(abs(y - BASE_Y * S), S * 0.004, aa))
            # land-curve stroke
            dmin = 1e9
            for k in range(len(CURVE) - 1):
                ax_, ay_ = CURVE[k][0] * S, CURVE[k][1] * S
                bx_, by_ = CURVE[k + 1][0] * S, CURVE[k + 1][1] * S
                dmin = min(dmin, seg_dist(x, y, ax_, ay_, bx_, by_))
            col = over(col, TERRA, cover(dmin, px_w, aa))
            # playhead vertical line (parchment)
            if ty0 <= y <= BASE_Y * S + S * 0.04:
                col = over(col, PARCH, 0.92 * cover(abs(x - PH_X * S), ph_w, aa))
            # triangle cap (terracotta) — convex polygon SDF
            d_ab = seg_dist(x, y, Ax, Ay, Bx, By)
            d_bc = seg_dist(x, y, Bx, By, Cx, Cy)
            d_ca = seg_dist(x, y, Cx, Cy, Ax, Ay)
            dtri = min(d_ab, d_bc, d_ca)
            # inside test via cross products (consistent winding A->B->C)
            def cross(ox, oy, ax_, ay_, bx_, by_):
                return (ax_ - ox) * (by_ - oy) - (ay_ - oy) * (bx_ - ox)
            s1 = cross(Ax, Ay, Bx, By, x, y)
            s2 = cross(Bx, By, Cx, Cy, x, y)
            s3 = cross(Cx, Cy, Ax, Ay, x, y)
            inside = (s1 >= 0 and s2 >= 0 and s3 >= 0) or (s1 <= 0 and s2 <= 0 and s3 <= 0)
            tri_sdf = -dtri if inside else dtri
            col = over(col, TERRA, max(0.0, min(1.0, 0.5 - tri_sdf / aa)))
            # node where playhead meets curve
            dn = math.hypot(x - node_x, y - node_y)
            col = over(col, TERRA, cover(dn, S * 0.028, aa))
            o = (j * S + i) * 4
            buf[o] = int(col[0] + 0.5)
            buf[o + 1] = int(col[1] + 0.5)
            buf[o + 2] = int(col[2] + 0.5)
            buf[o + 3] = int(bg_a * 255 + 0.5)
    return bytes(buf)


def write_png(path, S, rgba):
    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        return c + struct.pack(">I", zlib.crc32(typ + data) & 0xffffffff)
    raw = bytearray()
    for j in range(S):
        raw.append(0)
        raw += rgba[j * S * 4:(j + 1) * S * 4]
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", S, S, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return png


def main():
    os.makedirs(OUT, exist_ok=True)
    if os.path.isdir(ISET):
        shutil.rmtree(ISET)
    os.makedirs(ISET)
    sizes = [16, 32, 48, 64, 128, 256, 512, 1024]
    pngs = {}
    for S in sizes:
        pngs[S] = write_png(os.path.join(OUT, f"_{S}.png"), S, render(S))
        print(f"  rendered {S}px")
    # Tauri-referenced names
    shutil.copyfile(os.path.join(OUT, "_32.png"), os.path.join(OUT, "32x32.png"))
    shutil.copyfile(os.path.join(OUT, "_128.png"), os.path.join(OUT, "128x128.png"))
    shutil.copyfile(os.path.join(OUT, "_256.png"), os.path.join(OUT, "128x128@2x.png"))
    shutil.copyfile(os.path.join(OUT, "_512.png"), os.path.join(OUT, "icon.png"))
    # iconset for icns
    iset_map = {"icon_16x16.png": 16, "icon_16x16@2x.png": 32, "icon_32x32.png": 32,
                "icon_32x32@2x.png": 64, "icon_128x128.png": 128, "icon_128x128@2x.png": 256,
                "icon_256x256.png": 256, "icon_256x256@2x.png": 512, "icon_512x512.png": 512,
                "icon_512x512@2x.png": 1024}
    for name, S in iset_map.items():
        shutil.copyfile(os.path.join(OUT, f"_{S}.png"), os.path.join(ISET, name))
    try:
        subprocess.run(["iconutil", "-c", "icns", ISET, "-o",
                        os.path.join(OUT, "icon.icns")], check=True)
        print("  icon.icns ok")
    except Exception as e:
        print("  icns skipped:", e)
    # ICO (PNG-embedded entries; valid Vista+)
    ico_sizes = [16, 32, 48, 64, 128, 256]
    entries, blobs, offset = b"", b"", 6 + 16 * len(ico_sizes)
    for S in ico_sizes:
        data = pngs[S] if S in pngs else write_png(os.path.join(OUT, f"_{S}.png"), S, render(S))
        bsz = len(data)
        w = 0 if S >= 256 else S
        entries += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, bsz, offset)
        blobs += data
        offset += bsz
    with open(os.path.join(OUT, "icon.ico"), "wb") as f:
        f.write(struct.pack("<HHH", 0, 1, len(ico_sizes)) + entries + blobs)
    print("  icon.ico ok")
    # tidy scratch
    for S in sizes:
        p = os.path.join(OUT, f"_{S}.png")
        if os.path.exists(p):
            os.remove(p)
    shutil.rmtree(ISET, ignore_errors=True)
    print("done ->", OUT)


if __name__ == "__main__":
    main()
