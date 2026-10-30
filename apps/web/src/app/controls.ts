import { api } from '../lib/api'
import { live, type Gate } from '../live/store'
import { renderCaption } from './caption'
import { selectedDevice } from './devices'
import { currentCrop, lensEntries, renderLensPicker } from './lens'
import { sessionLost } from './shell'
import { state, type Device } from './state'

const UNCHECKED = 'Device controls have not been checked yet.'

const CONTROL_FEATURE: Record<string, string> = {
  up: 'ptz move',
  down: 'ptz move',
  left: 'ptz move',
  right: 'ptz move',
  home: 'ptz move',
  presets: 'ptz preset',
  strobe: 'white light',
  cruise: 'ptz patrol',
  intercom: 'two-way audio',
  move: 'ptz move',
  aim: 'ptz aim',
  zoom: 'ptz zoom',
  focus: 'ptz focus',
  presetEdit: 'ptz preset edit',
  calibrate: 'ptz calibrate',
}

const CONTROL_ABSENT: Record<string, string> = {
  tracking: 'No auto-tracking call exists on this server, and the capability report lists none for this device.',
  alarm: 'No audio-alarm call exists on this server; the device reports white light and two-way audio, not an alarm.',
  'cloud-storage': 'No server call for cloud storage in this build: the cloud API exposes the device list only.',
  'sd-album': 'No server call for the SD card album in this build: the cloud API exposes the device list only.',
}

const MOVES: Record<string, [number, number]> = { up: [0, -0.5], down: [0, 0.5], left: [-0.5, 0], right: [0.5, 0] }

function deviceQuery(separator: '?' | '&') {
  const id = state.selectedId || ''
  return id ? separator + 'device=' + encodeURIComponent(id) : ''
}

function movingLensName() {
  const entry = lensEntries().find((lens) => lens.role === 'moving')
  const raw = entry ? entry.rawLabel : ''
  return raw === 'top' || raw === 'bottom' ? raw : 'bottom'
}

function capabilityRow(feature: string, lens: string | null) {
  const rows = state.control.rows.filter((row) => row.feature === feature)
  if (lens) {
    const exact = rows.find((row) => row.lens === lens)
    if (exact) return exact
  }
  return rows.find((row) => row.state === 'supported') || rows[0] || null
}

function verdict(act: string): Gate {
  if (!state.control.rows.length) return { ok: false, why: state.control.error || UNCHECKED }
  const feature = CONTROL_FEATURE[act]
  if (!feature) return { ok: false, why: CONTROL_ABSENT[act] || 'No server call exists for this action in this build.' }
  const lens = feature === 'ptz move' || feature === 'ptz preset' ? movingLensName() : null
  const row = capabilityRow(feature, lens)
  if (!row) return { ok: false, why: 'The capability report does not list "' + feature + '" for this device.' }
  if (row.state !== 'supported') return { ok: false, why: row.evidence || 'The device reports "' + feature + '" as blocked.' }
  if (act === 'home' && !state.control.onvif) return { ok: false, why: 'Stopping pan and tilt is served by the ONVIF channel, which this server does not expose.' }
  return { ok: true, why: row.evidence || '' }
}

function renderCapabilities() {
  const rows = state.control.rows.map((row) => [row.feature, row.lens, row.backend, row.state, row.evidence])
  if (rows.length) {
    for (const [feature, act] of [['auto-tracking', 'tracking'], ['audio alarm', 'alarm'], ['cloud storage', 'cloud-storage'], ['sd card album', 'sd-album']]) {
      rows.push([feature, '—', 'none', 'unavailable', CONTROL_ABSENT[act]])
    }
  }
  live.settings.capabilities = rows.map((row) => row.map((cell) => (cell == null ? '' : String(cell))))
}

export function renderControlAvailability() {
  const rows = state.control.rows
  if (state.control.error) {
    live.settings.controlNote = 'Device controls could not be read: ' + state.control.error
  } else if (!rows.length) {
    live.settings.controlNote = UNCHECKED
  } else {
    const supported = rows.filter((row) => row.state === 'supported').length
    const backends = [state.control.onvif ? 'ONVIF' : null, state.control.dvrip ? 'DVRIP' : null].filter(Boolean)
    live.settings.controlNote = supported + ' of ' + rows.length + ' rows are usable' + (backends.length ? ' over ' + backends.join(' and ') : '') + '. Blocked rows keep the device\u2019s own reason.'
  }
  live.settings.light = verdict('strobe')
  for (const act of Object.keys(live.gate)) live.gate[act] = verdict(act)
  if (state.padOpen && !live.gate.move.ok) closePad()
  renderCapabilities()
  renderLensPicker()
}

