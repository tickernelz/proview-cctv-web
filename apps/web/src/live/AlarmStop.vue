<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { state } from '../app/state'
import { live } from './store'

const shown = ref(false)
const busy = ref(false)
const note = ref('Stop the sounding alarm')

const query = () => (state.selectedId ? '?device=' + encodeURIComponent(state.selectedId) : '')

let ticket = 0

async function check() {
  const mine = ++ticket
  shown.value = false
  if (!state.selectedId) return
  try {
    const response = await fetch('/api/device/detect/alarm/stop' + query(), { credentials: 'same-origin' })
    const body = response.ok ? await response.json() : null
    if (mine === ticket) shown.value = Boolean(body && body.gate && body.gate.supported)
  } catch {
    if (mine === ticket) shown.value = false
  }
}

async function stop() {
  if (busy.value) return
  busy.value = true
  try {
    const response = await fetch('/api/device/detect/alarm/stop' + query(), { method: 'POST', credentials: 'same-origin' })
    note.value = response.ok ? 'The camera acknowledged the stop' : 'Stop failed: HTTP ' + response.status
  } finally {
    busy.value = false
  }
}

watch(() => live.camera, check)
onMounted(check)
</script>

<template>
  <button v-if="shown" id="alarm-stop" data-testid="ctl-alarm-stop" class="ctl" type="button" :title="note" :aria-busy="busy" @click="stop">
    <Icon name="bell" /><span>Stop alarm</span>
  </button>
</template>
