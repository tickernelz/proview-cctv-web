<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { state } from '../app/state'
import { live } from './store'
import { alarmStopNote as note } from './stage/notes'

const shown = ref(false)
const busy = ref(false)

const query = () => (state.selectedId ? '?device=' + encodeURIComponent(state.selectedId) : '')

let ticket = 0

async function check() {
  const mine = ++ticket
  shown.value = false
  note.text = ''
  note.bad = false
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
  note.bad = false
  note.text = 'Sending stop…'
  try {
    const response = await fetch('/api/device/detect/alarm/stop' + query(), { method: 'POST', credentials: 'same-origin' })
    note.bad = !response.ok
    note.text = response.ok ? 'The camera acknowledged the stop.' : 'Stop failed: HTTP ' + response.status
  } catch (error) {
    note.bad = true
    note.text = 'Stop failed: ' + (error instanceof Error ? error.message : String(error))
  } finally {
    busy.value = false
  }
}

watch(() => live.camera, check)
onMounted(check)
</script>

<template>
  <button v-if="shown" id="alarm-stop" data-testid="ctl-alarm-stop" class="dk alarm" type="button" :aria-busy="busy" :aria-describedby="note.text ? 'alarm-stop-note' : undefined" @click="stop">
    <Icon name="bell" /><span class="dk-label">Stop alarm</span>
  </button>
</template>