export async function probeControlChannel(device: Device) {
  if (state.control.checkedFor === device.id) {
    renderControlAvailability()
    return
  }
  try {
    const payload = await api('/api/device/controls?device=' + encodeURIComponent(device.id))
    state.control = {
      rows: Array.isArray(payload.capabilities) ? payload.capabilities : [],
      onvif: payload.onvif === true,
      dvrip: payload.dvrip === true,
      error: null,
      checkedFor: device.id,
    }
  } catch (error: any) {
    if (sessionLost(error)) return
    state.control = { rows: [], onvif: false, dvrip: false, error: error.status + ' ' + error.message, checkedFor: null }
  }
  renderControlAvailability()
}

export function setControlStatus(message: string) {
  live.pad.status = message || ''
}

type DevicePreset = { channel?: number; id: number; name?: string }
type DeviceTourLine = { channel: number; id: number; name: string; points: { preset: number; seconds: number }[] }

const LENS_ACTIONS = new Set(['zoom-in', 'zoom-out', 'focus-near', 'focus-far'])

function showPresets(presets: { token: string; name?: string }[]) {
  live.pad.presets = presets.map((preset) => ({ token: preset.token, name: preset.name || preset.token }))
  live.pad.presetsShown = presets.length > 0
}

function editLines(lines: DeviceTourLine[]) {
  live.pad.edit.lines = lines.map((line) => ({ channel: line.channel, id: line.id, name: line.name, points: line.points.map((point) => point.preset) }))
}

function showEditListing(presets: DevicePreset[], lines: DeviceTourLine[]) {
  live.pad.edit.presets = presets.map((preset) => ({ token: String(preset.id), name: preset.name || 'Preset' + preset.id }))
  editLines(lines)
  live.pad.edit.loaded = true
}

async function loadPresets() {
  live.pad.presets = []
  live.pad.presetsShown = false
  const row = capabilityRow('ptz preset', movingLensName())
  const editable = live.gate.presetEdit.ok
  if (!editable && (!row || row.state !== 'supported')) return
  try {
    showPresets(await api('/api/device/controls/presets' + deviceQuery('?')))
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus('Presets could not be read: ' + error.status + ' ' + error.message)
  }
}

async function editCall(path: string, done: (body: any) => string) {
  if (live.pad.busy) return
  live.pad.busy = true
  try {
    const body = await api(path + deviceQuery(path.includes('?') ? '&' : '?'), { method: 'POST' })
    if (Array.isArray(body.presets)) {
      showEditListing(body.presets, Array.isArray(body.lines) ? body.lines : [])
      loadPresets()
    } else if (Array.isArray(body.lines)) {
      editLines(body.lines)
    }
    setControlStatus(done(body))
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus('The camera refused: ' + error.status + ' ' + error.message)
  } finally {
    live.pad.busy = false
  }
}

async function readEditListing(what: string) {
  try {
    const tour = await api('/api/device/controls/tour' + deviceQuery('?'))
    showEditListing(Array.isArray(tour.presets) ? tour.presets : [], Array.isArray(tour.lines) ? tour.lines : [])
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus(what + ' could not be read: ' + error.status + ' ' + error.message)
  }
}

export async function openEditor() {
  live.pad.editing = !live.pad.editing
  if (!live.pad.editing) return
  live.pad.edit.loaded = false
  live.pad.edit.presets = []
  live.pad.edit.lines = []
  await readEditListing('Presets and tours')
}

export function addPreset(name: string) {
  return editCall('/api/device/controls/preset/add?name=' + encodeURIComponent(name.trim()), (body) => 'Saved this position as ' + body.name + ' (preset ' + body.id + ').')
}

export function renamePreset(token: string, name: string) {
  if (!name.trim()) {
    setControlStatus('A preset name cannot be blank.')
    return Promise.resolve()
  }
  return editCall('/api/device/controls/preset/rename?token=' + encodeURIComponent(token) + '&name=' + encodeURIComponent(name.trim()), (body) => 'Renamed preset ' + body.id + ' to ' + body.name + '.')
}

