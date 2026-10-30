import { renderControlAvailability, togglePad } from './controls'
import { chooseLens } from './lens'
import { live } from '../live/store'
import { startTalk, stopTalk } from '../live/talk'
import { renderCaption } from './caption'
import { renderBanner } from './chrome'
import { captureSnapshot, renderMute, toggleMute, toggleRecord } from './media'
import { openSettings, renderPrefsIntoForm, showSection, signedIn } from './shell'
import { requestStream, tick } from './stage'
import { state } from './state'
import { applyTheme } from './theme'
import { setQuality, toggleFullscreen } from './view'

Object.assign(live.actions, { startTalk, stopTalk, toggleMute, toggleRecord, captureSnapshot, togglePad, toggleFullscreen, chooseLens, setQuality })

window.addEventListener('offline', () => {
  state.networkLostAt = Date.now()
  renderBanner()
  renderCaption()
})

window.addEventListener('online', () => {
  state.networkLostAt = null
  renderBanner()
  renderCaption()
  if (state.screen === 'watch' && state.view === 'live' && state.stage.name !== 'playing') requestStream()
})

export function boot() {
  state.muted = state.prefs.muted
  state.lensView = state.prefs.lens
  renderMute()
  renderPrefsIntoForm()
  applyTheme()
  renderControlAvailability()
  tick()
  signedIn()
  Object.assign(window, { openSettings, showSection, state, signedIn, setQuality })
}
