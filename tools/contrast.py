#!/usr/bin/env python3
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TOKENS = ROOT / 'apps' / 'web' / 'src' / 'tokens.css'

TEXT = 'text'
GRAPHIC = 'graphic'
WHITE = (1.0, 1.0, 1.0, 1.0)

PAIRS = [
    ('ink on page', 'ink', 'bg', TEXT, 4.5),
    ('ink on bar', 'ink', 'bg-2', TEXT, 4.5),
    ('ink on hover', 'ink', 'bg-3', TEXT, 4.5),
    ('ink-2 on page', 'ink-2', 'bg', TEXT, 4.5),
    ('ink-2 on bar', 'ink-2', 'bg-2', TEXT, 4.5),
    ('ink-2 on hover', 'ink-2', 'bg-3', TEXT, 4.5),
    ('ink-3 on page', 'ink-3', 'bg', TEXT, 4.5),
    ('ink-3 on bar', 'ink-3', 'bg-2', TEXT, 4.5),
    ('page on ink (filled button label)', 'bg', 'ink', TEXT, 4.5),
    ('stage-ink on stage', 'stage-ink', 'stage', TEXT, 4.5),
    ('stage-ink-2 on stage', 'stage-ink-2', 'stage', TEXT, 4.5),
    ('stage-ink on matte over a white frame', 'stage-ink', 'stage-matte@white', TEXT, 4.5),
    ('stage-ink-2 on matte over a white frame', 'stage-ink-2', 'stage-matte@white', TEXT, 4.5),
    ('live on page', 'live', 'bg', TEXT, 4.5),
    ('live on live-soft over page', 'live', 'live-soft@bg', TEXT, 4.5),
    ('ok on page', 'ok', 'bg', TEXT, 4.5),
    ('bad on page', 'bad', 'bg', TEXT, 4.5),
    ('warn on page', 'warn', 'bg', TEXT, 4.5),
    ('ok dot on page', 'ok', 'bg', GRAPHIC, 3.0),
    ('live dot on page', 'live', 'bg', GRAPHIC, 3.0),
    ('warn icon on bar', 'warn', 'bg-2', GRAPHIC, 3.0),
    ('line on page', 'line', 'bg', GRAPHIC, 1.6),
    ('line-soft on page', 'line-soft', 'bg', GRAPHIC, 1.4),
    ('focus ring on page', 'focus', 'bg', GRAPHIC, 3.0),
    ('focus ring on bar', 'focus', 'bg-2', GRAPHIC, 3.0),
    ('focus ring on stage', 'focus', 'stage', GRAPHIC, 3.0),
]


def parse_block(css, selector):
    found = re.search(re.escape(selector) + r'\s*\{([^}]*)\}', css)
    if not found:
        raise SystemExit(f'tokens.css has no {selector} block')
    return dict(re.findall(r'--([a-z0-9-]+):\s*([^;]+);', found.group(1)))


def parse_colour(raw):
    raw = raw.strip()
    hexed = re.fullmatch(r'#([0-9a-fA-F]{6})', raw)
    if hexed:
        value = hexed.group(1)
        return tuple(int(value[i:i + 2], 16) / 255 for i in (0, 2, 4)) + (1.0,)
    rgb = re.fullmatch(r'rgb\(\s*(\d+)\s+(\d+)\s+(\d+)\s*(?:/\s*([0-9.]+)\s*)?\)', raw)
    if rgb:
        alpha = float(rgb.group(4)) if rgb.group(4) else 1.0
        return tuple(int(rgb.group(i)) / 255 for i in (1, 2, 3)) + (alpha,)
    raise SystemExit(f'unsupported colour value: {raw}')


def over(top, bottom):
    return tuple(top[i] * top[3] + bottom[i] * (1 - top[3]) for i in range(3)) + (1.0,)


def luminance(rgb):
    def linear(channel):
        return channel / 12.92 if channel <= 0.04045 else ((channel + 0.055) / 1.055) ** 2.4

    r, g, b = (linear(c) for c in rgb[:3])
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def ratio(foreground, background):
    high, low = sorted((luminance(foreground), luminance(background)), reverse=True)
    return (high + 0.05) / (low + 0.05)


def hexof(rgb):
    return '#' + ''.join(f'{round(min(1, max(0, c)) * 255):02x}' for c in rgb[:3])


def resolve(theme, name):
    if '@' not in name:
        return over(theme[name], WHITE)
    top, base = name.split('@')
    return over(theme[top], WHITE if base == 'white' else theme[base])


def main():
    css = TOKENS.read_text()
    shared = {k: parse_colour(v) for k, v in parse_block(css, ':root').items() if v.strip().startswith(('#', 'rgb('))}
    failures = 0
    rows = 0
    for name in ('dark', 'light'):
        raw = parse_block(css, f':root[data-theme="{name}"]')
        theme = dict(shared)
        theme.update({k: parse_colour(v) for k, v in raw.items() if v.strip().startswith(('#', 'rgb('))})
        for label, fg_name, bg_name, kind, floor in PAIRS:
            background = resolve(theme, bg_name)
            foreground = over(theme[fg_name], background)
            value = ratio(foreground, background)
            verdict = 'PASS' if value >= floor else 'FAIL'
            failures += verdict == 'FAIL'
            rows += 1
            print(f'  {name:<6} {kind:<8} {label:<40} {hexof(foreground)} on {hexof(background)}  {value:5.2f}:1  floor {floor:.1f}  {verdict}')
    if failures:
        print(f'{failures} of {rows} pairs are under their floor')
        return 1
    print(f'all {rows} pairs clear their floor')
    return 0


if __name__ == '__main__':
    sys.exit(main())
