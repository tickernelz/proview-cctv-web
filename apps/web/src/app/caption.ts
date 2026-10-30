import { live } from '../live/store'
import { lensEntries, lensViewLabel } from './lens'
import { currentCamera } from './devices'
import { hms } from './format'
import { state } from './state'

export function elapsed(since: number) {
  const seconds = Math.max(0, Math.floor((Date.now() - since) / 1000))
  return String(Math.floor(seconds / 60)).padStart(2, '0') + ':' + String(seconds % 60).padStart(2, '0')
}

function liveLag() {
  const video = document.getElementById('video') as HTMLVideoElement | null
  if (!video || !video.buffered || video.buffered.length === 0) return null
  const lag = video.buffered.end(video.buffered.length - 1) - video.currentTime
  return Number.isFinite(lag) && lag >= 0 ? lag : null
}

function captionText() {
  const name = state.stage.name
  if (state.networkLostAt && state.stage.lastFrameAt) return 'not updating — last frame ' + hms(state.stage.lastFrameAt)
  if (live.talk.on) return 'live — the microphone is open'
  if (name === 'loading') return 'connecting — nothing has been received yet'
  if (name === 'stalled') return 'frozen — the frame below is from ' + hms(state.stage.lastFrameAt || Date.now())
  if (name === 'error') {
    const after = Math.max(0, Math.round((state.stage.failedAt - state.stage.requestedAt) / 1000))
    return 'failed at ' + hms(state.stage.failedAt) + ' — ' + after + ' s after the request'
  }
  if (name === 'offline') return 'offline'
  if (name === 'playback') return 'recording' + (live.playback.at === null ? '' : ' from ' + hms(live.playback.at))
  if (name === 'playing') {
    const lens = lensEntries().length > 1 ? ', ' + lensViewLabel().toLowerCase() : ''
    return 'live' + lens + (state.padOpen ? ' — arrow keys move it' : '')
  }
  return ''
}

export function renderFlag() {
  const name = state.stage.name
  let text = ''
  let tone = ''
  if (state.recorder) {
    text = 'Recording ' + elapsed(state.recordStart)
    tone = 'rec'
  } else if (live.talk.on) {
    text = 'release to stop'
  } else if (state.networkLostAt) {
    text = 'stale'
    tone = 'warn'
  } else if (name === 'stalled') {
    text = 'frozen ' + Math.max(0, Math.round((Date.now() - (state.stage.lastFrameAt || Date.now())) / 1000)) + ' s'
    tone = 'warn'
  } else if (name === 'offline') {
    text = 'offline'
    tone = 'warn'
  } else if (name === 'loading' || name === 'error') {
    text = 'not live'
  } else if (name === 'playing') {
    const lag = liveLag()
    text = lag === null ? '' : lag.toFixed(1) + ' s behind'
  }
  live.flag.text = text
  live.flag.tone = tone
}

export function renderCaption() {
  const camera = currentCamera()
  live.camera = camera ? camera.name : ''
  live.state = captionText()
  renderFlag()
}
