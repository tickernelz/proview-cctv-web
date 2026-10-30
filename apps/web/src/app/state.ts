import type { ModeReport, Prefs } from '../live/store'

export type Device = {
  id: string
  name: string
  model?: string
  online?: boolean
  online_status?: string
  online_detail?: string
  channels?: number
  lan_host?: string
}
export type Camera = { device: Device; channel: number; key: string; name: string }

const PREFS_KEY = 'pvw_prefs_v2'
export const SECTIONS = ['playback', 'camera', 'device', 'connection', 'config', 'server', 'alarm', 'account']

function defaultPrefs(): Prefs {
  return { theme: 'system', quality: 'main', lens: 'combined', osd: true, muted: true, flip: false }
}

function loadPrefs(): Prefs {
  let stored: Partial<Prefs> = {}
  try {
    stored = JSON.parse(localStorage.getItem(PREFS_KEY) || '{}') || {}
  } catch {
    stored = {}
  }
  let legacyTheme: string | null = null
  try {
    legacyTheme = localStorage.getItem('pvw_theme')
  } catch {
    legacyTheme = null
  }
  const prefs = { ...defaultPrefs(), ...stored }
  if (!stored.theme && (legacyTheme === 'dark' || legacyTheme === 'light')) prefs.theme = legacyTheme
  return prefs
}

export const state = {
  screen: 'boot',
  view: 'live' as string | null,
  section: 'playback',
  signedIn: false,
  account: null as string | null,
  devices: [] as Device[],
  cameras: [] as Camera[],
  selectedId: null as string | null,
  channel: 1,
  lens: null as any,
  lensSupport: 'unknown',
  lensView: 'combined' as string | number,
  control: { rows: [] as any[], onvif: false, dvrip: false, error: null as string | null, checkedFor: null as string | null },
  players: new Map<string, any>(),
  ticket: null as any,
  mode: null as ModeReport | null,
  recorder: null as MediaRecorder | null,
  recordChunks: [] as Blob[],
  recordStart: 0,
  muted: true,
  stage: { name: 'idle', requestedAt: 0, failedAt: 0, lastFrameAt: 0, error: null as any, attempt: null as any },
  padOpen: false,
  tour: null as any,
  bannerError: '',
  networkLostAt: null as number | null,
  expiredAt: null as number | null,
  resume: null as { id: string; channel: number } | null,
  prefs: loadPrefs(),
}

export function savePrefs() {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(state.prefs))
  } catch {
    return
  }
}
