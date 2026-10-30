import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import type { Plugin } from 'vite'

export const ICONS: Record<string, string> = {
  mic: 'mic',
  speaker: 'volume-2',
  'speaker-off': 'volume-x',
  record: 'circle-dot',
  camera: 'camera',
  expand: 'maximize',
  gear: 'settings',
  grid: 'layout-grid',
  frame: 'square',
  'chevron-down': 'chevron-down',
  'chevron-up': 'chevron-up',
  sliders: 'sliders-horizontal',
  'chevron-left': 'chevron-left',
  'chevron-right': 'chevron-right',
  move: 'move',
  stop: 'circle-stop',
  check: 'check',
  warning: 'triangle-alert',
  info: 'info',
  refresh: 'refresh-cw',
  'arrow-left': 'arrow-left',
  x: 'x',
  sun: 'sun',
  moon: 'moon',
  clock: 'clock',
  history: 'history',
  disconnect: 'wifi-off',
  more: 'ellipsis',
  bell: 'bell',
  'no-picture': 'image-off',
  download: 'download',
  cloud: 'cloud',
  cctv: 'cctv',
  plus: 'plus',
  play: 'play',
  pause: 'pause',
  calendar: 'calendar-days',
  loader: 'loader-circle',
  alert: 'circle-alert',
  layers: 'layers',
  swap: 'repeat-2',
  flip: 'flip-vertical-2',
  lightbulb: 'lightbulb',
  'low-light': 'sun-dim',
  'zoom-in': 'zoom-in',
  'zoom-out': 'zoom-out',
  focus: 'focus',
  route: 'route',
  pencil: 'pencil',
  crosshair: 'crosshair',
}

const SPRITE = /<svg class="sprite"[^>]*>[\s\S]*?<\/svg>/

export function lucideSprite(): string {
  const icons = join(dirname(createRequire(import.meta.url).resolve('lucide-static/package.json')), 'icons')
  const symbols = Object.entries(ICONS).map(([name, lucide]) => {
    const svg = readFileSync(join(icons, lucide + '.svg'), 'utf8')
    const body = svg.slice(svg.indexOf('>', svg.indexOf('<svg')) + 1, svg.lastIndexOf('</svg>'))
    return '<symbol id="i-' + name + '" viewBox="0 0 24 24">' + body.replace(/\s*\n\s*/g, '') + '</symbol>'
  })
  return '<svg class="sprite" aria-hidden="true" focusable="false"><defs>' + symbols.join('') + '</defs></svg>'
}

export const iconSprite = (): Plugin => ({
  name: 'proview-icon-sprite',
  transformIndexHtml(html) {
    if (!SPRITE.test(html)) throw new Error('index.html has no <svg class="sprite"> to replace')
    return html.replace(SPRITE, lucideSprite())
  },
})
