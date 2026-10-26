import math

def oklch_to_srgb(L, C, h):
    h = math.radians(h)
    a = C * math.cos(h)
    b = C * math.sin(h)
    l_ = L + 0.3963377774*a + 0.2158037573*b
    m_ = L - 0.1055613458*a - 0.0638541728*b
    s_ = L - 0.0894841775*a - 1.2914855480*b
    l, m, s = l_**3, m_**3, s_**3
    r = +4.0767416621*l - 3.3077115913*m + 0.2309699292*s
    g = -1.2684380046*l + 2.6097574011*m - 0.3413193965*s
    bb = -0.0041960863*l - 0.7034186147*m + 1.7076147010*s
    def enc(x):
        x = max(0.0, min(1.0, x))
        return 12.92*x if x <= 0.0031308 else 1.055*(x**(1/2.4)) - 0.055
    return tuple(enc(v) for v in (r, g, bb))

def lum(rgb):
    def lin(c):
        return c/12.92 if c <= 0.04045 else ((c+0.055)/1.055)**2.4
    r, g, b = (lin(c) for c in rgb)
    return 0.2126*r + 0.7152*g + 0.0722*b

def ratio(fg, bg):
    l1, l2 = lum(fg), lum(bg)
    hi, lo = max(l1, l2), min(l1, l2)
    return (hi+0.05)/(lo+0.05)

def hexs(rgb):
    return '#%02x%02x%02x' % tuple(round(c*255) for c in rgb)

dark = {
  'bg':      (0.16, 0.008, 265),
  'surface': (0.21, 0.010, 265),
  'surface2':(0.26, 0.012, 265),
  'ink':     (0.96, 0.004, 265),
  'muted':   (0.74, 0.012, 265),
  'primary': (0.62, 0.190, 27),
}
light = {
  'bg':      (0.985, 0.002, 265),
  'surface': (1.000, 0.000, 0),
  'surface2':(0.962, 0.004, 265),
  'ink':     (0.22, 0.012, 265),
  'muted':   (0.50, 0.014, 265),
  'primary': (0.52, 0.190, 27),
}
for name, p in (('dark', dark), ('light', light)):
    print('==', name)
    for k, v in p.items():
        print(f'  {k:9s} oklch({v[0]} {v[1]} {v[2]})  {hexs(oklch_to_srgb(*v))}')
    bg = oklch_to_srgb(*p['bg'])
    for k in ('ink','muted','primary'):
        r_ = ratio(oklch_to_srgb(*p[k]), bg)
        verdict = 'PASS' if r_ >= 4.5 else ('large-only' if r_ >= 3 else 'FAIL')
        print(f'  {k:9s} on bg: {r_:.2f}:1  {verdict}')
