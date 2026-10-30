import { AVC_TYPE, decodes, mimeOf, rememberHevc, shouldAskForHevc, watchFirstFrame, type StreamShape } from './hevc'

export const LIVE_WINDOW_SECONDS = 12
export const LIVE_TARGET_LAG_SECONDS = 1.0

export type StreamTicket = { kind: string; url: string }
export type PlayerState = 'playing' | 'stalled' | 'error'

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

  destroy() {
    this.clearHevcWatch()
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
    let hello: { type?: string; mode?: string; codec?: string | null; width?: number | null; height?: number | null } | null = null
    try {
      hello = JSON.parse(text)
    } catch {
      hello = null
    }
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
        socket.onmessage = (event) => {
          if (typeof event.data === 'string') {
            this.onHello(media, event.data)
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
