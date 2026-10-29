import { setControlStatus } from './controls'
import { currentCrop } from './lens'
import { live } from '../live/store'
import { elapsed, renderFlag } from './caption'
import { selectedDevice } from './devices'
import { state } from './state'

const video = () => document.getElementById('video') as HTMLVideoElement

export function renderMute() {
  video().muted = state.muted
  live.muted = state.muted
}

export function toggleMute() {
  state.muted = !state.muted
  renderMute()
}

function download(blob: Blob, name: string) {
  const link = document.createElement('a')
  link.href = URL.createObjectURL(blob)
  link.download = name
  link.click()
  window.setTimeout(() => URL.revokeObjectURL(link.href), 4000)
}

function stamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

export function captureSnapshot() {
  const frame = video()
  const device = selectedDevice()
  if (!device || state.stage.name !== 'playing' || !frame.videoWidth) return
  const crop = currentCrop()
  const sx = Math.round(crop.x * frame.videoWidth)
  const sy = Math.round(crop.y * frame.videoHeight)
  const sw = Math.round(crop.w * frame.videoWidth)
  const sh = Math.round(crop.h * frame.videoHeight)
  const canvas = document.createElement('canvas')
  canvas.width = sw
  canvas.height = sh
  const context = canvas.getContext('2d')
  if (!context) return
  if (state.prefs.flip) {
    context.translate(sw, sh)
    context.rotate(Math.PI)
  }
  context.drawImage(frame, sx, sy, sw, sh, 0, 0, sw, sh)
  canvas.toBlob((blob) => {
    if (blob) download(blob, device.id + '_ch' + state.channel + '_lens-' + state.lensView + '_' + stamp() + '.png')
  }, 'image/png')
}

export function stopRecording() {
  if (state.recorder) state.recorder.stop()
}

function renderRecord() {
  live.record.on = Boolean(state.recorder)
  live.record.elapsed = live.record.on ? elapsed(state.recordStart) : ''
  renderFlag()
}

export function toggleRecord() {
  const frame = video() as HTMLVideoElement & { captureStream?: () => MediaStream }
  if (state.recorder) {
    state.recorder.stop()
    return
  }
  if (state.stage.name !== 'playing' || typeof frame.captureStream !== 'function') {
    setControlStatus('Recording needs a playing picture and a browser with captureStream.')
    return
  }
  const stream = frame.captureStream()
  if (!stream || stream.getVideoTracks().length === 0) return
  const candidates = ['video/webm;codecs=vp9', 'video/webm;codecs=vp8', 'video/webm']
  const mimeType = candidates.find((type) => window.MediaRecorder && MediaRecorder.isTypeSupported(type))
  if (!mimeType) return
  state.recordChunks = []
  const recorder = new MediaRecorder(stream, { mimeType })
  state.recorder = recorder
  state.recordStart = Date.now()
  recorder.addEventListener('dataavailable', (event) => {
    if (event.data && event.data.size) state.recordChunks.push(event.data)
  })
  recorder.addEventListener('stop', () => {
    const device = selectedDevice()
    const blob = new Blob(state.recordChunks, { type: mimeType })
    state.recordChunks = []
    state.recorder = null
    renderRecord()
    if (blob.size) download(blob, (device ? device.id : 'camera') + '_ch' + state.channel + '_' + stamp() + '.webm')
    for (const track of stream.getTracks()) track.stop()
  })
  recorder.start()
  renderRecord()
}
