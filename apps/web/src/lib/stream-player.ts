import { AVC_TYPE, decodes, mimeOf, rememberHevc, shouldAskForHevc, watchFirstFrame, type StreamShape } from './hevc'
import { decodeAlaw, PlaybackSound } from '../live/playback-audio'

export const LIVE_WINDOW_SECONDS = 12
export const LIVE_TARGET_LAG_SECONDS = 1.0

export type StreamTicket = { kind: string; url: string }
export type PlayerState = 'playing' | 'stalled' | 'error'
export type SoundState = 'off' | 'waiting' | 'playing' | 'silent' | 'blocked' | 'unplayable' | 'unsupported'

export const SOUND_SILENT_MS = 4000
export const TRANSCODE_VIDEO_DELAY_SECONDS = 0.2
export const AUDIO_MAX_AHEAD_SECONDS = 1.5
const EDGE_REACH_SECONDS = 4
const PLAYABLE_AUDIO = new Set(['alaw', 'PCMA'])

export function sequenceAt(want: number, last: number) {
  return last > want && last - want < AUDIO_MAX_AHEAD_SECONDS ? last : want
}

export function armSoundResume(context: () => AudioContext | null) {
  const resume = () => {
    const current = context()
    if (current && current.state !== 'running' && current.state !== 'closed') void current.resume()
  }
  window.addEventListener('pointerdown', resume, true)
  window.addEventListener('keydown', resume, true)
  return () => {
    window.removeEventListener('pointerdown', resume, true)
    window.removeEventListener('keydown', resume, true)
  }
}

type AudioFrame = { type: 'audio'; frame: number; codec: string; rate: number; data: string }

export function audioFrame(text: string): AudioFrame | null {
  if (!text.includes('"audio"')) return null
  try {
    const frame = JSON.parse(text)
    return frame && frame.type === 'audio' && typeof frame.data === 'string' ? (frame as AudioFrame) : null
  } catch {
    return null
  }
}

type HlsInstance = { on: (event: string, handler: (event: unknown, data: { fatal?: boolean; type?: string }) => void) => void; loadSource: (url: string) => void; attachMedia: (video: HTMLVideoElement) => void; destroy: () => void }
type HlsConstructor = { new (options: { lowLatencyMode: boolean }): HlsInstance; isSupported: () => boolean; Events: { ERROR: string } }

export class StreamPlayer {
  video: HTMLVideoElement
  onState: (name: PlayerState, message: string) => void
  muted: () => boolean
  socket: WebSocket | null = null
  media: MediaSource | null = null
  buffer: SourceBuffer | null = null
  queue: Uint8Array[] = []
  url: string | null = null
  hls: HlsInstance | null = null
  watchdog: number | null = null
  lastTime = 0
  lastAdvance = 0
  stalled = false
  metadataTimer: number | null = null
  announced = false
  onMetadata: (() => void) | null = null
  transport: 'hevc' | 'avc' | null = null
  stopHevcWatch: (() => void) | null = null
  sound = new PlaybackSound()
  soundOn = false
  soundState: SoundState = 'off'
  soundSince = 0
  soundTimer: number | null = null
  heardAt = 0
  heard = 0
  queuedAt = 0
  lastAt = 0
  helloAudio: string | null = null
  disarmResume: (() => void) | null = null
  announcedAt = 0
  edgeMedia = 0
  edgeWall = 0
  onSound: ((state: SoundState) => void) | null = null

  constructor(video: HTMLVideoElement, onState?: (name: PlayerState, message: string) => void, muted: () => boolean = () => true) {
    this.video = video
    this.onState = onState || (() => {})
    this.muted = muted
  }

  expectFirstFrame() {
    const video = this.video
    this.announced = false
    const announce = () => {
      if (this.announced) {
        return
      }
      this.announced = true
      this.announcedAt = performance.now()
      this.onMetadata = null
      if (this.metadataTimer) {
        window.clearTimeout(this.metadataTimer)
        this.metadataTimer = null
      }
      this.startWatchdog()
      this.onState('playing', '')
    }
    if (video.videoWidth > 0) {
      announce()
      return
    }
    this.onMetadata = announce
    video.addEventListener('loadedmetadata', announce, { once: true })
    this.metadataTimer = window.setTimeout(() => {
      if (!this.announced) {
        this.onState('error', 'The stream connected but sent no picture this browser could decode.')
      }
    }, 6000)
  }

