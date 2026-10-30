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
  write(VERDICT_KEY, verdict)
  if (shape) write(CODEC_KEY, JSON.stringify(shape))
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
