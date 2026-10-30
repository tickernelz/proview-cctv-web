import { api } from '../lib/api'
import { live } from '../live/store'
import { frameFit, pip } from '../live/stage/pip'
import { renderCaption } from './caption'
import { renderSwitcher } from './chrome'
import { sessionLost } from './shell'
import { renderFsLine } from './stage'
import { state } from './state'

type Crop = { x: number; y: number; w: number; h: number }
type Stream = { codec?: string; width?: number; height?: number }
export type LensEntry = { index: number; label: string; rawLabel: string; role: string; crop: Crop }
export type Lens = {
  deviceId: string
  channel: number
  streams: Stream[]
  countSource: string | null
  cropsVerified: boolean
  ptzSource: string | null
  ptzEvidence: string | null
  lenses: LensEntry[]
  source: string
}

const FULL: Crop = { x: 0, y: 0, w: 1, h: 1 }
export const PIP = 'pip'
const LENS_WORDS: Record<string, string> = { full: 'Full frame', top: 'Top lens', middle: 'Middle lens', bottom: 'Bottom lens' }

const video = () => document.getElementById('video') as HTMLVideoElement | null

function normaliseCrop(crop: Partial<Crop> | null | undefined, dims: { width: number; height: number }): Crop {
  const safe = crop || {}
  const raw = { x: Number(safe.x) || 0, y: Number(safe.y) || 0, w: Number(safe.w) || 1, h: Number(safe.h) || 1 }
  if (!(raw.w > 1 || raw.h > 1)) return raw
  return { x: raw.x / dims.width, y: raw.y / dims.height, w: raw.w / dims.width, h: raw.h / dims.height }
}

function lensLabel(lens: any, index: number) {
  const raw = typeof lens.label === 'string' ? lens.label : ''
  return LENS_WORDS[raw.toLowerCase()] || raw || 'Lens ' + (index + 1)
}

function lensRole(lens: any) {
  if (lens.ptz === true) return 'moving'
  if (lens.ptz === false) return 'fixed'
  return lens.role === 'moving' || lens.role === 'fixed' ? lens.role : 'unknown'
}

function normaliseLens(payload: any, source: string): Lens {
  const streams: Stream[] = Array.isArray(payload && payload.streams) ? payload.streams : []
  const dims = { width: streams[0] && streams[0].width ? streams[0].width : 1, height: streams[0] && streams[0].height ? streams[0].height : 1 }
  const lenses: any[] = Array.isArray(payload && payload.lenses) ? payload.lenses : []
  return {
    deviceId: payload && payload.deviceId ? payload.deviceId : state.selectedId,
    channel: payload && payload.channel ? payload.channel : state.channel,
    streams,
    countSource: payload && payload.countSource ? payload.countSource : null,
    cropsVerified: Boolean(payload && payload.cropsVerified === true),
    ptzSource: payload && payload.ptzSource ? payload.ptzSource : null,
    ptzEvidence: payload && payload.ptzEvidence ? payload.ptzEvidence : null,
    lenses: lenses.map((lens, index) => ({
      index: typeof lens.index === 'number' ? lens.index : index,
      label: lensLabel(lens, index),
      rawLabel: typeof lens.label === 'string' ? lens.label.toLowerCase() : '',
      role: lensRole(lens),
      crop: normaliseCrop(lens.crop, dims),
    })),
    source,
  }
}

function contractCaption(lens: Lens) {
  const countRule = lens.countSource === 'capability' ? 'reported by the device capabilities' : lens.countSource === 'geometry' ? 'inferred from the frame ratio' : 'reported by the device'
  const size = lens.streams[0] ? lens.streams[0].width + '×' + lens.streams[0].height : 'unknown size'
  const moving = lens.lenses.find((entry) => entry.role === 'moving')
  const fixed = lens.lenses.find((entry) => entry.role === 'fixed')
  const roles: string[] = []
  if (moving) roles.push(moving.label.toLowerCase() + ' can pan and tilt')
  if (fixed) roles.push(fixed.label.toLowerCase() + ' is aimed by hand')
  const motor = lens.ptzSource === 'device' ? 'the device names the motor' : lens.ptzSource === 'measured_default' ? 'the motor lens is assumed, the device has not answered' : ''
  return (
    'Lens layout: ' + lens.lenses.length + ' lens' + (lens.lenses.length === 1 ? '' : 'es') + ', ' + countRule + ' (' + size + ')' +
    (roles.length ? ' · ' + roles.join(' · ') : '') + (motor ? ' · ' + motor : '') + '.'
  )
}

