export type DstRule = { on: false } | { on: true; start: string; end: string }

const pad = (value: number) => String(value).padStart(2, '0')

function offsets(year: number) {
  return [new Date(year, 0, 1).getTimezoneOffset(), new Date(year, 6, 1).getTimezoneOffset()]
}

export function baseOffsetMinutes(now = new Date()) {
  return -Math.max(...offsets(now.getFullYear()))
}

export function dstRule(now = new Date()): DstRule {
  const year = now.getFullYear()
  const [winter, summer] = offsets(year)
  if (winter === summer) return { on: false }
  const standard = Math.max(winter, summer)
  const inDst = (y: number, month: number, day: number) => new Date(y, month - 1, day).getTimezoneOffset() < standard
  let begin = 0
  let end = 0
  for (let month = 1; month <= 12; month++) {
    const daylight = inDst(year, month, 1)
    if (begin === 0 && daylight) begin = month
    if (begin > 0 && end === 0 && !daylight) end = month
    if (begin > 0 && end > 0 && daylight) {
      begin = month
      break
    }
  }
  let beginDay = 0
  let endDay = 0
  if (begin > 1) {
    beginDay = 1
    for (let day = 1; day <= 31; day++) {
      if (inDst(year, begin - 1, day)) {
        beginDay = day
        begin--
        break
      }
    }
  }
  if (end > 1) {
    endDay = 1
    const endYear = begin > end ? year + 1 : year
    for (let day = 1; day <= 31; day++) {
      if (!inDst(endYear, end - 1, day)) {
        end--
        endDay = day
        break
      }
    }
  }
  const endYear = begin > end ? year + 1 : year
  return { on: true, start: year + '-' + pad(begin) + '-' + pad(beginDay), end: endYear + '-' + pad(end) + '-' + pad(endDay) }
}
