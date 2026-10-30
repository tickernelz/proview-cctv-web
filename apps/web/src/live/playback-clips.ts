export type Clip = { name: string; begin: string; end: string; bytes: number }

export function stamp(text: string) {
  const value = Date.parse(String(text).replace(' ', 'T'))
  return Number.isFinite(value) ? value : null
}

const pad = (value: number) => String(value).padStart(2, '0')

export function deviceTime(ms: number) {
  const at = new Date(ms)
  return at.getFullYear() + '-' + pad(at.getMonth() + 1) + '-' + pad(at.getDate()) + ' ' + pad(at.getHours()) + ':' + pad(at.getMinutes()) + ':' + pad(at.getSeconds())
}

export function mb(bytes: number) {
  return (bytes / 1048576).toFixed(2) + ' MB'
}

export const modeOf = (file: Clip) => (/\[A\]/.test(file.name) ? 'alarm' : /\[M\]/.test(file.name) ? 'motion' : 'continuous')

export const span = (file: Clip) => file.begin.slice(11) + ' – ' + file.end.slice(11)

export function secondsBetween(begin: string, end: string) {
  const a = stamp(begin)
  const b = stamp(end)
  if (a === null || b === null || b <= a) return 0
  return Math.round((b - a) / 1000)
}

export function fileAtTime(files: Clip[], at: number) {
  for (const file of files) {
    const from = stamp(file.begin)
    const to = stamp(file.end)
    if (from !== null && to !== null && at >= from && at < to) return file
  }
  return null
}
