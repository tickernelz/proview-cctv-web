import { reactive } from 'vue'
import { state } from '../../app/state'

type Feature<T> = { supported: boolean; evidence: string; value: T | null }
export type ImageLiveView = { low_lux: Feature<boolean>; privacy: Feature<boolean>; sleep: Feature<{ enable: boolean; sleeping: boolean }> }

export const imageLive = reactive({ view: null as ImageLiveView | null, busy: false, note: '' })

let asked = ''

function deviceQuery(separator: '?' | '&') {
  const id = state.selectedId || ''
  return id ? separator + 'device=' + encodeURIComponent(id) : ''
}

export function resetImageLive() {
  imageLive.view = null
  imageLive.note = ''
}

export async function readImageLive() {
  const id = state.selectedId || ''
  if (!id || state.screen !== 'watch') return
  asked = id
  try {
    const response = await fetch('/api/device/image/live' + deviceQuery('?'), { credentials: 'same-origin' })
    if (asked !== id) return
    imageLive.view = response.ok ? ((await response.json()) as ImageLiveView) : null
  } catch {
    imageLive.view = null
  }
}

async function post(path: string, done: string) {
  if (imageLive.busy) return
  imageLive.busy = true
  try {
    const response = await fetch('/api/device/image/' + path + deviceQuery('&'), { method: 'POST', credentials: 'same-origin' })
    const body = await response.json().catch(() => ({}))
    imageLive.note = response.ok ? done : 'The camera refused it: ' + (body.message || 'HTTP ' + response.status)
  } finally {
    imageLive.busy = false
  }
  await readImageLive()
}

export const toggleLowLux = () => post('low-lux?enable=' + String(!imageLive.view?.low_lux.value), 'Low-lux mode changed.')
export const uncoverLens = () => post('privacy?enable=false', 'The lens is uncovered; the picture comes back with the next frame.')
export const wakeCamera = () => post('sleep?wake=true', 'The camera was asked to wake up.')
