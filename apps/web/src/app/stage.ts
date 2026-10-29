import { api } from '../lib/api'
import { StreamPlayer } from '../lib/stream-player'
import { setControlStatus } from './controls'
import { layoutLensCrop, lensEntries, lensViewLabel } from './lens'
import { live } from '../live/store'
import { elapsed, renderCaption, renderFlag } from './caption'
import { renderBanner } from './chrome'
import { currentCamera, deviceStatus } from './devices'
import { hms } from './format'
import { stopRecording } from './media'
import { modeName, sessionLost, sourceLabel } from './shell'
import { state } from './state'

type Attempt = { stage?: string; elapsed_ms?: number; detail?: string; message?: string; stages?: { stage: string; ok: boolean; elapsed_ms: number; detail: string }[] }
type StageDetail = { text?: string; code?: string; stage?: Attempt | null }

const showingStage = (name: string) => name === 'playing' || name === 'stalled'

export function setStage(name: string, detail: StageDetail = {}) {
  const camera = currentCamera()
  state.stage.name = name
  if (name === 'loading') {
    state.stage.requestedAt = Date.now()
    state.stage.error = null
  }
  if (name === 'error') {
    state.stage.failedAt = Date.now()
    state.stage.error = detail
  }
  if (name === 'playing' && !performance.getEntriesByName('first-frame').length) performance.mark('first-frame')
  const showing = showingStage(name)
  const quality = state.prefs.quality === 'sub' ? 'SD' : 'HD'
  let title = ''
  let text = ''
  if (name === 'loading' && camera) {
    title = 'Requesting the stream'
    text = camera.name + ', channel ' + camera.channel + ', ' + quality + '. The server is asking the camera for a ticket.'
  } else if (name === 'error') {
    title = 'No picture from ' + (camera ? camera.name : 'this camera')
    text = detail.text || 'The stream stopped.'
  } else if (name === 'offline') {
    title = 'Camera offline'
    text = camera ? 'The picture comes back once ' + camera.name + ' answers and the list is refreshed.' : 'The cloud reports it offline.'
  }
  const stage = live.stage
  stage.name = name
  stage.title = title
  stage.text = text
  stage.code = name === 'error' && detail.code ? detail.code : ''
  renderStage(name === 'error' ? detail.stage : null)
  stage.passwordHint = name === 'error' && /401|unauthori|password|auth|login/i.test((detail.code || '') + ' ' + (detail.text || ''))
  live.record.disabled = name !== 'playing'
  live.snapshotDisabled = !showing
  if (!showing) stopRecording()
  renderCaption()
  renderFsLine()
  layoutLensCrop()
  renderBanner()
}

export function renderFsLine() {
  const camera = currentCamera()
  const showing = showingStage(state.stage.name)
  const fs = live.stage.fs
  fs.shown = Boolean(document.fullscreenElement && document.fullscreenElement === document.getElementById('well') && state.prefs.osd && showing && camera)
  fs.name = camera ? camera.name + (lensEntries().length > 1 ? ' · ' + lensViewLabel().toLowerCase() : '') : ''
  fs.time = (state.stage.name === 'playing' ? 'live' : state.stage.name) + ' · ' + hms(Date.now())
}

export function tick() {
  live.clock = hms(Date.now())
  if (live.stage.fs.shown) renderFsLine()
  if (state.recorder) live.record.elapsed = elapsed(state.recordStart)
  renderFlag()
}

window.setInterval(tick, 1000)

const cloudPathWalks = new Set<string>()

async function cloudPathDetail(deviceId: string) {
  if (cloudPathWalks.has(deviceId)) return ''
  cloudPathWalks.add(deviceId)
  try {
    const path = await api('/api/cloud-path?device=' + encodeURIComponent(deviceId))
    const parts: string[] = []
    if (path.access) parts.push('access server ' + path.access)
    if (path.agent) parts.push('agent ' + path.agent)
    if (path.relayAccepted) parts.push('relay accepted the session for ' + (path.destIp || 'the device'))
    else if (path.relayError) parts.push('relay ' + path.relayError)
    if (!parts.length && path.detail) parts.push(path.detail)
    return parts.join(', ')
  } catch {
    return ''
  } finally {
    cloudPathWalks.delete(deviceId)
  }
}

function stageLine(attempt: Attempt) {
  return 'Failed at ' + attempt.stage + ' after ' + attempt.elapsed_ms + ' ms: ' + attempt.detail
}

function stageTrailInline(attempt: Attempt) {
  return 'Stages: ' + (attempt.stages || []).map((entry) => entry.stage + ' ' + (entry.ok ? 'ok' : 'failed') + ' ' + entry.elapsed_ms + ' ms').join(' · ')
}

function stageTrail(attempt: Attempt) {
  return (attempt.stages || []).map((entry) => entry.stage + ' ' + (entry.ok ? 'ok' : 'failed') + ' in ' + entry.elapsed_ms + ' ms: ' + entry.detail).join('\n')
}

