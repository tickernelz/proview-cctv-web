export type ApiError = Error & { status?: number; code?: string; body?: unknown }

export async function api<T = any>(path: string, options: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json' },
    ...options,
  })
  const text = await response.text()
  let body: any = null
  if (text) {
    try {
      body = JSON.parse(text)
    } catch {
      body = null
    }
  }
  if (!response.ok) {
    const message = body && body.message ? body.message : response.status + ' ' + response.statusText
    const error: ApiError = new Error(message)
    error.status = response.status
    if (body && body.code) error.code = body.code
    if (body) error.body = body
    throw error
  }
  return body
}
