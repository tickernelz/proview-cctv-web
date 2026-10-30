export const AVC_TYPE = 'video/mp4; codecs="avc1.42E01E"'
export const HEVC_PROBE_CODEC = 'hvc1.1.6.L153.B0'
export const HEVC_FIRST_FRAME_MS = 4000

const VERDICT_KEY = 'proview.hevc'
const CODEC_KEY = 'proview.hevc.codec'

export type HevcVerdict = 'yes' | 'no'
export type StreamShape = { codec: string; width?: number | null; height?: number | null }

function read(key: string): string | null {
  try {
    return window.sessionStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string) {
  try {
    window.sessionStorage.setItem(key, value)
  } catch {}
}

export function mimeOf(codec: string) {
  return 'video/mp4; codecs="' + codec + '"'
}

export function hevcVerdict(): HevcVerdict | null {
  const value = read(VERDICT_KEY)
  return value === 'yes' || value === 'no' ? value : null
}

export function rememberHevc(verdict: HevcVerdict, shape?: StreamShape) {
  if (verdict === 'yes' && hevcVerdict() === 'no') return
  write(VERDICT_KEY, verdict)
  if (shape) write(CODEC_KEY, JSON.stringify(shape))
}

export function framesShown(video: HTMLVideoElement) {
  if (typeof video.getVideoPlaybackQuality === 'function') return video.getVideoPlaybackQuality().totalVideoFrames > 0
  return video.currentTime > 0
}

export function watchFirstFrame(video: HTMLVideoElement, current: () => boolean, settle: (ok: boolean, reason: string) => void): () => void {
  const target = video as HTMLVideoElement & {
    requestVideoFrameCallback?: (callback: () => void) => number
    cancelVideoFrameCallback?: (handle: number) => void
  }
  let done = false
  let timer = 0
  let frame: number | null = null
  const cleanup = () => {
    done = true
    if (timer) window.clearTimeout(timer)
    timer = 0
    if (frame !== null && target.cancelVideoFrameCallback) target.cancelVideoFrameCallback(frame)
    frame = null
    target.removeEventListener('loadeddata', shown)
    target.removeEventListener('error', failed)
    document.removeEventListener('visibilitychange', visible)
  }
  const finish = (ok: boolean, reason: string) => {
    if (done) return
    cleanup()
    if (current()) settle(ok, reason)
  }
  const shown = () => finish(true, '')
  const failed = () => finish(false, 'the video element failed to decode')
  const check = () => {
    timer = 0
    if (done || !current()) return cleanup()
    if (framesShown(video)) return finish(true, '')
    if (document.hidden) {
      document.addEventListener('visibilitychange', visible)
      return
    }
    finish(false, 'no frame decoded within ' + HEVC_FIRST_FRAME_MS / 1000 + ' s')
  }
  const visible = () => {
    if (document.hidden || done) return
    document.removeEventListener('visibilitychange', visible)
    timer = window.setTimeout(check, HEVC_FIRST_FRAME_MS)
  }
  if (target.requestVideoFrameCallback) frame = target.requestVideoFrameCallback(shown)
  else target.addEventListener('loadeddata', shown)
  target.addEventListener('error', failed)
  timer = window.setTimeout(check, HEVC_FIRST_FRAME_MS)
  return () => {
    if (!done) cleanup()
  }
}

function knownShape(): StreamShape {
  const raw = read(CODEC_KEY)
  if (raw) {
    try {
      const shape = JSON.parse(raw) as StreamShape
      if (shape && typeof shape.codec === 'string') return shape
    } catch {}
  }
  return { codec: HEVC_PROBE_CODEC }
}

function mediaSourceSupports(codec: string) {
  const media = window.MediaSource
  return !!media && typeof media.isTypeSupported === 'function' && media.isTypeSupported(mimeOf(codec))
}

export async function decodes(shape: StreamShape): Promise<boolean> {
  if (!mediaSourceSupports(shape.codec)) return false
  const decoder = (window as unknown as { VideoDecoder?: { isConfigSupported?: (config: Record<string, unknown>) => Promise<{ supported?: boolean }> } }).VideoDecoder
  if (!decoder || typeof decoder.isConfigSupported !== 'function') return true
  const config: Record<string, unknown> = { codec: shape.codec }
  if (shape.width && shape.height) {
    config.codedWidth = shape.width
    config.codedHeight = shape.height
  }
  try {
    const answer = await decoder.isConfigSupported(config)
    return !!(answer && answer.supported)
  } catch {
    return false
  }
}

export function mayAskForHevc(): boolean {
  return hevcVerdict() !== 'no' && mediaSourceSupports(knownShape().codec)
}

export async function shouldAskForHevc(): Promise<boolean> {
  if (hevcVerdict() === 'no') return false
  return decodes(knownShape())
}
