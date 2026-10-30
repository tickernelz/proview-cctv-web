import { reactive } from 'vue'

const NARROW = '(max-width: 1023px), (max-height: 599px)'

export const padSheet = reactive({ on: false })

function update(query: MediaQueryList) {
  padSheet.on = query.matches && !document.fullscreenElement
}

if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
  const query = window.matchMedia(NARROW)
  update(query)
  query.addEventListener('change', () => update(query))
  document.addEventListener('fullscreenchange', () => update(query))
}