export function deletePreset(token: string, name: string) {
  if (!window.confirm('Are you sure you want to delete this preset? (' + name + ')')) return Promise.resolve()
  return editCall('/api/device/controls/preset/delete?token=' + encodeURIComponent(token), (body) => 'Deleted ' + body.name + '.')
}

export function addTourStop(tour: number, preset: string, index: number) {
  return editCall('/api/device/controls/tour/add?tour=' + tour + '&preset=' + encodeURIComponent(preset) + '&index=' + index, () => 'Added the stop to the tour.').then(() => readEditListing('The tour'))
}

export function removeTourStop(tour: number, preset: number) {
  return editCall('/api/device/controls/tour/remove?tour=' + tour + '&preset=' + preset, () => 'Removed the stop from the tour.')
}

export function clearTour(tour: number) {
  if (!window.confirm('Remove every stop from this tour?')) return Promise.resolve()
  return editCall('/api/device/controls/tour/clear?tour=' + tour, () => 'Cleared the tour.').then(() => readEditListing('The tour'))
}

export function calibrate() {
  if (!window.confirm('Calibrate the PTZ motor? The lens sweeps its full range and the picture moves for up to a minute.')) return Promise.resolve()
  return editCall('/api/device/controls/ptz/calibrate', (body) => 'Calibration finished in ' + body.calibrate_ms + ' ms.')
}

export function holdsAction(action: string) {
  return Boolean(state.control.dvrip) && (!state.control.onvif || LENS_ACTIONS.has(action))
}

let held: string | null = null
let pressedAt = 0
let pressing: Promise<boolean> = Promise.resolve(false)

export function pressControl(action: string) {
  if (!selectedDevice()) return
  if (!holdsAction(action)) {
    sendControl(action)
    return
  }
  if (held === action) return
  if (held) releaseControl()
  held = action
  pressedAt = performance.now()
  setControlStatus('Moving ' + action + '; release to stop.')
  const started = pressedAt
  const previous = pressing
  pressing = previous
    .then(() => api('/api/device/controls/ptz/press?action=' + encodeURIComponent(action) + deviceQuery('&'), { method: 'POST' }))
    .then(() => {
      if (held === action) setControlStatus('Moving ' + action + ' (' + Math.round(performance.now() - started) + ' ms to start); release to stop.')
      return true
    })
    .catch((error: any) => {
      if (held === action) held = null
      if (!sessionLost(error)) setControlStatus('The camera refused "' + action + '": ' + error.status + ' ' + error.message)
      return false
    })
}

export async function releaseControl() {
  if (!held) return
  const action = held
  const pressedMs = Math.round(performance.now() - pressedAt)
  held = null
  const pressed = pressing
  pressing = pressed.then(async (ok) => {
    if (!ok) return false
    const started = performance.now()
    try {
      const body = await api('/api/device/controls/ptz/release?pressed_ms=' + pressedMs + deviceQuery('&'), { method: 'POST' })
      if (held === null) setControlStatus(body.held ? 'Stopped ' + action + ' after ' + pressedMs + ' ms (' + Math.round(performance.now() - started) + ' ms to stop).' : 'Stopped.')
    } catch (error: any) {
      if (!sessionLost(error)) setControlStatus('The stop for "' + action + '" failed: ' + error.status + ' ' + error.message)
    }
    return false
  })
  await pressing
}

export function releaseOnExit() {
  if (!held) return
  const pressedMs = Math.round(performance.now() - pressedAt)
  held = null
  navigator.sendBeacon('/api/device/controls/ptz/release?pressed_ms=' + pressedMs + deviceQuery('&'))
}

export function tapControl(action: string) {
  pressControl(action)
  if (holdsAction(action)) releaseControl()
}

function aimLens() {
  const entries = lensEntries()
  return entries.find((entry) => entry.role === 'fixed') || entries.find((entry) => entry.rawLabel === 'top') || null
}