function renderStage(attempt: Attempt | null | undefined) {
  state.stage.attempt = attempt && attempt.stages && attempt.stages.length ? attempt : null
  live.stage.trail = state.stage.attempt ? stageTrailInline(state.stage.attempt) : ''
  live.stage.trailShown = Boolean(attempt)
  live.stage.canCopy = Boolean(state.stage.attempt)
  live.stage.copied = false
}

export async function copyStageDetail() {
  const attempt = state.stage.attempt as Attempt | null
  if (!attempt) return
  const head = attempt.stage ? stageLine(attempt) : 'No stage failed; the provider stopped: ' + (attempt.message || 'no message')
  try {
    await navigator.clipboard.writeText(head + '\n' + stageTrail(attempt))
    live.stage.copied = true
    window.setTimeout(() => {
      live.stage.copied = false
    }, 2000)
  } catch (error) {
    setControlStatus('The browser refused the clipboard: ' + (error as Error).message)
  }
}

function explainStreamError(error: any, attempt: Attempt | null) {
  if (error.code === 'misconfigured') return error.message
  if (attempt && attempt.stage) return stageLine(attempt)
  if (error.status === 501) return 'Live video is not wired yet: the device transport is still being built, so there is no frame to show.'
  if (error.status === 404) return 'The server has no playable stream for that channel: ' + error.message
  if (error.status === 401) return 'Session expired. Sign in again.'
  if (error.status === 502) return 'The camera or the recording did not answer, so the server has no playable stream for this channel. Its reason is below.'
  if (error.status === 400) return 'The server rejected the request: ' + error.message
  return 'The stream could not be started: the server answered ' + error.status + '. Its reason is below.'
}

function stagePlayer() {
  let player = state.players.get('stage')
  if (!player) {
    player = new StreamPlayer(
      document.getElementById('video') as HTMLVideoElement,
      (name: string, message: string) => {
        if (name === 'stalled') {
          if (state.stage.name !== 'stalled') setStage('stalled')
          return
        }
        if (name === 'error') {
          setStage('error', { text: message, code: '' })
          return
        }
        setStage(name)
        renderCameraInfo()
      },
      () => state.muted,
    )
    state.players.set('stage', player)
  }
  return player
}

export function stopStagePlayer() {
  const player = state.players.get('stage')
  if (player) {
    player.destroy()
    state.players.delete('stage')
  }
  stopRecording()
}

export async function requestStream() {
  const camera = currentCamera()
  if (!camera || (deviceStatus(camera.device).known && !camera.device.online) || !state.signedIn) return
  const previous = state.players.get('stage')
  if (previous) {
    previous.destroy()
    state.players.delete('stage')
  }
  const player = stagePlayer()
  state.ticket = null
  setStage('loading')
  try {
    const ticket = await api(
      '/api/stream?device=' + encodeURIComponent(camera.device.id) + '&channel=' + encodeURIComponent(camera.channel) + '&quality=' + encodeURIComponent(state.prefs.quality),
    )
    if (currentCamera() !== camera || state.players.get('stage') !== player) return
    state.ticket = ticket
    renderCameraInfo()
    await player.play(ticket)
  } catch (error: any) {
    if (sessionLost(error)) return
    const attempt = error.body && Array.isArray(error.body.stages) && error.body.stages.length ? error.body : null
    const base = explainStreamError(error, attempt)
    const misconfigured = error.code === 'misconfigured'
    setStage('error', { text: base, code: misconfigured ? '' : error.message, stage: attempt })
    console.error('stream request failed', error)
    if (error.status === 501 && !misconfigured && camera.device) {
      cloudPathDetail(camera.device.id).then((detail) => {
        if (!detail || currentCamera() !== camera || state.stage.name !== 'error') return
        setStage('error', { text: base + ' Cloud reach: ' + detail + '.', code: error.message, stage: attempt })
      })
    }
  }
}

export function renderCameraInfo() {
  const camera = currentCamera()
  if (!camera) {
    live.settings.camera = null
    return
  }
  const device = camera.device
  const ticket = state.ticket
  const mode = state.mode ? modeName(state.mode.mode) : ''
  live.settings.camera = {
    name: camera.name,
    id: device.id,
    channel: String(camera.channel),
    model: device.model || 'model not reported',
    connection: (state.mode ? mode[0].toUpperCase() + mode.slice(1) + ' · answering ' + sourceLabel() : sourceLabel()) + (device.lan_host ? ' · ' + device.lan_host : ''),
    stream: ticket
      ? 'Stream: ' + ticket.kind.toUpperCase() + ' · ' + (state.prefs.quality === 'sub' ? 'SD' : 'HD') + ' · ' + ticket.url + (ticket.expires_in ? ' · ticket expires in ' + ticket.expires_in + ' s' : '')
      : 'No stream ticket yet.',
  }
}

export function shortStreamError(error: any) {
  if (error.code === 'misconfigured') return error.message
  if (error.status === 404) return 'This channel does not exist on the device.'
  if (error.status === 401) return 'Session expired. Sign in again.'
  if (error.status === 502) return 'No playable stream for this channel: the server answered 502.'
  if (error.status === 501) return 'Live video is not wired for this channel yet.'
  return 'The stream could not be started: the server answered ' + error.status + '.'
}
