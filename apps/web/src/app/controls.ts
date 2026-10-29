import { api } from '../lib/api'
import { live, type Gate } from '../live/store'
import { renderCaption } from './caption'
import { selectedDevice } from './devices'
import { lensEntries } from './lens'
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
}

const CONTROL_ABSENT: Record<string, string> = {
  tracking: 'No auto-tracking call exists on this server, and the capability report lists none for this device.',
  alarm: 'No audio-alarm call exists on this server; the device reports white light and two-way audio, not an alarm.',
  'cloud-storage': 'No server call for cloud storage in this build: the cloud API exposes the device list only.',
  'sd-album': 'No server call for the SD card album in this build: the cloud API exposes the device list only.',
}

const MOVES: Record<string, [number, number]> = { up: [0, -0.5], down: [0, 0.5], left: [-0.5, 0], right: [0.5, 0] }

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
  const blocked = state.control.talkBlocked
  if (blocked && blocked.device === state.selectedId) {
    live.gate.intercom = { ok: false, why: 'two-way audio is unavailable: ' + blocked.why }
  }
  if (state.padOpen && !live.gate.move.ok) closePad()
  renderCapabilities()
}

export async function probeControlChannel(device: Device) {
  if (state.control.checkedFor === device.id) {
    renderControlAvailability()
    return
  }
  try {
    const payload = await api('/api/device/controls?device=' + encodeURIComponent(device.id))
    state.control = {
      ...state.control,
      rows: Array.isArray(payload.capabilities) ? payload.capabilities : [],
      onvif: payload.onvif === true,
      dvrip: payload.dvrip === true,
      error: null,
      checkedFor: device.id,
    }
  } catch (error: any) {
    if (sessionLost(error)) return
    state.control = { ...state.control, rows: [], onvif: false, dvrip: false, error: error.status + ' ' + error.message, checkedFor: null }
  }
  renderControlAvailability()
}

export function setControlStatus(message: string) {
  live.pad.status = message || ''
}

async function loadPresets() {
  live.pad.presets = []
  live.pad.presetsShown = false
  const row = capabilityRow('ptz preset', movingLensName())
  if (!row || row.state !== 'supported') return
  try {
    const presets = await api('/api/device/controls/presets')
    live.pad.presets = presets.map((preset: { token: string; name?: string }) => ({ token: preset.token, name: preset.name || preset.token }))
    live.pad.presetsShown = presets.length > 0
  } catch (error: any) {
    if (!sessionLost(error)) setControlStatus('Presets could not be read: ' + error.status + ' ' + error.message)
  }
}

export async function gotoPreset(token: string, name: string) {
  try {
    await api('/api/device/controls/preset/goto?token=' + encodeURIComponent(token), { method: 'POST' })
    setControlStatus('At ' + name + '.')
  } catch (error: any) {
    setControlStatus('The camera refused ' + name + ': ' + error.status + ' ' + error.message)
  }
}

export async function toggleWhiteLight() {
  try {
    const current = await api('/api/device/controls/light')
    const next = current.mode === 'always' ? 'off' : 'always'
    const applied = await api('/api/device/controls/light?mode=' + encodeURIComponent(next), { method: 'POST' })
    live.settings.lightStatus = 'White light: ' + (applied.mode || next) + ' (it was ' + (current.mode || 'not reported') + ').'
  } catch (error: any) {
    if (!sessionLost(error)) live.settings.lightStatus = 'The camera refused the white light: ' + error.status + ' ' + error.message
  }
}

async function toggleTour() {
  try {
    if (state.tour !== null) {
      const stopped = await api('/api/device/controls/tour/stop?tour=' + encodeURIComponent(state.tour), { method: 'POST' })
      state.tour = null
      live.pad.tour = false
      setControlStatus('Stopped tour ' + stopped.tour + '.')
      return
    }
    const tour = await api('/api/device/controls/tour')
    const line = (tour.lines || []).find((entry: { points?: unknown[] }) => (entry.points || []).length > 0)
    if (!line) {
      setControlStatus('No tour line with points on this camera: ' + (tour.note || 'add presets to a tour first'))
      return
    }
    const started = await api('/api/device/controls/tour/start?tour=' + encodeURIComponent(line.id), { method: 'POST' })
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
      const moved = await api('/api/device/controls/ptz?lens=' + encodeURIComponent(movingLensName()) + '&pan=' + pan + '&tilt=' + tilt + '&hold_ms=600', { method: 'POST' })
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
  state.padOpen = false
  live.padOpen = false
  renderCaption()
}

export function togglePad() {
  if (state.padOpen) closePad()
  else openPad()
}
