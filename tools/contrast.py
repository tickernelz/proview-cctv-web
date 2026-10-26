#!/usr/bin/env python3
import math
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOKENS = ROOT / 'web' / 'tokens.css'


def oklch_to_srgb(light, chroma, hue, alpha=1.0):
    hue = math.radians(hue)
    a = chroma * math.cos(hue)
    b = chroma * math.sin(hue)
    l_ = light + 0.3963377774 * a + 0.2158037573 * b
    m_ = light - 0.1055613458 * a - 0.0638541728 * b
    s_ = light - 0.0894841775 * a - 1.2914855480 * b
    l, m, s = l_ ** 3, m_ ** 3, s_ ** 3
    r = +4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    bb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

    def encode(value):
        value = max(0.0, min(1.0, value))
        return 12.92 * value if value <= 0.0031308 else 1.055 * (value ** (1 / 2.4)) - 0.055

    return (encode(r), encode(g), encode(bb), alpha)


def parse_block(css, selector):
    pattern = re.escape(selector) + r'\s*\{([^}]*)\}'
    match = re.search(pattern, css)
    if not match:
        raise SystemExit(f'tokens.css has no {selector} block')
    tokens = {}
    for name, body in re.findall(r'--([a-z0-9-]+):\s*([^;]+);', match.group(1)):
        tokens[name] = body.strip()
    return tokens


def colour(theme, name):
    raw = theme[name]
    match = re.match(r'oklch\(\s*([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s*(?:/\s*([0-9.]+))?\s*\)', raw)
    if not match:
        raise SystemExit(f'token --{name} is not an oklch colour: {raw}')
    lightness, chroma, hue, alpha = match.groups()
    return oklch_to_srgb(float(lightness), float(chroma), float(hue), float(alpha) if alpha else 1.0)


def over(top, bottom):
    return tuple(top[i] * top[3] + bottom[i] * (1 - top[3]) for i in range(3)) + (1.0,)


def luminance(rgb):
    def linear(channel):
        return channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4

    r, g, b = (linear(c) for c in rgb[:3])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(foreground, background):
    a = luminance(foreground)
    b = luminance(background)
    high, low = max(a, b), min(a, b)
    return (high + 0.05) / (low + 0.05)


def flattens(theme, name, base):
    return over(colour(theme, name), colour(theme, base))


WHITE = (1.0, 1.0, 1.0, 1.0)
MID = (0.5, 0.5, 0.5, 1.0)

TEXT_PAIRS = [
    ('ink', 'bg'), ('ink', 'surface'), ('ink', 'surface-2'), ('ink', 'surface-3'),
    ('ink-dim', 'bg'), ('ink-dim', 'surface'), ('ink-dim', 'surface-2'), ('ink-dim', 'surface-3'),
    ('muted', 'bg'), ('muted', 'surface'), ('muted', 'surface-2'),
    ('primary-ink', 'primary'), ('primary-ink', 'primary-hover'),
    ('danger', 'bg'), ('danger', 'surface'),
    ('success', 'surface'),
    ('stage-ink', 'stage'), ('stage-ink', 'stage-matte'), ('stage-danger', 'stage-matte'),
    ('focus-ring', 'bg'), ('focus-ring', 'surface'),
]

GRAPHIC_PAIRS = [
    ('primary', 'bg'), ('primary', 'surface'),
    ('success', 'surface'), ('warning', 'surface'), ('danger', 'surface'),
]


def check(theme, pairs, threshold, label):
    failures = []
    rows = []
    for foreground, background in pairs:
        value = ratio(colour(theme, foreground), colour(theme, background))
        rows.append((foreground, background, value, threshold))
        if value < threshold:
            failures.append((foreground, background, value))
    for foreground, background, value, limit in rows:
        mark = 'PASS' if value >= limit else 'FAIL'
        print(f'  {label:7s} {foreground:14s} on {background:12s} {value:6.2f}:1  {mark}')
    return failures


def main():
    css = TOKENS.read_text()
    dark = parse_block(css, ':root')
    light = dict(dark)
    light.update(parse_block(css, ':root[data-theme="light"]'))
    failures = []

    print('text pairs (WCAG AA 4.5:1)')
    for name, theme in (('dark', dark), ('light', light)):
        failures += check(theme, TEXT_PAIRS, 4.5, name)
        selected = flattens(theme, 'primary-soft', 'bg')
        value = ratio(colour(theme, 'muted'), selected)
        mark = 'PASS' if value >= 4.5 else 'FAIL'
        print(f'  {name:7s} {"muted":14s} on {"primary-soft":12s} {value:6.2f}:1  {mark}  (selected device row)')
        if value < 4.5:
            failures.append(('muted', 'primary-soft', value))
        value = ratio(colour(theme, 'ink'), colour(theme, 'surface'))
        mark = 'PASS' if value >= 4.5 else 'FAIL'
        print(f'  {name:7s} {"ink":14s} on {"surface":12s} {value:6.2f}:1  {mark}  (error banner)')
        if value < 4.5:
            failures.append(('ink', 'surface', value))
        for base_name, base in (('white frame', WHITE), ('mid frame', MID)):
            scrim = over(colour(theme, 'scrim'), base)
            value = ratio(colour(theme, 'on-scrim'), scrim)
            mark = 'PASS' if value >= 4.5 else 'FAIL'
            print(f'  {name:7s} {"on-scrim":14s} on {("scrim/" + base_name):12s} {value:6.2f}:1  {mark}')
            if value < 4.5:
                failures.append(('on-scrim', 'scrim over ' + base_name, value))

    print('graphic pairs (WCAG 1.4.11 3:1)')
    for name, theme in (('dark', dark), ('light', light)):
        failures += check(theme, GRAPHIC_PAIRS, 3.0, name)

    if failures:
        print(f'\n{len(failures)} pair(s) below the floor')
        return 1
    print('\nall pairs clear their floor')
    return 0


if __name__ == '__main__':
    sys.exit(main())
