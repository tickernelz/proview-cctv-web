const pad2 = (value: number) => String(value).padStart(2, '0')

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

export function hm(ms: number) {
  const at = new Date(ms)
  return pad2(at.getHours()) + ':' + pad2(at.getMinutes())
}

export function hms(ms: number) {
  const at = new Date(ms)
  return pad2(at.getHours()) + ':' + pad2(at.getMinutes()) + ':' + pad2(at.getSeconds())
}

export function dayLabel(day: string, relative: boolean) {
  const [year, month, date] = day.split('-').map(Number)
  if (!year || !month || !date) return day
  const at = new Date(year, month - 1, date)
  const short = date + ' ' + MONTHS[month - 1]
  if (!relative) return short
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const gap = Math.round((today - at.getTime()) / 86400000)
  if (gap === 0) return 'Today, ' + short
  if (gap === 1) return 'Yesterday, ' + short
  return WEEKDAYS[at.getDay()] + ', ' + short + (year !== now.getFullYear() ? ' ' + year : '')
}
