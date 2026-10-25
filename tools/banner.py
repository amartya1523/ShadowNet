"""Render media/banner.svg from the real generated topology.

    shadownetd topology > /tmp/topo.json && python3 tools/banner.py /tmp/topo.json

Every dot is a router and every line a link of the default 100k-node
network, in the same projection the dashboard uses.
"""

import json
import math
import sys

W, H = 1280, 400
MAP_X0, MAP_X1, MAP_Y0, MAP_Y1 = 40, 1240, 20, 380


def project(lat, lon):
    # Equirectangular, cropped to where people live (-56..72 latitude).
    x = MAP_X0 + (lon + 180) / 360 * (MAP_X1 - MAP_X0)
    y = MAP_Y0 + (72 - lat) / 128 * (MAP_Y1 - MAP_Y0)
    return x, y


def main(path):
    topo = json.load(open(path))
    routers = topo["routers"]
    tier = {a["id"]: a["tier"] for a in topo["ases"]}
    pos = [project(r["lat"], r["lon"]) for r in routers]

    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" height="{H}">',
        "<defs>",
        '<radialGradient id="bg" cx="50%" cy="45%" r="75%">'
        '<stop offset="0" stop-color="#0b1626"/><stop offset="1" stop-color="#04070d"/></radialGradient>',
        '<linearGradient id="fade" x1="0" x2="0" y1="0" y2="1">'
        '<stop offset="0.35" stop-color="#04070d" stop-opacity="0"/>'
        '<stop offset="1" stop-color="#04070d" stop-opacity="0.92"/></linearGradient>',
        '<filter id="glow"><feGaussianBlur stdDeviation="2.2"/></filter>',
        "</defs>",
        f'<rect width="{W}" height="{H}" rx="18" fill="url(#bg)"/>',
    ]
    # graticule
    for lon in range(-180, 181, 30):
        x, _ = project(0, lon)
        out.append(f'<line x1="{x:.1f}" y1="{MAP_Y0}" x2="{x:.1f}" y2="{MAP_Y1}" stroke="#16263a" stroke-width="0.6"/>')
    for lat in range(-45, 72, 15):
        _, y = project(lat, 0)
        out.append(f'<line x1="{MAP_X0}" y1="{y:.1f}" x2="{MAP_X1}" y2="{y:.1f}" stroke="#16263a" stroke-width="0.6"/>')

    lines = []
    for l in topo["links"]:
        (x1, y1), (x2, y2) = pos[l["a"]], pos[l["b"]]
        if abs(x1 - x2) > (MAP_X1 - MAP_X0) / 2:
            continue  # wraps the antimeridian; skip rather than draw across the map
        kind = l["kind"]
        color, width, alpha = {
            "internal": ("#2dd4bf", 0.5, 0.35),
            "peering": ("#a78bfa", 0.7, 0.55),
            "transit": ("#38bdf8", 0.6, 0.45),
        }[kind]
        if l["capacityGbps"] >= 100:
            width, alpha = 0.9, 0.6
        # Gentle arc so long-haul links read as cables, not ruler lines.
        mx, my = (x1 + x2) / 2, (y1 + y2) / 2
        d = math.hypot(x2 - x1, y2 - y1)
        cx, cy = mx, my - d * 0.12
        lines.append(
            f'<path d="M{x1:.1f},{y1:.1f} Q{cx:.1f},{cy:.1f} {x2:.1f},{y2:.1f}" fill="none" '
            f'stroke="{color}" stroke-opacity="{alpha}" stroke-width="{width}"/>'
        )
    out.append('<g>' + "".join(lines) + "</g>")

    dots = []
    glow = []
    for r, (x, y) in zip(routers, pos):
        t = tier[r["as"]]
        if t == "tier1":
            dots.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="2.1" fill="#e0f2fe"/>')
            glow.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="4.5" fill="#22d3ee" opacity="0.5"/>')
        elif r["role"] == "datacenter":
            dots.append(f'<rect x="{x-1.8:.1f}" y="{y-1.8:.1f}" width="3.6" height="3.6" fill="#fbbf24"/>')
        elif t == "tier2":
            dots.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="1.5" fill="#67e8f9" opacity="0.9"/>')
        else:
            dots.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="0.9" fill="#94a3b8" opacity="0.7"/>')
    out.append('<g filter="url(#glow)">' + "".join(glow) + "</g>")
    out.append("<g>" + "".join(dots) + "</g>")

    out.append(f'<rect width="{W}" height="{H}" rx="18" fill="url(#fade)"/>')
    s = topo["summary"]
    font = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"
    out.append(
        f'<text x="56" y="318" font-family="{font}" font-size="58" font-weight="700" '
        f'fill="#f1f5f9" letter-spacing="6">SHADOWNET</text>'
    )
    out.append(
        f'<text x="60" y="352" font-family="{font}" font-size="17" fill="#7dd3fc">'
        f'{s["nodes"]:,} nodes · {s["routers"]:,} routers · {s["tier1"] + s["tier2"] + s["stubs"]} autonomous systems'
        f" · none of them real</text>"
    )
    out.append(
        f'<text x="1240" y="44" text-anchor="end" font-family="{font}" font-size="13" fill="#475569">'
        f"every dot on this map is a router in the default topology</text>"
    )
    out.append("</svg>")
    open("media/banner.svg", "w").write("\n".join(out))
    print("wrote media/banner.svg")


if __name__ == "__main__":
    main(sys.argv[1])
