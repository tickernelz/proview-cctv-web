import { ref } from 'vue'

export const gridNow = ref(Date.now())

let timer = 0

export function runGridClock(on: boolean) {
  window.clearInterval(timer)
  timer = 0
  if (!on) return
  gridNow.value = Date.now()
  timer = window.setInterval(() => (gridNow.value = Date.now()), 1000)
}
