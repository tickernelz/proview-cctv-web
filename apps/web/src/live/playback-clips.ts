export type Clip = { name: string; begin: string; end: string; bytes: number }

export function stamp(text: string) {
  const value = Date.parse(String(text).replace(' ', 'T'))
  return Number.isFinite(value) ? value : null
}

export function mb(bytes: number) {
  return (bytes / 1048576).toFixed(2) + ' MB'
}

export const modeOf = (file: Clip) => (/\[M\]/.test(file.name) ? 'motion' : 'continuous')

export const span = (file: Clip) => file.begin.slice(11) + ' – ' + file.end.slice(11)

export function secondsBetween(begin: string, end: string) {
  const a = stamp(begin)
  const b = stamp(end)
  if (a === null || b === null || b <= a) return 0
  return Math.round((b - a) / 1000)
}
