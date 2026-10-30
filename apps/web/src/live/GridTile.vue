<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { api } from '../lib/api'
import { StreamPlayer, type SoundState } from '../lib/stream-player'
import { deviceStatus, statusLine } from '../app/devices'
import { hms } from '../app/format'
import { sessionLost } from '../app/shell'
import { shortStreamError } from '../app/stage'
import { gridNow } from './grid-clock'
import { live } from './store'

type View = { device: string; channel: number; quality: 'main' | 'sub' }
type Camera = { key: string; name: string; channel: number; device: { id: string; online?: boolean } }
type TileState = 'loading' | 'playing' | 'stalled' | 'error' | 'offline'

const props = defineProps<{ view: View; camera: Camera; index: number; selected: boolean; run: number }>()
const emit = defineEmits<{ state: [name: TileState]; toggle: []; open: [] }>()

const WORDS: Record<TileState, string> = { loading: 'connecting', playing: 'live', stalled: 'frozen', error: 'failed', offline: 'offline' }

const video = ref<HTMLVideoElement>()
const status = reactive({ name: 'loading' as TileState, detail: '', title: '', at: 0, sound: 'off' as SoundState })
const SOUND_WORDS: Partial<Record<SoundState, string>> = {
  playing: ' · sound',
  silent: ' · no sound',
  blocked: ' · click for sound',
  unplayable: ' · sound not playable',
  unsupported: ' · no Web Audio',
}
let player: StreamPlayer | null = null
let nonce = 0

const key = computed(() => props.view.device + ':' + props.view.channel)
const hd = computed(() => props.view.quality === 'main')
const clock = computed(() => (status.name === 'playing' ? hms(gridNow.value) : status.name === 'error' ? hms(status.at) : ''))

function set(name: TileState, detail = '') {
  status.name = name
  status.detail = detail
  status.at = Date.now()
  emit('state', name)
}

function stop() {
  nonce += 1
  if (player) player.destroy()
  player = null
}

async function start() {
  stop()
  const mine = nonce
  status.title = ''
  if (deviceStatus(props.camera.device).known && !props.camera.device.online) return set('offline')
  if (!video.value) return
  const own = new StreamPlayer(video.value, (name, message) => player === own && set(name, message), () => live.muted || !props.selected)
  own.onSound = (sound) => {
    if (player !== own) return
    status.sound = sound
    if (props.selected) live.sound = sound
  }
  player = own
  set('loading')
  try {
    const ticket = await api('/api/stream?device=' + encodeURIComponent(props.view.device) + '&channel=' + props.view.channel + '&quality=' + encodeURIComponent(props.view.quality))
    if (mine !== nonce || player !== own) return own.destroy()
    await own.play(ticket)
  } catch (error) {
    if (mine !== nonce || sessionLost(error)) return
    set('error', shortStreamError(error))
    status.title = error instanceof Error ? error.message : String(error)
  }
}

watch(() => [props.run, props.view.quality], start)
watch(
  () => props.selected,
  (selected) => {
    if (selected) live.sound = status.sound
  },
)
onMounted(() => {
  if (video.value) video.value.muted = true
  start()
})
onBeforeUnmount(stop)
</script>

<template>
  <article class="tile" :class="{ sel: selected }" data-testid="grid-tile" :data-key="key" :data-state="status.name" :data-index="index">
    <button class="tile-open" type="button" :aria-label="'Open ' + camera.name + ' in Live'" @click="emit('open')" />
    <div class="well" :data-state="status.name">
      <video ref="video" data-testid="grid-tile-video" playsinline :hidden="status.name !== 'playing' && status.name !== 'stalled'" />
      <div v-show="status.name === 'loading'" class="prog" />
      <p v-if="status.name === 'loading'" class="tile-msg" data-testid="grid-tile-loading">Connecting…</p>
      <div v-if="status.name === 'error' || status.name === 'offline'" class="tile-fail" data-testid="grid-tile-fail">
        <strong>{{ status.name === 'offline' ? 'Offline' : 'No picture' }}</strong>
        <span :title="status.title || null">{{ status.name === 'offline' ? statusLine(camera.device) + '.' : status.detail || 'The stream stopped.' }}</span>
        <button v-if="status.name === 'error'" class="btn btn-line inv" type="button" data-testid="grid-tile-retry" @click="start">Try again</button>
      </div>
    </div>
    <div class="tile-meta">
      <p><strong>{{ camera.name }}</strong><span class="tile-state" data-testid="grid-tile-state" :data-sound="status.sound">{{ WORDS[status.name] + (SOUND_WORDS[status.sound] || '') }}</span></p>
      <button
        class="tile-pick"
        type="button"
        data-testid="grid-tile-quality"
        title="Main stream or the lighter sub stream, for this tile"
        :aria-label="'This tile plays the ' + (hd ? 'main stream' : 'sub stream') + '. Switch it.'"
        @click="emit('toggle')"
      >{{ hd ? 'HD' : 'SD' }}</button>
      <span class="right" :class="{ warn: status.name === 'error' || status.name === 'stalled' }" data-testid="grid-tile-clock">{{ clock }}</span>
    </div>
  </article>
</template>
