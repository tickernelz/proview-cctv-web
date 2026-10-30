<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { state } from '../app/state'
import { live } from './store'

type Feature<T> = { supported: boolean; evidence: string; value: T | null }
type Live = { low_lux: Feature<boolean>; privacy: Feature<boolean>; sleep: Feature<{ enable: boolean; sleeping: boolean }> }

const view = ref<Live | null>(null)
const busy = ref(false)
const note = ref('')
let asked = ''

function deviceQuery(separator: '?' | '&') {
  const id = state.selectedId || ''
  return id ? separator + 'device=' + encodeURIComponent(id) : ''
}

async function read() {
  const id = state.selectedId || ''
  if (!id || state.screen !== 'watch') return
  asked = id
  try {
    const response = await fetch('/api/device/image/live' + deviceQuery('?'), { credentials: 'same-origin' })
    if (asked !== id) return
    view.value = response.ok ? ((await response.json()) as Live) : null
  } catch {
    view.value = null
  }
}

async function post(path: string, done: string) {
  if (busy.value) return
  busy.value = true
  try {
    const response = await fetch('/api/device/image/' + path + deviceQuery('&'), { method: 'POST', credentials: 'same-origin' })
    const body = await response.json().catch(() => ({}))
    note.value = response.ok ? done : 'The camera refused it: ' + (body.message || 'HTTP ' + response.status)
  } finally {
    busy.value = false
  }
  await read()
}

const toggleLowLux = () => post('low-lux?enable=' + String(!view.value?.low_lux.value), 'Low-lux mode changed.')
const uncover = () => post('privacy?enable=false', 'The lens is uncovered; the picture comes back with the next frame.')
const wake = () => post('sleep?wake=true', 'The camera was asked to wake up.')

let timer: number | undefined
onMounted(() => {
  read()
  timer = window.setInterval(read, 60000)
})
onBeforeUnmount(() => window.clearInterval(timer))
watch(() => [live.camera, live.settings.shown], () => {
  view.value = null
  note.value = ''
  read()
})
</script>

<template>
  <div v-if="view && (view.low_lux.supported || view.privacy.value || view.sleep.value?.sleeping)" class="image-live" data-testid="image-live">
    <button
      v-if="view.low_lux.supported"
      id="low-lux"
      data-testid="ctl-low-lux"
      class="ctl"
      type="button"
      :aria-pressed="Boolean(view.low_lux.value)"
      :aria-busy="busy"
      @click="toggleLowLux"
    >
      <span>Low light</span>
    </button>
    <p v-if="view.privacy.supported && view.privacy.value" class="hint warn-text" data-testid="image-live-privacy" role="status">
      Privacy shading is on, so the camera sends no picture.
      <button class="btn" type="button" :aria-busy="busy" @click="uncover">Uncover the lens</button>
    </p>
    <p v-if="view.sleep.supported && view.sleep.value?.sleeping" class="hint warn-text" data-testid="image-live-sleep" role="status">
      The camera is inside its timed sleep window, so it sends no picture.
      <button class="btn" type="button" :aria-busy="busy" @click="wake">Wake it</button>
    </p>
    <p v-if="note" class="hint" role="status">{{ note }}</p>
  </div>
</template>
