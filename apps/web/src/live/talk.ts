import { state } from '../app/state'
import { renderCaption } from '../app/caption'
import { setControlStatus } from '../app/controls'
import { live } from './store'

type Ready = { kind: 'ready'; path?: string; channel: number | null; claim_ms: number }
type Report = {
  kind: 'report'
  frames: number
  frames_per_second: number
  claim_ms: number
  write_ms_mean: number
}

const TALK_RATE = 8000
const TALK_FRAME = 320
const TALK_WORKLET = `
class TalkTap extends AudioWorkletProcessor {
  constructor() {
    super();
    this.buffer = new Float32Array(${TALK_FRAME});
    this.filled = 0;
  }
  process(inputs) {
    const channel = inputs[0] && inputs[0][0];
    if (!channel) {
      return true;
    }
    for (let index = 0; index < channel.length; index += 1) {
      this.buffer[this.filled] = channel[index];
      this.filled += 1;
      if (this.filled === this.buffer.length) {
        this.port.postMessage(this.buffer.slice(0));
        this.filled = 0;
      }
    }
    return true;
  }
}
registerProcessor('talk-tap', TalkTap);
`

const uplink = {
  socket: null as WebSocket | null,
  context: null as AudioContext | null,
  stream: null as MediaStream | null,
  source: null as MediaStreamAudioSourceNode | null,
  node: null as AudioWorkletNode | null,
  moduleUrl: null as string | null,
  chunks: 0,
  frames: 0,
  bytes: 0,
  ready: null as Ready | null,
  startedAt: 0,
  report: null as Report | null,
}

function talkSocketUrl() {
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:'
  const device = state.selectedId || ''
  const query = device ? '?device=' + encodeURIComponent(device) : ''
  return scheme + '//' + location.host + '/ws/talk' + query
}

function toPcm16(samples: Float32Array) {
  const view = new DataView(new ArrayBuffer(samples.length * 2))
  for (let index = 0; index < samples.length; index += 1) {
    const value = Math.max(-1, Math.min(1, samples[index]))
    view.setInt16(index * 2, value < 0 ? value * 0x8000 : value * 0x7fff, true)
  }
  return view.buffer
}

function paintLevel(samples: Float32Array) {
  let sum = 0
  for (const sample of samples) sum += sample * sample
  const rms = Math.sqrt(sum / samples.length)
  live.talk.level = Math.min(5, Math.round(rms * 20))
}

function pushChunk(samples: Float32Array) {
  if (!uplink.socket || uplink.socket.readyState !== WebSocket.OPEN) return
  const pcm = toPcm16(samples)
  uplink.socket.send(pcm)
  uplink.chunks += 1
  uplink.bytes += pcm.byteLength
  paintLevel(samples)
}

async function startWorkletTap(context: AudioContext, stream: MediaStream) {
  uplink.moduleUrl = URL.createObjectURL(new Blob([TALK_WORKLET], { type: 'application/javascript' }))
  await context.audioWorklet.addModule(uplink.moduleUrl)
  const source = context.createMediaStreamSource(stream)
  const node = new AudioWorkletNode(context, 'talk-tap', {
    numberOfInputs: 1,
    numberOfOutputs: 0,
    channelCount: 1,
  })
  node.port.onmessage = (event: MessageEvent<Float32Array>) => pushChunk(event.data)
  source.connect(node)
  uplink.source = source
  uplink.node = node
}

function reportText(report: Report) {
  return (
    report.frames +
    ' frames \u00b7 ' +
    report.frames_per_second.toFixed(1) +
    ' fps \u00b7 claim ' +
    report.claim_ms +
    ' ms \u00b7 write ' +
    report.write_ms_mean.toFixed(2) +
    ' ms mean'
  )
}