export function deriveLensFromFrame() {
  const frame = video()
  const width = frame ? frame.videoWidth : 0
  const height = frame ? frame.videoHeight : 0
  if (!width || !height) return null
  const streams = [{ codec: '', width, height }]
  if (height / width <= 0.9) {
    return normaliseLens(
      { deviceId: state.selectedId, channel: state.channel, streams, lenses: [{ index: 0, label: 'Single lens', role: 'unknown', crop: FULL }] },
      'geometry',
    )
  }
  return normaliseLens(
    {
      deviceId: state.selectedId,
      channel: state.channel,
      streams,
      lenses: [
        { index: 0, label: 'top', crop: { x: 0, y: 0, w: 1, h: 0.5 } },
        { index: 1, label: 'bottom', crop: { x: 0, y: 0.5, w: 1, h: 0.5 } },
      ],
    },
    'geometry',
  )
}

function knownLayout(payload: any) {
  if (!payload || !Array.isArray(payload.lenses) || payload.lenses.length === 0) return false
  if (payload.countSource === 'unknown') return false
  return typeof payload.lensCount !== 'number' || payload.lensCount > 0
}

function lensQuery(deviceId: string, channel: number) {
  return '/api/lens?device=' + encodeURIComponent(deviceId) + '&channel=' + encodeURIComponent(channel)
}

function useLayout(lens: Lens) {
  state.lens = lens
  renderLensPicker()
  applyLensView(state.lensView)
}

export async function loadLens(deviceId: string, channel: number) {
  state.lens = null
  renderLensPicker()
  if (state.lensSupport === 'absent') return
  try {
    const payload = await api(lensQuery(deviceId, channel))
    state.lensSupport = 'present'
    if (state.selectedId !== deviceId || state.channel !== channel) return
    if (knownLayout(payload)) useLayout(normaliseLens(payload, 'contract'))
  } catch (error: any) {
    if (error.status === 404 || error.status === 501) {
      state.lensSupport = 'absent'
      renderLensPicker()
      return
    }
    if (sessionLost(error)) return
    console.error('lens contract failed', error)
    live.settings.lensNote = 'The server answered ' + error.status + ' for GET /api/lens.'
  }
}

export async function resolveLensFromFrame() {
  const deviceId = state.selectedId
  const channel = state.channel
  const frame = video()
  if (state.lens || !deviceId || !frame || !frame.videoWidth || !frame.videoHeight) return
  const width = frame.videoWidth
  const height = frame.videoHeight
  if (state.lensSupport === 'present') {
    try {
      const payload = await api(lensQuery(deviceId, channel) + '&width=' + width + '&height=' + height)
      if (state.lens || state.selectedId !== deviceId || state.channel !== channel) return
      if (knownLayout(payload)) {
        useLayout(normaliseLens(payload, 'contract'))
        return
      }
    } catch (error: any) {
      if (sessionLost(error)) return
      console.error('lens contract with the frame size failed', error)
    }
  }
  if (state.lens || state.selectedId !== deviceId || state.channel !== channel) return
  const derived = deriveLensFromFrame()
  if (derived) useLayout(derived)
}

export function lensEntries(): LensEntry[] {
  return state.lens && state.lens.lenses.length ? state.lens.lenses : []
}

export function lensViewValue() {
  return activeView()
}

function activeView() {
  const view = String(state.lensView)
  if (lensEntries().length < 2) return 'combined'
  if (view === PIP || view === 'combined') return view
  return lensEntries().some((lens) => String(lens.index) === view) ? view : 'combined'
}

function pipPair(): [LensEntry, LensEntry] {
  const entries = lensEntries()
  const top = entries.find((lens) => lens.rawLabel === 'top') || entries[0]
  const bottom = entries.find((lens) => lens !== top && lens.rawLabel === 'bottom') || entries.find((lens) => lens !== top) || entries[1]
  return pip.swapped ? [bottom, top] : [top, bottom]
}

export function lensViewLabel() {
  const view = activeView()
  if (view === PIP) return 'Picture in picture'
  if (view === 'combined') return 'Both lenses'
  const entry = lensEntries().find((lens) => String(lens.index) === view)
  return entry ? entry.label : 'Both lenses'
}

function moveReason() {
  if (live.gate.move.ok) return ''
  const why = (live.gate.move.why || '').trim()
  return why ? 'Move is off: ' + why : 'Move is off for this camera.'
}

function lensNote() {
  if (!state.selectedId) return 'Pick a camera.'
  if (!state.lens) return 'Waiting for the first frame.'
  const blocked = moveReason()
  const base =
    state.lens.source === 'contract'
      ? contractCaption(state.lens).replace(/^Lens layout: /, '')
      : (() => {
          const dims = state.lens.streams[0].width + '×' + state.lens.streams[0].height
          return state.lens.lenses.length > 1
            ? 'Split inferred from the frame shape (' + dims + '). The server has not reported the lens layout yet.'
            : 'One picture (' + dims + '), no stacked lens.'
        })()
  return blocked ? base + ' ' + blocked : base
}

