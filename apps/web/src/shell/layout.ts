import { computed, reactive } from 'vue'
import { EVENTS_HASH, go, liveHash, type LiveMode, type View } from './router'

const PANEL_KEY = 'pvw_events_panel'
const wideQuery = window.matchMedia('(min-width: 1280px)')
const narrowQuery = window.matchMedia('(max-width: 639px)')

function storedPanel(): boolean | null {
  try {
    const value = localStorage.getItem(PANEL_KEY)
    return value === 'open' ? true : value === 'closed' ? false : null
  } catch {
    return null
  }
}

export const shell = reactive({
  screen: 'boot',
  view: 'live' as View,
  mode: 'single' as Exclude<LiveMode, 'playback'>,
  docked: storedPanel() ?? true,
  overlay: false,
  wide: wideQuery.matches,
  narrow: narrowQuery.matches,
})

wideQuery.addEventListener('change', (event) => {
  shell.wide = event.matches
  shell.overlay = false
})
narrowQuery.addEventListener('change', (event) => {
  shell.narrow = event.matches
})

export const panelOpen = computed(() => (shell.narrow ? false : shell.wide ? shell.docked : shell.overlay))
export const eventsOpen = computed(() => shell.screen === 'watch' && (shell.view === 'events' || panelOpen.value))

function setPanel(open: boolean) {
  if (!shell.wide) {
    shell.overlay = open
    return
  }
  shell.docked = open
  try {
    localStorage.setItem(PANEL_KEY, open ? 'open' : 'closed')
  } catch {
    return
  }
}

export function openEvents() {
  if (shell.narrow) {
    go(EVENTS_HASH)
    return
  }
  setPanel(true)
  if (shell.screen !== 'watch') go(liveHash(shell.mode))
}

export function closeEvents() {
  if (shell.view === 'events') go(liveHash(shell.mode))
  else setPanel(false)
}

export function toggleEvents() {
  if (eventsOpen.value) closeEvents()
  else openEvents()
}
