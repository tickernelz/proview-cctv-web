#!/usr/bin/env python3
import math, re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
TOKENS = ROOT / "tokens.css"
OUT = ROOT / "contrast.md"


def oklch_to_srgb(light, chroma, hue, alpha=1.0):
    hue = math.radians(hue)
    a = chroma * math.cos(hue)
    b = chroma * math.sin(hue)
    l_ = light + 0.3963377774 * a + 0.2158037573 * b
    m_ = light - 0.1055613458 * a - 0.0638541728 * b
    s_ = light - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_ ** 3, m_ ** 3, s_ ** 3
    r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

    def encode(x):
        x = min(1.0, max(0.0, x))
        return 12.92 * x if x <= 0.0031308 else 1.055 * x ** (1 / 2.4) - 0.055

    return (encode(r), encode(g), encode(bb), alpha)


def parse_block(css, selector):
    m = re.search(re.escape(selector) + r"\s*\{([^}]*)\}", css)
    if not m:
        sys.exit("selector not found: " + selector)
    out = {}
    for line in m.group(1).splitlines():
        line = line.strip()
        if not line.startswith("--"):
            continue
        name, value = line.split(":", 1)
        out[name.strip()[2:]] = value.strip().rstrip(";")
    return out


def colour(theme, name):
    raw = theme[name]
    m = re.match(r"oklch\(([^)]*)\)", raw)
    if not m:
        sys.exit("not an oklch value: " + name)
    parts = m.group(1).replace("/", " ").split()
    light, chroma, hue = (float(p) for p in parts[:3])
    alpha = float(parts[3]) if len(parts) > 3 else 1.0
    return oklch_to_srgb(light, chroma, hue, alpha)


def over(top, bottom):
    return tuple(top[i] * top[3] + bottom[i] * (1 - top[3]) for i in range(3)) + (1.0,)


def luminance(rgb):
    def linear(channel):
        return channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4

    r, g, b = (linear(c) for c in rgb[:3])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(fg, bg):
    a, b = luminance(fg), luminance(bg)
    high, low = max(a, b), min(a, b)
    return (high + 0.05) / (low + 0.05)


def hexof(rgb):
    return "#" + "".join(f"{round(min(1, max(0, c)) * 255):02x}" for c in rgb[:3])


TEXT = "text"
GRAPHIC = "graphic"

PAIRS = [
    ("ink on page", "ink", "bg", TEXT, 4.5),
    ("ink on bar", "ink", "bg-2", TEXT, 4.5),
    ("ink-2 on page", "ink-2", "bg", TEXT, 4.5),
    ("ink-2 on bar", "ink-2", "bg-2", TEXT, 4.5),
    ("ink-3 on page", "ink-3", "bg", TEXT, 4.5),
    ("ink-3 on bar", "ink-3", "bg-2", TEXT, 4.5),
    ("page on ink (filled button label)", "bg", "ink", TEXT, 4.5),
    ("stage-ink on stage", "stage-ink", "stage", TEXT, 4.5),
    ("stage-ink-2 on stage", "stage-ink-2", "stage", TEXT, 4.5),
    ("live on page", "live", "bg", TEXT, 4.5),
    ("live on live-soft over page", "live", "live-soft", TEXT, 4.5),
    ("ok on page", "ok", "bg", TEXT, 4.5),
    ("bad on page", "bad", "bg", TEXT, 4.5),
    ("warn on page", "warn", "bg", TEXT, 4.5),
    ("ok dot on page", "ok", "bg", GRAPHIC, 3.0),
    ("live dot on page", "live", "bg", GRAPHIC, 3.0),
    ("warn on page (graphic)", "warn", "bg", GRAPHIC, 3.0),
    ("line on page", "line", "bg", GRAPHIC, 1.6),
    ("line-soft on page", "line-soft", "bg", GRAPHIC, 1.4),
    ("focus ring on page", "focus", "bg", GRAPHIC, 3.0),
    ("focus ring on bar", "focus", "bg-2", GRAPHIC, 3.0),
    ("focus ring on stage", "focus", "stage", GRAPHIC, 3.0),
]


def main():
    css = TOKENS.read_text()
    themes = {
        "dark": parse_block(css, ':root[data-theme="dark"]'),
        "light": parse_block(css, ':root[data-theme="light"]'),
    }
    rows = []
    failures = []
    for theme_name, theme in themes.items():
        for label, fg_name, bg_name, kind, floor in PAIRS:
            fg = colour(theme, fg_name)
            bg = colour(theme, bg_name)
            composite = bg if bg_name != "live-soft" else over(bg, colour(theme, "bg"))
            r = ratio(over(fg, composite), composite)
            ok = r >= floor
            if not ok:
                failures.append(f"{theme_name}: {label} {r:.2f} < {floor}")
            rows.append((theme_name, label, hexof(over(fg, composite)), hexof(composite), f"{r:.2f}", f"{floor:.1f}", "pass" if ok else "FAIL"))

    lines = ["| Theme | Pair | Foreground | Background | Ratio | Floor | Result |", "|---|---|---|---|---|---|---|"]
    for r in rows:
        lines.append("| " + " | ".join(r) + " |")
    md = "\n".join(lines) + "\n"
    OUT.write_text(md)
    print(md)
    if failures:
        print("FAILURES:", *failures, sep="\n  ")
        return 1
    print(f"all {len(rows)} pairs pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
