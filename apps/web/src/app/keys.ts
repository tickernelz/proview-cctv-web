import { closePad, sendControl, togglePad } from './controls'
import { cycleLens } from './lens'
import { live, menuOpen } from '../live/store'
import { startTalk, stopTalk } from '../live/talk'
import { closeFloating, toggleKeys } from './chrome'
import { captureSnapshot, toggleMute, toggleRecord } from './media'
import { openSettings } from './shell'
import { state } from './state'
import { moveTileFocus, openCamera, setQuality, setView, toggleFullscreen } from './view'

const ARROWS: Record<string, string> = { ArrowUp: 'up', ArrowDown: 'down', ArrowLeft: 'left', ArrowRight: 'right' }

function typing(target: EventTarget | null) {
  return Boolean(target instanceof Element && target.closest('input, textarea, select, [contenteditable="true"]'))
}

function onWatchKey(event: KeyboardEvent) {
  const key = event.key
  if (key === 'Escape') {
    if (closeFloating(true)) {
      event.preventDefault()
    } else if (state.padOpen) {
      event.preventDefault()
      closePad()
      document.getElementById('move')?.focus()
    } else if (document.fullscreenElement) {
      event.preventDefault()
      document.exitFullscreen().catch(() => {})
    }
    return
  }
  if (event.ctrlKey || event.metaKey || event.altKey || typing(event.target)) return
  if (/^[1-9]$/.test(key)) {
    event.preventDefault()
    closeFloating(false)
    openCamera(Number(key) - 1)
    return
  }
  if (key.startsWith('Arrow')) {
    if (state.view === 'live' && state.padOpen) {
      event.preventDefault()
      const act = ARROWS[key]
      if (live.gate[act].ok) sendControl(act)
    } else if (state.view === 'grid') {
      event.preventDefault()
      moveTileFocus(key)
    }
    return
  }
  if (key === ' ') {
    if (state.padOpen && !(event.target instanceof Element && event.target.closest('button'))) {
      event.preventDefault()
      if (live.gate.home.ok) sendControl('home')
    }
    return
  }
  const actions: Record<string, () => unknown> = {
    g: () => (live.top.gridDisabled ? null : setView(state.view === 'grid' ? 'live' : 'grid')),
    f: () => toggleFullscreen(),
    m: () => toggleMute(),
    r: () => (live.record.disabled ? null : toggleRecord()),
    s: () => (live.snapshotDisabled ? null : captureSnapshot()),
    l: () => cycleLens(),
    q: () => setQuality(state.prefs.quality === 'sub' ? 'main' : 'sub'),
    v: () => togglePad(),
    t: () => (event.repeat ? null : startTalk()),
    ',': () => openSettings(state.section),
    '?': () => toggleKeys(),
  }
  const action = actions[key.length === 1 ? key.toLowerCase() : key]
  if (action) {
    event.preventDefault()
    action()
  }
}

document.addEventListener('keydown', (event) => {
  if (event.defaultPrevented || menuOpen() || live.screen) return
  if (state.screen === 'watch') onWatchKey(event)
})

document.addEventListener('keyup', (event) => {
  if (event.key.toLowerCase() === 't') stopTalk()
})

window.addEventListener('blur', () => stopTalk())

document.querySelector('.skip')?.addEventListener('click', (event) => {
  event.preventDefault()
  const controls = document.getElementById('controls')
  if (!controls) return
  const first = controls.querySelector('button:not(:disabled):not([hidden])') as HTMLElement | null
  ;(first || controls).focus()
})
