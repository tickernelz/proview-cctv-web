import { api } from '../lib/api'
import { live } from '../live/store'
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

export async function loadLens(deviceId: string, channel: number) {
  state.lens = null
  renderLensPicker()
  if (state.lensSupport === 'absent') return
  try {
    const payload = await api('/api/lens?device=' + encodeURIComponent(deviceId) + '&channel=' + encodeURIComponent(channel))
    state.lensSupport = 'present'
    state.lens = normaliseLens(payload, 'contract')
    renderLensPicker()
    applyLensView(state.lensView)
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

export function lensEntries(): LensEntry[] {
  return state.lens && state.lens.lenses.length ? state.lens.lenses : []
}

export function lensViewLabel() {
  if (state.lensView === 'combined') return 'Both lenses'
  const entry = lensEntries().find((lens) => String(lens.index) === String(state.lensView))
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
  if (!pair && state.lensView !== 'combined') state.lensView = 'combined'
  live.lens.shown = pair
  live.lens.label = lensViewLabel()
  live.settings.lensNote = lensNote()
  live.settings.lensTitle = state.lens && state.lens.ptzEvidence ? state.lens.ptzEvidence : ''
  live.settings.lensRolesKnown = entries.some((entry) => entry.role !== 'unknown')
  live.lens.options = [{ value: 'combined', name: 'Both lenses', sub: 'the frame as the camera sends it' }].concat(
    entries.map((entry) => ({ value: String(entry.index), name: entry.label, sub: lensSub(entry) })),
  )
  live.lens.value = String(state.lensView)
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
  const order = ['combined'].concat(entries.map((entry) => String(entry.index)))
  chooseLens(order[(order.indexOf(String(state.lensView)) + 1) % order.length])
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
  if (!well || !(state.stage.name === 'playing' || state.stage.name === 'stalled')) return
  const dims = frameDims()
  const crop = currentCrop()
  const ratio = (crop.w * dims.width) / (crop.h * dims.height)
  const available = { width: well.clientWidth || 1, height: well.clientHeight || 1 }
  let width = available.width
  let height = width / ratio
  if (height > available.height) {
    height = available.height
    width = height * ratio
  }
  live.stage.crop.width = Math.max(1, Math.round(width)) + 'px'
  live.stage.crop.height = Math.max(1, Math.round(height)) + 'px'
  live.stage.crop.flipped = state.prefs.flip ? 'true' : 'false'
}

export function applyLensView(value: string | number) {
  state.lensView = value
  const crop = currentCrop()
  live.stage.view.left = (-crop.x / crop.w) * 100 + '%'
  live.stage.view.top = (-crop.y / crop.h) * 100 + '%'
  live.stage.view.width = 100 / crop.w + '%'
  live.stage.view.height = 100 / crop.h + '%'
  live.stage.lens = String(value)
  layoutLensCrop()
  renderFsLine()
}

export function currentCrop(): Crop {
  if (state.lensView === 'combined') return FULL
  const entry = lensEntries().find((lens) => String(lens.index) === String(state.lensView))
  return entry ? entry.crop : FULL
}

window.addEventListener('resize', () => layoutLensCrop())