function openSocket() {
  return new Promise<Ready>((resolve, reject) => {
    const socket = new WebSocket(talkSocketUrl())
    socket.binaryType = 'arraybuffer'
    uplink.socket = socket
    let settled = false
    socket.addEventListener('message', (event) => {
      if (typeof event.data !== 'string') return
      let payload = null
      try {
        payload = JSON.parse(event.data)
      } catch {
        return
      }
      if (payload.kind === 'ready') {
        uplink.ready = payload
        if (!settled) {
          settled = true
          resolve(payload)
        }
        return
      }
      if (payload.kind === 'report') {
        uplink.report = payload
        uplink.frames = payload.frames
        setControlStatus(reportText(payload))
        return
      }
      if (payload.kind === 'error') {
        if (!settled) {
          settled = true
          reject(new Error(payload.message))
        } else {
          setControlStatus(payload.message)
          stopTalk()
        }
      }
    })
    socket.addEventListener('error', () => {
      if (!settled) {
        settled = true
        reject(new Error('the talk socket failed to open'))
      }
    })
    socket.addEventListener('close', () => {
      if (!settled) {
        settled = true
        reject(new Error('the talk socket closed before the camera answered'))
      }
      if (live.talk.on) stopTalk()
    })
  })
}

export async function startTalk() {
  if (live.talk.on || !live.gate.intercom.ok || state.view !== 'live') return
  live.talk.on = true
  renderCaption()
  uplink.chunks = 0
  uplink.frames = 0
  uplink.bytes = 0
  uplink.report = null
  uplink.startedAt = performance.now()
  try {
    const ready = await openSocket()
    const Context = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
    const context = new Context({ sampleRate: TALK_RATE })
    uplink.context = context
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
    })
    if (!live.talk.on) {
      stream.getTracks().forEach((track) => track.stop())
      return
    }
    uplink.stream = stream
    if (!context.audioWorklet || typeof AudioWorkletNode !== 'function') {
      throw new Error('this browser has no AudioWorklet, so the microphone cannot be sampled at 8 kHz')
    }
    await startWorkletTap(context, stream)
    setControlStatus(
      'talk open over ' +
        (ready.path || 'the device') +
        ' \u00b7 channel ' +
        (ready.channel === null ? 'device' : ready.channel) +
        ' \u00b7 claim ' +
        ready.claim_ms +
        ' ms',
    )
  } catch (error) {
    stopTalk()
    setControlStatus('The talk channel did not open: ' + (error as Error).message)
  }
}

export function stopTalk() {
  const wasActive = live.talk.on || uplink.socket !== null
  if (uplink.socket && uplink.socket.readyState === WebSocket.OPEN) {
    uplink.socket.send(JSON.stringify({ type: 'stop' }))
  }
  if (uplink.node) {
    uplink.node.disconnect()
    uplink.node.port.onmessage = null
  }
  if (uplink.source) uplink.source.disconnect()
  if (uplink.moduleUrl) {
    URL.revokeObjectURL(uplink.moduleUrl)
    uplink.moduleUrl = null
  }
  if (uplink.stream) uplink.stream.getTracks().forEach((track) => track.stop())
  if (uplink.context) uplink.context.close().catch(() => {})
  if (uplink.socket && uplink.socket.readyState <= WebSocket.OPEN) uplink.socket.close()
  const seconds = uplink.startedAt ? (performance.now() - uplink.startedAt) / 1000 : 0
  if (wasActive && uplink.chunks > 0 && seconds > 0) {
    setControlStatus(
      'talk closed \u00b7 ' +
        uplink.chunks +
        ' chunks \u00b7 ' +
        uplink.bytes +
        ' PCM bytes \u00b7 ' +
        seconds.toFixed(1) +
        ' s \u00b7 ' +
        (uplink.chunks / seconds).toFixed(1) +
        ' chunks/s',
    )
  }
  uplink.socket = null
  uplink.context = null
  uplink.stream = null
  uplink.source = null
  uplink.node = null
  live.talk.on = false
  live.talk.level = 0
  renderCaption()
}

Object.assign(window, {
  proviewTalk: {
    start: startTalk,
    stop: stopTalk,
    state: () => ({
      active: live.talk.on,
      open: uplink.socket !== null && uplink.socket.readyState === WebSocket.OPEN,
      chunks: uplink.chunks,
      bytes: uplink.bytes,
      frames: uplink.frames,
      ready: uplink.ready,
      report: uplink.report,
    }),
  },
})