export async function aimAt(event: MouseEvent) {
  if (!live.padOpen || !live.gate.aim.ok || !selectedDevice()) return
  const frame = event.currentTarget as HTMLElement
  const box = frame.getBoundingClientRect()
  if (!box.width || !box.height) return
  let x = (event.clientX - box.left) / box.width
  let y = (event.clientY - box.top) / box.height
  if (live.stage.crop.flipped === 'true') {
    x = 1 - x
    y = 1 - y
  }
  const crop = currentCrop()
  const lens = aimLens()
  if (!lens) return
  const u = (crop.x + x * crop.w - lens.crop.x) / lens.crop.w
  const v = (crop.y + y * crop.h - lens.crop.y) / lens.crop.h
  if (u < 0 || u > 1 || v < 0 || v > 1) {
    setControlStatus('Click inside the ' + lens.label.toLowerCase() + ' picture to aim the moving lens there.')
    return
  }
  try {
    const body = await api('/api/device/controls/ptz/locate?x=' + u.toFixed(4) + '&y=' + v.toFixed(4) + deviceQuery('&'), { method: 'POST' })
    setControlStatus('Aiming at ' + Math.round(u * 100) + '% across, ' + Math.round(v * 100) + '% down (offset ' + body.xoffset + ', ' + body.yoffset + ').')
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus('The camera refused the aim: ' + error.status + ' ' + error.message)
  }
}

export async function gotoPreset(token: string, name: string) {
  try {
    await api('/api/device/controls/preset/goto?token=' + encodeURIComponent(token) + deviceQuery('&'), { method: 'POST' })
    setControlStatus('At ' + name + '.')
  } catch (error: any) {
    setControlStatus('The camera refused ' + name + ': ' + error.status + ' ' + error.message)
  }
}

export async function toggleWhiteLight() {
  try {
    const current = await api('/api/device/controls/light' + deviceQuery('?'))
    const next = current.mode === 'always' ? 'off' : 'always'
    const applied = await api('/api/device/controls/light?mode=' + encodeURIComponent(next) + deviceQuery('&'), { method: 'POST' })
    live.settings.lightStatus = 'White light: ' + (applied.mode || next) + ' (it was ' + (current.mode || 'not reported') + ').'
  } catch (error: any) {
    if (!sessionLost(error)) live.settings.lightStatus = 'The camera refused the white light: ' + error.status + ' ' + error.message
  }
}

async function toggleTour() {
  try {
    if (state.tour !== null) {
      const stopped = await api('/api/device/controls/tour/stop?tour=' + encodeURIComponent(state.tour) + deviceQuery('&'), { method: 'POST' })
      state.tour = null
      live.pad.tour = false
      setControlStatus('Stopped tour ' + stopped.tour + '.')
      return
    }
    const tour = await api('/api/device/controls/tour' + deviceQuery('?'))
    const line = (tour.lines || []).find((entry: { points?: unknown[] }) => (entry.points || []).length > 0)
    if (!line) {
      setControlStatus('No tour line with points on this camera: ' + (tour.note || 'add presets to a tour first'))
      return
    }
    const started = await api('/api/device/controls/tour/start?tour=' + encodeURIComponent(line.id) + deviceQuery('&'), { method: 'POST' })
    state.tour = line.id
    live.pad.tour = true
    const stops = line.points.length === 1 ? '1 stop' : line.points.length + ' stops'
    setControlStatus('Tour ' + started.tour + ' running over ' + stops + '; the camera acks before it moves, so watch the picture.')
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus('The camera refused the tour: ' + error.status + ' ' + error.message)
  }
}

let controlBusy = false

export async function sendControl(action: string) {
  if (controlBusy || !selectedDevice()) return
  controlBusy = true
  try {
    if (action === 'home') {
      const stopped = await api('/api/device/controls/ptz/stop', { method: 'POST' })
      setControlStatus(stopped.detail || 'Stopped.')
    } else if (action === 'cruise') {
      await toggleTour()
    } else if (MOVES[action]) {
      const [pan, tilt] = MOVES[action]
      const moved = await api('/api/device/controls/ptz?lens=' + encodeURIComponent(movingLensName()) + '&pan=' + pan + '&tilt=' + tilt + '&hold_ms=600' + deviceQuery('&'), { method: 'POST' })
      setControlStatus('Moved the ' + moved.lens + ' lens over ' + moved.backend + ' for ' + moved.hold_ms + ' ms.')
    } else {
      setControlStatus('No server call exists for "' + action + '" in this build.')
    }
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus('The camera refused "' + action + '": ' + error.status + ' ' + error.message)
  } finally {
    controlBusy = false
  }
}

function openPad() {
  if (!live.gate.move.ok || state.view !== 'live') return
  state.padOpen = true
  live.padOpen = true
  setControlStatus('')
  loadPresets()
  renderCaption()
}

export function closePad() {
  if (!state.padOpen) return
  releaseControl()
  state.padOpen = false
  live.padOpen = false
  live.pad.editing = false
  renderCaption()
}

export function togglePad() {
  if (state.padOpen) closePad()
  else openPad()
}
