export type View = 'live' | 'events' | 'settings'
export type LiveMode = 'single' | 'grid' | 'playback'
export type Route = { view: View; mode: LiveMode; section: string }

export function parseHash(hash: string): Route {
  const parts = (hash || '').replace(/^#\/?/, '').split('/').filter(Boolean)
  if (parts[0] === 'settings') return { view: 'settings', mode: 'single', section: parts[1] || '' }
  if (parts[0] === 'events') return { view: 'events', mode: 'single', section: '' }
  const mode = parts[0] === 'live' && (parts[1] === 'grid' || parts[1] === 'playback') ? parts[1] : 'single'
  return { view: 'live', mode, section: '' }
}

export const liveHash = (mode: LiveMode = 'single') => (mode === 'single' ? '#/live' : '#/live/' + mode)
export const settingsHash = (section: string) => '#/settings/' + section
export const EVENTS_HASH = '#/events'

export function replaceHash(hash: string) {
  if (location.hash !== hash) history.replaceState(null, '', hash)
}

export function go(hash: string) {
  if (location.hash !== hash) location.hash = hash
}
