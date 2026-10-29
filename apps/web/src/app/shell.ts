import { api } from '../lib/api'
import { live, type Prefs } from '../live/store'
import {
  layoutLensCrop,
  loadDevices,
  renderCameraInfo,
  renderControlAvailability,
  renderFsLine,
  renderMute,
  setQuality,
  setView,
  stopAll,
} from '../legacy/app.js'
import { closeFloating } from './chrome'
import { savePrefs, SECTIONS, state } from './state'
import { applyTheme } from './theme'

export function showScreen(name: string) {
  closeFloating(false)
  state.screen = name
  for (const id of ['boot', 'signin', 'watch', 'settings']) {
    const node = document.getElementById(id)
    if (node) node.hidden = id !== name
  }
  document.body.dataset.screen = name
  live.settings.shown = name === 'settings'
}

export function showSignin(options: { expired?: boolean; focus?: boolean } = {}) {
  stopAll()
  state.signedIn = false
  state.devices = []
  state.cameras = []
  state.selectedId = null
  live.settings.signedIn = false
  showScreen('signin')
  const signin = live.signin
  signin.expired = Boolean(options.expired)
  signin.expiredAt = state.expiredAt || 0
  signin.lastFrameAt = state.stage.lastFrameAt || 0
  signin.generation += 1
  if (options.focus !== false) signin.focus += 1
}

export function sessionLost(error: unknown) {
  const failure = error as { status?: number } | null
  if (!failure || failure.status !== 401 || !state.signedIn) return false
  state.expiredAt = Date.now()
  state.resume = state.selectedId ? { id: state.selectedId, channel: state.channel } : null
  showSignin({ expired: true })
  return true
}

export async function signedIn() {
  let me: { account?: string }
  try {
    me = await api('/api/me')
  } catch {
    showSignin()
    await loadMode()
    return
  }
  state.signedIn = true
  state.account = me.account || null
  live.settings.signedIn = true
  live.settings.account = state.account || ''
  if (state.screen !== 'settings') showScreen('watch')
  await loadMode()
  await loadDevices()
  if (state.screen === 'settings') {
    stopAll()
    renderSettingsChrome()
    return
  }
  route(location.hash, false)
}

export function route(hash: string, focus: boolean) {
  const raw = (hash || '').replace(/^#/, '')
  if (raw !== 'playback') live.screen = ''
  if (raw.startsWith('settings')) {
    openSettings(raw.split('/')[1] || state.section, { focus })
    return
  }
  if (!state.signedIn) return
  if (state.screen !== 'watch') showScreen('watch')
  if (raw === 'playback') {
    live.screen = 'playback'
    return
  }
  setView(raw === 'grid' && state.cameras.length > 1 ? 'grid' : 'live')
}

export function modeName(mode: string | undefined) {
  if (mode === 'local') return 'this network'
  if (mode === 'cloud') return 'cloud account'
  return 'unknown'
}

export function sourceLabel() {
  if (state.ticket && state.ticket.source) return state.ticket.source
  if (state.mode && state.mode.source) return state.mode.source
  return 'source unknown'
}

export async function loadMode() {
  try {
    state.mode = await api('/api/config/mode')
  } catch (error) {
    state.mode = null
    console.error('mode report failed', error)
  }
  renderMode()
}

function renderMode() {
  live.signin.mode = state.mode
  live.settings.mode = state.mode
  live.settings.signedIn = state.signedIn
  if (state.mode) renderCameraInfo()
}

export async function setMode(mode: string) {
  live.settings.modeError = ''
  try {
    state.mode = await api('/api/config/mode', { method: 'POST', body: JSON.stringify({ mode }) })
    state.control = { rows: [], onvif: false, dvrip: false, error: null, checkedFor: null }
    state.tour = null
    renderMode()
    await loadDevices({ keepScreen: true })
  } catch (error) {
    if (sessionLost(error)) return
    await loadMode()
    live.settings.modeError = 'The server did not change the provider: ' + (error as Error).message
  }
}

export async function signOut() {
  await api('/api/logout', { method: 'POST' }).catch(() => null)
  state.stage.lastFrameAt = 0
  live.settings.reset += 1
  showSignin()
  await loadMode()
}

export function showSection(name: string, focus?: boolean) {
  const section = SECTIONS.includes(name) ? name : 'playback'
  state.section = section
  live.settings.section = section
  if (focus) live.settings.focusHeading += 1
}

function renderSettingsChrome() {
  renderMode()
  renderCameraInfo()
  renderControlAvailability()
}

export function renderPrefsIntoForm() {
  Object.assign(live.settings.prefs, state.prefs)
  live.quality = state.prefs.quality
}

export function openSettings(section?: string, options: { focus?: boolean } = {}) {
  stopAll()
  live.settings.modeError = ''
  showScreen('settings')
  showSection(section || state.section, options.focus)
  if (location.hash !== '#settings/' + state.section) history.replaceState(null, '', '#settings/' + state.section)
  renderPrefsIntoForm()
  renderSettingsChrome()
  live.settings.load += 1
  if (!options.focus) live.settings.focusBack += 1
}

export function leaveSettings() {
  if (!state.signedIn) {
    showSignin()
    return
  }
  showScreen('watch')
  const view = state.view
  state.view = null
  setView(view)
  document.getElementById('open-settings')?.focus()
}

export function setPref<K extends keyof Prefs>(name: K, value: Prefs[K]) {
  if (name === 'quality') {
    setQuality(value as string)
    return
  }
  state.prefs[name] = value
  savePrefs()
  renderPrefsIntoForm()
  if (name === 'theme') applyTheme()
  else if (name === 'osd') renderFsLine()
  else if (name === 'muted') {
    state.muted = value as boolean
    renderMute()
  } else if (name === 'flip') layoutLensCrop()
}

window.addEventListener('hashchange', () => route(location.hash, true))

window.addEventListener('desktop-live', () => {
  if (state.signedIn) {
    live.screen = ''
    showScreen('watch')
    setView('live')
  }
})
