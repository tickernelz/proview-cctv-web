import { state } from '../app/state'
import { renderCaption } from '../app/caption'
import { live } from './store'
import { decodeAlaw, PlaybackSound } from './playback-audio'
import { armSoundResume, audioFrame, sequenceAt, SOUND_SILENT_MS } from '../lib/stream-player'

type Ready = { kind: 'ready'; path?: string; channel: number | null; claim_ms: number }
type Report = {
  kind: 'report'
  frames: number
  bytes: number
  chunks: number
  dropped: number
  frames_per_second: number
  claim_ms: number
  write_ms_mean: number
  write_ms_max: number
}

type Session = {
  socket: WebSocket | null
  context: AudioContext | null
  stream: MediaStream | null
  source: MediaStreamAudioSourceNode | null
  node: AudioWorkletNode | null
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

let attempt = 0

const TALK_BACK_DELAY = 0.15

const back = {
  sound: new PlaybackSound(),
  on: false,
  heard: 0,
  heardAt: 0,
  queuedAt: 0,
  lastAt: 0,
  since: 0,
  timer: null as number | null,
  disarm: null as (() => void) | null,
}

function releaseBackResume() {
  if (back.disarm) back.disarm()
  back.disarm = null
}

function sendSound(session: Session) {
  if (session.socket && session.socket.readyState === WebSocket.OPEN) session.socket.send(JSON.stringify({ type: 'sound', on: back.on }))
}

function backTick(session: Session) {
  const want = !state.muted
  if (want !== back.on) {
    if (want && !back.sound.enable()) return
    if (!want) {
      back.sound.disable()
      back.sound.clear()
    }
    back.on = want
    back.since = performance.now()
    back.lastAt = 0
    sendSound(session)
  }
  back.sound.tick(performance.now() / 1000, back.on)
  if (!back.on) {
    releaseBackResume()
    live.sound = 'off'
    return
  }
  const context = back.sound.context
  if (context && context.state !== 'running') {
    if (!back.disarm) back.disarm = armSoundResume(() => back.sound.context)
    live.sound = 'blocked'
    return
  }
  releaseBackResume()
  const now = performance.now()
  if (back.queuedAt > back.since && now - back.queuedAt <= SOUND_SILENT_MS) live.sound = 'playing'
  else if (back.heardAt > back.since && now - back.heardAt <= SOUND_SILENT_MS) live.sound = 'waiting'
  else live.sound = now - back.since > SOUND_SILENT_MS ? 'silent' : 'waiting'
}

function hearBack(data: string) {
  const frame = audioFrame(data)
  if (!frame) return false
  back.heard += 1
  back.heardAt = performance.now()
  if (back.on && frame.codec === 'alaw') {
    const now = performance.now() / 1000
    const samples = decodeAlaw(frame.data)
    const rate = Number(frame.rate) || TALK_RATE
    const at = sequenceAt(now + TALK_BACK_DELAY, back.lastAt)
    back.lastAt = at + samples.length / rate
    back.sound.enqueue(at, samples, rate)
    back.queuedAt = performance.now()
    back.sound.tick(now, true)
  }
  return true
}

function startBack(session: Session) {
  stopBack()
  back.heard = 0
  back.heardAt = 0
  back.queuedAt = 0
  back.lastAt = 0
  back.since = performance.now()
  back.timer = window.setInterval(() => backTick(session), 100)
  backTick(session)
}

function stopBack() {
  if (!back.timer) return
  window.clearInterval(back.timer)
  back.timer = null
  releaseBackResume()
  back.sound.disable()
  back.sound.clear()
  if (back.sound.context) void back.sound.context.close()
  back.sound = new PlaybackSound()
  back.on = false
  live.sound = 'off'
}

const uplink = {
  chunks: 0,
  frames: 0,
  bytes: 0,
  ready: null as Ready | null,
  startedAt: 0,
  report: null as Report | null,
  opened: [] as MediaStream[],
  live: null as Session | null,
}

class StalePress extends Error {}

function liveMicrophoneTracks() {
  return uplink.opened.reduce(
    (total, stream) => total + stream.getTracks().filter((track) => track.readyState === 'live').length,
    0,
  )
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

function pushChunk(session: Session, samples: Float32Array) {
  if (!session.socket || session.socket.readyState !== WebSocket.OPEN) return
  const pcm = toPcm16(samples)
  session.socket.send(pcm)
  uplink.chunks += 1
  uplink.bytes += pcm.byteLength
  paintLevel(samples)
}

async function startWorkletTap(session: Session, mine: number) {
  const context = session.context
  const stream = session.stream
  if (!context || !stream) return
  const url = URL.createObjectURL(new Blob([TALK_WORKLET], { type: 'application/javascript' }))
  try {
    await context.audioWorklet.addModule(url)
  } finally {
    URL.revokeObjectURL(url)
  }
  const source = context.createMediaStreamSource(stream)
  const node = new AudioWorkletNode(context, 'talk-tap', {
    numberOfInputs: 1,
    numberOfOutputs: 0,
    channelCount: 1,
  })
  node.port.onmessage = (event: MessageEvent<Float32Array>) => {
    if (mine !== attempt) return
    pushChunk(session, event.data)
  }
  source.connect(node)
  session.source = source
  session.node = node
}

function reportText(report: Report) {
  return (
    'sending \u00b7 ' +
    report.frames +
    ' frames \u00b7 ' +
    report.bytes +
    ' G.711 bytes \u00b7 ' +
    report.frames_per_second.toFixed(1) +
    ' fps \u00b7 write ' +
    report.write_ms_mean.toFixed(2) +
    ' ms mean, ' +
    report.write_ms_max.toFixed(1) +
    ' ms max'
  )
}

function pathLabel(path?: string) {
  if (path === 'relay') return 'the cloud relay'
  if (path === 'lan') return 'the direct address'
  return path || 'the device'
}

function failTalk(message: string, scope?: string) {
  stopTalk()
  live.talk.phase = 'error'
  live.talk.note = 'talk failed: ' + message
  if (scope === 'path') {
    live.gate.intercom = { ok: false, why: 'two-way audio is unavailable: ' + message }
  }
  renderCaption()
}

function closeSession(session: Session) {
  if (session.socket && session.socket.readyState === WebSocket.OPEN) {
    session.socket.send(JSON.stringify({ type: 'stop' }))
  }
  if (session.node) {
    session.node.port.onmessage = null
    session.node.disconnect()
  }
  if (session.source) session.source.disconnect()
  if (session.stream) session.stream.getTracks().forEach((track) => track.stop())
  if (session.context) session.context.close().catch(() => {})
  if (session.socket && session.socket.readyState <= WebSocket.OPEN) session.socket.close()
  session.node = null
  session.source = null
  session.socket = null
  session.stream = null
  session.context = null
}

function openSocket(session: Session, mine: number) {
  return new Promise<Ready>((resolve, reject) => {
    const socket = new WebSocket(talkSocketUrl())
    socket.binaryType = 'arraybuffer'
    session.socket = socket
    let settled = false
    const stale = () => mine !== attempt
    socket.addEventListener('message', (event) => {
      if (typeof event.data !== 'string') return
      if (stale()) return
      if (hearBack(event.data)) return
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
        live.talk.phase = 'sending'
        live.talk.note = reportText(payload)
        return
      }
      if (payload.kind === 'error') {
        if (!settled) {
          settled = true
          reject(Object.assign(new Error(payload.message), { scope: payload.scope }))
        } else {
          failTalk(payload.message, payload.scope)
        }
      }
    })
    socket.addEventListener('error', () => {
      if (!settled) {
        settled = true
        reject(stale() ? new StalePress() : new Error('the talk socket failed to open'))
      }
    })
    socket.addEventListener('close', () => {
      if (!settled) {
        settled = true
        reject(
          stale()
            ? new StalePress()
            : new Error('the talk socket closed before the camera answered'),
        )
        return
      }
      if (!stale() && uplink.live === session && live.talk.on) stopTalk()
    })
  })
}

export async function startTalk() {
  if (live.talk.on || !live.gate.intercom.ok || state.view !== 'live') return
  const mine = ++attempt
  const session: Session = {
    socket: null,
    context: null,
    stream: null,
    source: null,
    node: null,
  }
  uplink.live = session
  live.talk.on = true
  live.talk.phase = 'asking'
  live.talk.note = 'asking for the microphone'
  renderCaption()
  uplink.chunks = 0
  uplink.frames = 0
  uplink.bytes = 0
  uplink.ready = null
  uplink.report = null
  uplink.startedAt = performance.now()
  try {
    const Context = window.AudioContext || (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext
    const context = new Context({ sampleRate: TALK_RATE })
    session.context = context
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
    })
    if (mine !== attempt) {
      stream.getTracks().forEach((track) => track.stop())
      context.close().catch(() => {})
      return
    }
    session.stream = stream
    uplink.opened.push(stream)
    if (uplink.opened.length > 4) uplink.opened.shift()
    if (!context.audioWorklet || typeof AudioWorkletNode !== 'function') {
      throw new Error('this browser has no AudioWorklet, so the microphone cannot be sampled at 8 kHz')
    }
    live.talk.phase = 'opening'
    live.talk.note = 'microphone open, opening the talk channel'
    const ready = await openSocket(session, mine)
    if (mine !== attempt) {
      closeSession(session)
      return
    }
    await startWorkletTap(session, mine)
    if (mine !== attempt) {
      closeSession(session)
      return
    }
    startBack(session)
    live.talk.phase = 'sending'
    live.talk.note =
      'sending over ' +
      pathLabel(ready.path) +
      ' \u00b7 channel ' +
      (ready.channel === null ? 'device' : ready.channel) +
      ' \u00b7 claim ' +
      ready.claim_ms +
      ' ms'
  } catch (error) {
    if (mine !== attempt || error instanceof StalePress) {
      closeSession(session)
      return
    }
    const typed = error as Error & { scope?: string }
    failTalk(typed.message, typed.scope)
  }
}