  startWatchdog() {
    this.stopWatchdog()
    this.lastTime = this.video.currentTime
    this.lastAdvance = Date.now()
    this.stalled = false
    this.watchdog = window.setInterval(() => {
      const now = Date.now()
      const time = this.video.currentTime
      if (time !== this.lastTime) {
        this.lastTime = time
        this.lastAdvance = now
        if (this.stalled) {
          this.stalled = false
          this.onState('playing', '')
        }
        return
      }
      if (this.video.paused) {
        return
      }
      const idle = Math.round((now - this.lastAdvance) / 1000)
      if (idle >= 3) {
        this.stalled = true
        this.onState(
          'stalled',
          'The picture has not advanced for ' +
            idle +
            ' s. The last decoded frame stays on screen, and the clock on it is not the time of what you see.',
        )
      }
    }, 1000)
  }

  stopWatchdog() {
    if (this.watchdog) {
      window.clearInterval(this.watchdog)
      this.watchdog = null
    }
  }

  clearHevcWatch() {
    if (this.stopHevcWatch) this.stopHevcWatch()
    this.stopHevcWatch = null
  }

  setSoundState(next: SoundState) {
    if (next === this.soundState) return
    this.soundState = next
    if (this.onSound) this.onSound(next)
  }

  sendSound() {
    const socket = this.socket
    if (socket && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'sound', on: this.soundOn }))
  }

  applySound(on: boolean) {
    if (on === this.soundOn) return
    if (on && !this.sound.enable()) {
      this.setSoundState('unsupported')
      return
    }
    if (!on) {
      this.sound.disable()
      this.sound.clear()
    }
    this.soundOn = on
    this.soundSince = performance.now()
    this.lastAt = 0
    this.sendSound()
  }

  releaseResume() {
    if (this.disarmResume) this.disarmResume()
    this.disarmResume = null
  }

  soundTick() {
    this.applySound(!this.muted())
    this.tickSound()
    if (!this.soundOn) {
      this.releaseResume()
      if (this.soundState !== 'unsupported' || this.muted()) this.setSoundState('off')
      return
    }
    if (this.helloAudio && !PLAYABLE_AUDIO.has(this.helloAudio)) return this.setSoundState('unplayable')
    const context = this.sound.context
    if (context && context.state !== 'running') {
      if (!this.disarmResume) this.disarmResume = armSoundResume(() => this.sound.context)
      return this.setSoundState('blocked')
    }
    this.releaseResume()
    const now = performance.now()
    const from = Math.max(this.soundSince, this.announcedAt)
    if (this.queuedAt > from && now - this.queuedAt <= SOUND_SILENT_MS) return this.setSoundState('playing')
    if (this.heardAt > from && now - this.heardAt <= SOUND_SILENT_MS) return this.setSoundState('waiting')
    this.setSoundState(this.announced && now - from > SOUND_SILENT_MS ? 'silent' : 'waiting')
  }

  startSound() {
    this.stopSound()
    this.soundTimer = window.setInterval(() => this.soundTick(), 100)
    this.soundTick()
  }

  stopSound() {
    if (this.soundTimer) {
      window.clearInterval(this.soundTimer)
      this.soundTimer = null
    }
  }

  noteEdge() {
    const buffer = this.buffer
    if (!buffer || buffer.buffered.length === 0) return
    const end = buffer.buffered.end(buffer.buffered.length - 1)
    if (end === this.edgeMedia) return
    this.edgeMedia = end
    this.edgeWall = performance.now() / 1000
  }

  tickSound() {
    this.sound.tick(this.video.currentTime, !this.video.paused)
  }

  audioTime() {
    const reach = performance.now() / 1000 - this.edgeWall
    if (this.edgeWall === 0 || reach > EDGE_REACH_SECONDS) return null
    return this.edgeMedia + reach + (this.transport === 'hevc' ? 0 : TRANSCODE_VIDEO_DELAY_SECONDS)
  }

  hear(frame: AudioFrame) {
    this.heard += 1
    this.heardAt = performance.now()
    if (!this.soundOn || frame.codec !== 'alaw' || !this.buffer) return
    const want = this.audioTime()
    if (want === null) return
    const samples = decodeAlaw(frame.data)
    const rate = Number(frame.rate) || 8000
    const at = sequenceAt(want, this.lastAt)
    this.lastAt = at + samples.length / rate
    this.sound.enqueue(at, samples, rate)
    this.queuedAt = performance.now()
    this.tickSound()
  }

  destroy() {
    this.clearHevcWatch()
    this.stopSound()
    this.releaseResume()
    this.sound.disable()
    this.sound.clear()
    if (this.sound.context) void this.sound.context.close()
    this.sound = new PlaybackSound()
    this.soundOn = false
    this.heard = 0
    this.heardAt = 0
    this.queuedAt = 0
    this.lastAt = 0
    this.helloAudio = null
    this.edgeMedia = 0
    this.edgeWall = 0
    this.setSoundState('off')
    this.ticket = null
    this.transport = null
    delete this.video.dataset.transport
    this.stopWatchdog()
    if (this.onMetadata) {
      this.video.removeEventListener('loadedmetadata', this.onMetadata)
      this.onMetadata = null
    }
    if (this.metadataTimer) {
      window.clearTimeout(this.metadataTimer)
      this.metadataTimer = null
    }
    if (this.socket) {
      this.socket.onmessage = null
      this.socket.onclose = null
      this.socket.onerror = null
      this.socket.close()
      this.socket = null
    }
    if (this.buffer && this.media && this.media.readyState === 'open') {
      try {
        this.media.removeSourceBuffer(this.buffer)
      } catch {}
    }
    if (this.hls) {
      this.hls.destroy()
      this.hls = null
    }
    if (this.url) {
      URL.revokeObjectURL(this.url)
      this.url = null
    }
    this.buffer = null
    this.media = null
    this.queue = []
    this.video.pause()
    this.video.removeAttribute('src')
    this.video.load()
  }

  followEdge() {
    const video = this.video
    const buffer = this.buffer
    if (!video || !buffer || buffer.buffered.length === 0) {
      return
    }
    const end = buffer.buffered.end(buffer.buffered.length - 1)
    const start = buffer.buffered.start(0)
    const inside = video.currentTime >= start && video.currentTime <= end
    const behind = !inside || video.currentTime < end - LIVE_TARGET_LAG_SECONDS - 2
    if (behind) {
      video.currentTime = Math.max(start, end - LIVE_TARGET_LAG_SECONDS)
      video.play().catch(() => {})
    }
    if (start < end - LIVE_WINDOW_SECONDS && !buffer.updating) {
      try {
        buffer.remove(0, end - LIVE_WINDOW_SECONDS)
      } catch {}
    }
  }

  pump() {
    const buffer = this.buffer
    if (!buffer || buffer.updating) {
      return
    }
    const next = this.queue.shift()
    if (!next) {
      this.followEdge()
      return
    }
    try {
      buffer.appendBuffer(next as BufferSource)
    } catch (error) {
      if (this.transport === 'hevc' && this.stopHevcWatch) {
        this.fallBack('the H.265 fragment was refused')
        return
      }
      console.error('append failed, dropping to the next keyframe', error)
      this.queue = []
    }
  }

  ticket: StreamTicket | null = null

  fallBack(reason: string) {
    const ticket = this.ticket
    rememberHevc('no')
    console.warn('H.265 passthrough failed, switching to the H.264 transcode:', reason)
    this.destroy()
    if (ticket) this.playOverSocket(ticket, false)
  }

  openBuffer(media: MediaSource, type: string): boolean {
    let buffer: SourceBuffer
    try {
      buffer = media.addSourceBuffer(type)
    } catch (error) {
      if (this.transport === 'hevc') {
        this.fallBack('the browser refused ' + type)
        return false
      }
      this.onState('error', 'This browser cannot decode the stream codec: ' + (error as Error).message)
      return false
    }
    this.buffer = buffer
    buffer.addEventListener('updateend', () => {
      this.noteEdge()
      this.pump()
      this.followEdge()
    })
    buffer.addEventListener('error', () => {
      if (this.transport === 'hevc' && this.buffer === buffer) this.fallBack('the source buffer reported an error')
    })
    return true
  }

  watchHevc(media: MediaSource, shape: StreamShape) {
    this.stopHevcWatch = watchFirstFrame(
      this.video,
      () => this.media === media && this.transport === 'hevc',
      (ok, reason) => {
        this.stopHevcWatch = null
        if (ok) rememberHevc('yes', shape)
        else this.fallBack(reason)
      },
    )
  }

  onHello(media: MediaSource, text: string) {
    let hello: { type?: string; mode?: string; codec?: string | null; width?: number | null; height?: number | null; audio?: string | null } | null = null
    try {
      hello = JSON.parse(text)
    } catch {
      hello = null
    }
    if (hello && hello.type === 'hello') this.helloAudio = hello.audio || null
    if (!hello || hello.type !== 'hello' || this.buffer) return
    if (hello.mode !== 'passthrough' || !hello.codec) {
      this.transport = 'avc'
      this.video.dataset.transport = 'avc'
      if (this.openBuffer(media, hello.codec && hello.codec.startsWith('avc1') ? mimeOf(hello.codec) : AVC_TYPE)) this.pump()
      return
    }
    const shape: StreamShape = { codec: hello.codec, width: hello.width, height: hello.height }
    this.transport = 'hevc'
    this.video.dataset.transport = 'hevc'
    this.watchHevc(media, shape)
    decodes(shape).then((ok) => {
      if (this.media !== media) return
      if (!ok) {
        this.fallBack('the browser cannot decode ' + shape.codec + ' at ' + shape.width + 'x' + shape.height)
        return
      }
      if (this.openBuffer(media, mimeOf(shape.codec))) this.pump()
    })
  }

  playOverSocket(ticket: StreamTicket, askHevc: boolean) {
    const video = this.video
    const media = new MediaSource()
    this.ticket = ticket
    this.media = media
    this.url = URL.createObjectURL(media)
    video.src = this.url

    media.addEventListener(
      'sourceopen',
      () => {
        if (this.media !== media) {
          return
        }
        const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
        const base = ticket.url.startsWith('/') ? scheme + '://' + location.host + ticket.url : ticket.url.replace(/^http/, 'ws')
        const target = askHevc ? base + (base.includes('?') ? '&' : '?') + 'codec=hevc' : base
        const socket = new WebSocket(target)
        socket.binaryType = 'arraybuffer'
        this.socket = socket
        socket.addEventListener('open', () => {
          if (this.socket === socket && this.soundOn) this.sendSound()
        })
        socket.onmessage = (event) => {
          if (typeof event.data === 'string') {
            const frame = audioFrame(event.data)
            if (frame) this.hear(frame)
            else this.onHello(media, event.data)
            return
          }
          this.queue.push(new Uint8Array(event.data as ArrayBuffer))
          if (!this.buffer && !this.transport) {
            this.transport = 'avc'
            this.video.dataset.transport = 'avc'
            this.openBuffer(media, AVC_TYPE)
          }
          this.pump()
        }
        socket.onerror = () => this.onState('error', 'The live socket failed to connect.')
        socket.onclose = () => {
          if (!video.paused) {
            this.onState('error', 'Live socket closed.')
          }
        }
        video.muted = this.muted()
        this.expectFirstFrame()
        this.startSound()
        video.play().catch(() => {})
      },
      { once: true },
    )
  }

  async play(ticket: StreamTicket) {
    const video = this.video
    if (ticket.kind === 'ws') {
      if (!window.MediaSource) {
        this.onState('error', 'This browser cannot play the live socket (no MediaSource).')
        return
      }
      this.ticket = ticket
      const askHevc = await shouldAskForHevc()
      if (this.ticket !== ticket) return
      this.playOverSocket(ticket, askHevc)
      return
    }
    if (ticket.kind === 'mp4' || ticket.url.endsWith('.mp4')) {
      video.src = ticket.url
      this.expectFirstFrame()
      await video.play().catch(() => {})
      return
    }
    if (ticket.kind !== 'hls') {
      this.onState('error', 'Unsupported stream kind: ' + ticket.kind)
      return
    }
    const Hls = (window as unknown as { Hls?: HlsConstructor }).Hls
    if (Hls && Hls.isSupported()) {
      const hls = new Hls({ lowLatencyMode: true })
      hls.on(Hls.Events.ERROR, (_event, data) => {
        if (data && data.fatal) {
          this.onState('error', 'Playlist could not be loaded: ' + data.type)
        }
      })
      hls.loadSource(ticket.url)
      hls.attachMedia(video)
      this.hls = hls
      this.expectFirstFrame()
      await video.play().catch(() => {})
      return
    }
    if (video.canPlayType('application/vnd.apple.mpegurl')) {
      video.src = ticket.url
      this.expectFirstFrame()
      await video.play().catch(() => {})
      return
    }
    this.onState('error', 'This browser has no HLS support and no hls.js is loaded.')
  }
}
