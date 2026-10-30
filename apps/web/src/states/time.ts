const two = (value: number) => String(value).padStart(2, '0')

export function clockHm(time: number) {
  const date = new Date(time)
  return two(date.getHours()) + ':' + two(date.getMinutes())
}

export function clockHms(time: number) {
  const date = new Date(time)
  return clockHm(time) + ':' + two(date.getSeconds())
}

export function dayLabel(day: number | Date) {
  return new Date(day).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

export function detailText(details: string | Record<string, string> | Array<[string, string]>) {
  if (typeof details === 'string') return details
  const rows = Array.isArray(details) ? details : Object.entries(details)
  const width = Math.max(0, ...rows.map(([key]) => key.length)) + 2
  return rows.map(([key, value]) => key.padEnd(width) + value).join('\n')
}