export function stopTalk() {
  attempt += 1
  stopBack()
  const session = uplink.live
  const wasActive = live.talk.on || session !== null
  if (session) closeSession(session)
  uplink.live = null
  const seconds = uplink.startedAt ? (performance.now() - uplink.startedAt) / 1000 : 0
  if (wasActive && uplink.chunks > 0 && seconds > 0) {
    live.talk.note =
      'talk closed \u00b7 ' +
      pathLabel(uplink.ready?.path) +
      ' \u00b7 ' +
      uplink.chunks +
      ' chunks \u00b7 ' +
      uplink.bytes +
      ' PCM16 bytes \u00b7 ' +
      seconds.toFixed(1) +
      ' s \u00b7 ' +
      (uplink.chunks / seconds).toFixed(1) +
      ' chunks/s'
  } else if (live.talk.phase !== 'error') {
    live.talk.note = ''
  }
  if (live.talk.phase !== 'error') live.talk.phase = 'idle'
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
      phase: live.talk.phase,
      note: live.talk.note,
      mic: liveMicrophoneTracks(),
      open: uplink.live?.socket !== null && uplink.live?.socket?.readyState === WebSocket.OPEN,
      chunks: uplink.chunks,
      bytes: uplink.bytes,
      frames: uplink.frames,
      heard: back.heard,
      played: back.sound.played,
      ready: uplink.ready,
      report: uplink.report,
    }),
  },
})
