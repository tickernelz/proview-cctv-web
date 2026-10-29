import { live } from '../live/store'
import { savePrefs, state } from './state'

export function resolveTheme() {
  const chosen = state.prefs.theme
  if (chosen === 'light' || chosen === 'dark') return chosen
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

export function applyTheme() {
  const theme = resolveTheme()
  document.documentElement.dataset.theme = theme
  live.theme = theme
  live.settings.prefs.theme = state.prefs.theme
  for (const button of document.querySelectorAll('[data-theme-toggle]')) {
    button.setAttribute('aria-label', theme === 'dark' ? 'Switch to the light theme' : 'Switch to the dark theme')
    button.querySelector('use')?.setAttribute('href', theme === 'dark' ? '#i-sun' : '#i-moon')
  }
}

export function toggleTheme() {
  state.prefs.theme = resolveTheme() === 'dark' ? 'light' : 'dark'
  savePrefs()
  applyTheme()
}

for (const button of document.querySelectorAll('[data-theme-toggle]')) button.addEventListener('click', toggleTheme)

window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => {
  if (state.prefs.theme === 'system') applyTheme()
})
