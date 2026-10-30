<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { state } from '../app/state'
import { live } from './store'

const shown = ref(false)
const busy = ref(false)
const note = ref('')
const bad = ref(false)

const query = () => (state.selectedId ? '?device=' + encodeURIComponent(state.selectedId) : '')

let ticket = 0

async function check() {
  const mine = ++ticket
  shown.value = false
  note.value = ''
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
  bad.value = false
  note.value = 'Sending stop…'
  try {
    const response = await fetch('/api/device/detect/alarm/stop' + query(), { method: 'POST', credentials: 'same-origin' })
    bad.value = !response.ok
    note.value = response.ok ? 'The camera acknowledged the stop.' : 'Stop failed: HTTP ' + response.status
  } catch (error) {
    bad.value = true
    note.value = 'Stop failed: ' + (error instanceof Error ? error.message : String(error))
  } finally {
    busy.value = false
  }
}

watch(() => live.camera, check)
onMounted(check)
</script>

<template>
  <template v-if="shown">
    <button id="alarm-stop" data-testid="ctl-alarm-stop" class="ctl" type="button" :aria-busy="busy" aria-describedby="alarm-stop-note" @click="stop">
      <Icon name="bell" /><span>Stop alarm</span>
    </button>
    <span v-if="note" id="alarm-stop-note" data-testid="ctl-alarm-stop-note" :class="['stop-note', { bad }]" role="status">{{ note }}</span>
  </template>
</template>

<style scoped>
.stop-note { align-self: center; font-size: var(--t-13); color: var(--ink-2); }
.stop-note.bad { color: var(--bad); }
</style>
