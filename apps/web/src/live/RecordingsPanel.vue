<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { sessionLost } from '../app/shell'
import { state as page } from '../app/state'
import { addLauncher, live } from './store'

type Camera = { id: string; name?: string }
type Recording = { start: string; duration_secs?: number; size_bytes?: number }
type Listing = { recordings?: Recording[]; notice?: string }

const open = computed(() => live.sheet === 'recordings')
const panel = ref<HTMLElement>()
const dayInput = ref<HTMLInputElement>()
const cameras = ref<Camera[]>([])
const form = reactive({ day: localDay(), device: '' })
const result = reactive({
  phase: 'idle' as 'idle' | 'loading' | 'done' | 'error',
  message: '',
  recordings: [] as Recording[],
})

const launcher = addLauncher({
  id: 'recordings-launcher',
  label: 'Cloud',
  icon: 'cloud',
  slot: 'start',
  pressed: false,
  controls: 'recordings-panel',
  onClick: () => (open.value ? close() : show()),
})

watch(open, (shown) => (launcher.pressed = shown))

function localDay() {
  const now = new Date()
  return now.getFullYear() + '-' + String(now.getMonth() + 1).padStart(2, '0') + '-' + String(now.getDate()).padStart(2, '0')
}

async function show() {
  live.sheet = 'recordings'
  await nextTick()
  dayInput.value?.focus()
  cameras.value = page.devices as Camera[]
  const wanted = cameras.value.find((camera) => camera.id === page.selectedId) || cameras.value[0]
  if (!cameras.value.some((camera) => camera.id === form.device)) form.device = wanted ? wanted.id : ''
}

function close(returnFocus = true) {
  if (live.sheet === 'recordings') live.sheet = ''
  if (returnFocus) document.getElementById('recordings-launcher')?.focus()
}

function fail(error: unknown) {
  result.phase = 'error'
  result.recordings = []
  const text = error instanceof Error ? error.message : String(error)
  result.message = text.charAt(0).toUpperCase() + text.slice(1)
}

async function load() {
  if (result.phase === 'loading') return
  result.phase = 'loading'
  result.recordings = []
  const day = form.day
  const camera = cameras.value.find((entry) => entry.id === form.device)?.name || form.device
  const query = new URLSearchParams({ day, channel: '0' })
  if (form.device) query.set('device', form.device)
  try {
    const listing = await api<Listing>('/api/recordings?' + query.toString())
    result.recordings = listing.recordings || []
    const count = result.recordings.length
    result.message = count
      ? [count + (count === 1 ? ' clip' : ' clips'), camera, day].filter(Boolean).join(' · ')
      : listing.notice || 'No cloud clips for ' + camera + ' on ' + day + '.'
    result.phase = 'done'
  } catch (error) {
    if (sessionLost(error)) return close(false)
    fail(error)
  }
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  event.preventDefault()
  close()
}

function size(bytes?: number) {
  if (!bytes) return '—'
  if (bytes >= 1048576) return (bytes / 1048576).toFixed(1) + ' MB'
  if (bytes >= 1024) return Math.round(bytes / 1024) + ' KB'
  return bytes + ' B'
}

function duration(seconds?: number) {
  if (!seconds) return '—'
  return Math.floor(seconds / 60) + 'm ' + String(seconds % 60).padStart(2, '0') + 's'
}
</script>

<template>
  <section
    id="recordings-panel"
    ref="panel"
    data-testid="recordings-panel"
    role="dialog"
    aria-labelledby="recordings-title"
    :hidden="!open"
    @keydown="onKey"
  >
    <header>
      <strong id="recordings-title">Cloud recordings</strong>
      <button class="iconbtn" type="button" data-testid="recordings-close" aria-label="Close cloud recordings" @click="close()">
        <Icon name="x" />
      </button>
    </header>
    <form @submit.prevent="load">
      <label>Day <input ref="dayInput" v-model="form.day" type="date" name="day" data-testid="recordings-day" required></label>
      <label>Camera
        <select v-model="form.device" name="device" data-testid="recordings-camera" :disabled="!cameras.length">
          <option v-for="camera in cameras" :key="camera.id" :value="camera.id">{{ camera.name || camera.id }}</option>
        </select>
      </label>
      <button class="btn btn-fill" type="submit" data-testid="recordings-load" :aria-busy="result.phase === 'loading'">
        {{ result.phase === 'loading' ? 'Loading…' : 'Load' }}
      </button>
    </form>
    <p v-if="result.phase === 'error'" class="err" data-status data-testid="recordings-error" role="alert">
      <Icon name="warning" small /><span>{{ result.message }}</span>
    </p>
    <p v-else data-status data-testid="recordings-status" role="status">
      {{ result.phase === 'idle' ? 'Choose a day and load the cloud list.' : result.phase === 'loading' ? 'Loading the cloud list…' : result.message }}
    </p>
    <ul v-if="result.recordings.length" data-testid="recordings-list">
      <li v-for="item in result.recordings" :key="item.start" data-testid="recordings-item">
        <span>{{ item.start }}</span><span>{{ duration(item.duration_secs) }} · {{ size(item.size_bytes) }}</span>
      </li>
    </ul>
  </section>
</template>
