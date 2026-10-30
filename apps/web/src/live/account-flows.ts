export function stamp(value?: string | null) {
  const text = (value || '').trim()
  if (!text) return 'not reported'
  if (!/^\d+$/.test(text)) return text
  const raw = Number(text)
  const date = new Date(raw > 1e12 ? raw : raw * 1000)
  return Number.isNaN(date.getTime()) ? text : date.toLocaleString()
}

export function sentence(error: unknown) {
  const text = error instanceof Error ? error.message : String(error)
  return text.charAt(0).toUpperCase() + text.slice(1)
}

export function looksLikeMail(value: string) {
  const at = value.trim().indexOf('@')
  return at > 0 && value.trim().slice(at + 1).includes('.')
}
