export function hms(time: number) {
  return new Date(time).toLocaleTimeString([], { hour12: false })
}

export function hm(time: number) {
  return new Date(time).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit' })
}