function lensSub(entry: LensEntry) {
  if (entry.role === 'moving') return 'pan and tilt from here'
  if (entry.role === 'fixed') return 'fixed, aimed by hand'
  return entry.rawLabel === 'top' ? 'upper half of the frame' : entry.rawLabel === 'bottom' ? 'lower half of the frame' : 'one part of the frame'
}

export function renderLensPicker() {
  const entries = lensEntries()
  const pair = entries.length > 1
  live.lens.shown = pair
  live.lens.label = lensViewLabel()
  live.settings.lensNote = lensNote()
  live.settings.lensTitle = state.lens && state.lens.ptzEvidence ? state.lens.ptzEvidence : ''
  live.settings.lensRolesKnown = entries.some((entry) => entry.role !== 'unknown')
  const pipOption = pair ? [{ value: PIP, name: 'Picture in picture', sub: 'one lens large, the other in a corner' }] : []
  live.lens.options = pipOption.concat(
    [{ value: 'combined', name: 'Both lenses', sub: 'stacked, as the camera sends the frame' }],
    entries.map((entry) => ({ value: String(entry.index), name: entry.label, sub: lensSub(entry) })),
  )
  live.lens.value = activeView()
  live.lens.note = lensNote()
  renderSwitcher()
  renderCaption()
}

export function chooseLens(value: string) {
  applyLensView(value)
  renderLensPicker()
}

export function cycleLens() {
  const entries = lensEntries()
  if (entries.length < 2) return
  const order = [PIP, 'combined'].concat(entries.map((entry) => String(entry.index)))
  chooseLens(order[(order.indexOf(activeView()) + 1) % order.length])
}

export function swapLenses() {
  if (activeView() !== PIP) return
  pip.swapped = !pip.swapped
  applyLensView(PIP)
  renderLensPicker()
}

function frameDims() {
  const frame = video()
  const streams = state.lens && state.lens.streams.length ? state.lens.streams : null
  return {
    width: streams && streams[0].width ? streams[0].width : (frame && frame.videoWidth) || 16,
    height: streams && streams[0].height ? streams[0].height : (frame && frame.videoHeight) || 9,
  }
}

export function layoutLensCrop() {
  const well = document.getElementById('well')
  if (!well || !['playing', 'stalled', 'playback'].includes(state.stage.name)) return
  const dims = frameDims()
  const crop = currentCrop()
  const ratio = (crop.w * dims.width) / (crop.h * dims.height)
  const narrow = typeof window.matchMedia === 'function' && window.matchMedia('(max-width: 640px)').matches && !document.fullscreenElement
  const width0 = well.clientWidth || 1
  frameFit.height = narrow ? Math.round(Math.min(width0 / ratio, window.innerHeight * 0.62)) + 'px' : ''
  const available = { width: width0, height: narrow ? Math.round(Math.min(width0 / ratio, window.innerHeight * 0.62)) : well.clientHeight || 1 }
  let width = available.width
  let height = width / ratio
  if (height > available.height) {
    height = available.height
    width = height * ratio
  }
  live.stage.crop.width = Math.max(1, Math.round(width)) + 'px'
  live.stage.crop.height = Math.max(1, Math.round(height)) + 'px'
  live.stage.crop.flipped = state.prefs.flip ? 'true' : 'false'
  renderPip(dims)
}

function renderPip(dims: { width: number; height: number }) {
  const on = activeView() === PIP
  pip.shown = on
  if (!on) return
  const [main, inset] = pipPair()
  pip.main = main.label
  pip.inset = inset.label
  pip.crop = { ...inset.crop }
  pip.ratio = (inset.crop.w * dims.width) / (inset.crop.h * dims.height)
}

export function applyLensView(value: string | number) {
  state.lensView = value
  const crop = currentCrop()
  live.stage.view.left = (-crop.x / crop.w) * 100 + '%'
  live.stage.view.top = (-crop.y / crop.h) * 100 + '%'
  live.stage.view.width = 100 / crop.w + '%'
  live.stage.view.height = 100 / crop.h + '%'
  live.stage.lens = activeView()
  pip.shown = false
  layoutLensCrop()
  renderFsLine()
}

export function currentCrop(): Crop {
  const view = activeView()
  if (view === 'combined') return FULL
  if (view === PIP) return pipPair()[0].crop
  const entry = lensEntries().find((lens) => String(lens.index) === view)
  return entry ? entry.crop : FULL
}

window.addEventListener('resize', () => layoutLensCrop())
