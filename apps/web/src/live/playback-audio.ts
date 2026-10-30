const TABLE = (() => {
  const table = new Float32Array(256)
  for (let byte = 0; byte < 256; byte += 1) {
    const value = byte ^ 0x55
    const exponent = (value & 0x70) >> 4
    const mantissa = value & 0x0f
    const magnitude = exponent === 0 ? (mantissa << 4) + 8 : ((mantissa << 4) + 0x108) << (exponent - 1)
    table[byte] = ((value & 0x80) !== 0 ? magnitude : -magnitude) / 32768
  }
  return table
})()

export function decodeAlaw(base64: string) {
  const raw = atob(base64)
  const out = new Float32Array(raw.length)
  for (let index = 0; index < raw.length; index += 1) out[index] = TABLE[raw.charCodeAt(index)]
  return out
}

type Chunk = { at: number; samples: Float32Array; rate: number }

export class PlaybackSound {
  context: AudioContext | null = null
  enabled = false
  queue: Chunk[] = []
  next = 0
  received = 0
  played = 0

  enable() {
    const Context = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!Context) return false
    if (!this.context) this.context = new Context()
    void this.context.resume()
    this.enabled = true
    this.next = 0
    return true
  }

  disable() {
    this.enabled = false
    this.next = 0
    if (this.context) void this.context.suspend()
  }

  enqueue(at: number, samples: Float32Array, rate: number) {
    this.received += 1
    this.queue.push({ at, samples, rate })
    if (this.queue.length > 600) this.queue.splice(0, this.queue.length - 600)
  }

  clear() {
    this.queue = []
    this.next = 0
  }

  tick(now: number, playing: boolean) {
    while (this.queue.length && this.queue[0].at + this.queue[0].samples.length / this.queue[0].rate < now - 0.1) this.queue.shift()
    const context = this.context
    if (!this.enabled || !playing || !context || context.state !== 'running') {
      this.next = 0
      return
    }
    while (this.queue.length && this.queue[0].at <= now + 0.5) {
      const chunk = this.queue.shift() as Chunk
      const target = context.currentTime + (chunk.at - now)
      const start = this.next > context.currentTime && Math.abs(target - this.next) < 0.25 ? this.next : Math.max(target, context.currentTime)
      const buffer = context.createBuffer(1, chunk.samples.length, chunk.rate)
      buffer.copyToChannel(chunk.samples as Float32Array<ArrayBuffer>, 0)
      const source = context.createBufferSource()
      source.buffer = buffer
      source.connect(context.destination)
      source.start(start)
      this.next = start + chunk.samples.length / chunk.rate
      this.played += 1
    }
  }
}
