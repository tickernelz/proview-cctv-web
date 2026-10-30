<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { StreamPlayer, type SoundState } from '../lib/stream-player'
import { deviceStatus, statusLine } from '../app/devices'
import { sessionLost } from '../app/shell'
import { shortStreamError } from '../app/stage'
import { CameraOffline, ErrorWithDetails, RelaySlow, StageButton, StageChip, StageLoading, clockHms } from '../states'
import { gridNow } from './grid-clock'
import { live } from './store'

type View = { device: string; channel: number; quality: 'main' | 'sub' }
type Camera = { key: string; name: string; channel: number; device: { id: string; online?: boolean; online_status?: string; online_detail?: string } }
type TileState = 'loading' | 'playing' | 'stalled' | 'error' | 'offline'

const props = defineProps<{ view: View; camera: Camera; index: number; selected: boolean; run: number }>()
const emit = defineEmits<{ state: [name: TileState]; toggle: []; open: []; select: []; refresh: [] }>()

const WORDS: Record<TileState, string> = { loading: 'connecting', playing: 'live', stalled: 'reconnecting', error: 'no picture', offline: 'offline' }
const SOUND_WORDS: Partial<Record<SoundState, string>> = {
  playing: 'Sound on',
  waiting: 'Sound on',
  silent: 'No sound from the camera',
  blocked: 'Click for sound',
  unplayable: 'Sound not playable',
  unsupported: 'No Web Audio',
}

const video = ref<HTMLVideoElement>()
const status = reactive({ name: 'loading' as TileState, detail: '', title: '', at: 0, frameAt: 0, sound: 'off' as SoundState })
let player: StreamPlayer | null = null
let nonce = 0

const key = computed(() => props.view.device + ':' + props.view.channel)
const hd = computed(() => props.view.quality === 'main')
const shown = computed(() => status.name === 'playing' || status.name === 'stalled')
const clock = computed(() => (status.name === 'playing' ? clockHms(gridNow.value) : ''))
const soundWord = computed(() => {
  if (!props.selected) return ''
  if (live.muted) return 'Sound off'
  return SOUND_WORDS[status.sound] || 'Sound on'
})
const label = computed(() => props.camera.name + ', ' + WORDS[status.name] + (props.selected ? ', selected' : ''))
const deviceToken = computed(() => new RegExp('(?<![A-Za-z0-9])' + props.view.device.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '(?![A-Za-z0-9])', 'g'))
const scrub = (text: string) => (text ? text.replace(deviceToken.value, props.camera.name) : text)
const details = computed(() => (status.title && status.title !== status.detail ? scrub(status.title) : ''))

function set(name: TileState, detail = '') {
  if (name === 'playing') status.frameAt = Date.now()
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

function frame() {
  if (status.name === 'playing') status.frameAt = Date.now()
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Enter') return
  event.preventDefault()
  emit('open')
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
    <div class="tile-well" :data-state="status.name">
      <video ref="video" data-testid="grid-tile-video" playsinline :hidden="!shown" @timeupdate="frame" />
      <button
        class="tile-open"
        type="button"
        :aria-label="label"
        :aria-pressed="selected"
        aria-keyshortcuts="Enter"
        aria-describedby="grid-open-hint"
        @click="emit('select')"
        @dblclick="emit('open')"
        @keydown="onKey"
      />
      <div v-if="status.name === 'playing'" class="tile-plate"><StageChip tone="live" label="LIVE" /></div>
      <StageLoading v-if="status.name === 'loading'" data-testid="grid-tile-loading" label="Connecting…" />
      <RelaySlow v-else-if="status.name === 'stalled'" data-testid="grid-tile-stalled" :last-frame-at="status.frameAt || null" />
      <CameraOffline
        v-else-if="status.name === 'offline'"
        class="tile-fail"
        data-testid="grid-tile-fail"
        compact
        :name="camera.name"
        :detail="statusLine(camera.device)"
        @retry="emit('refresh')"
      />
      <ErrorWithDetails
        v-else-if="status.name === 'error'"
        class="tile-fail"
        data-testid="grid-tile-fail"
        compact
        title="No picture"
        :description="scrub(status.detail) || 'The stream stopped.'"
        :description-title="scrub(status.title)"
        :details="details || null"
        @retry="start"
      />
      <div class="tile-actions">
        <StageButton icon="frame" :aria-label="'Open ' + camera.name + ' in Live'" data-testid="grid-tile-open" @click="emit('open')">Open</StageButton>
      </div>
    </div>
    <div class="tile-meta">
      <p class="tile-name"><strong>{{ camera.name }}</strong></p>
      <span class="tile-state" data-testid="grid-tile-state" :data-sound="status.sound" :data-selected="selected">
        <Icon v-if="soundWord" :name="live.muted || status.sound === 'silent' ? 'speaker-off' : 'speaker'" small />
        <span v-if="soundWord">{{ soundWord }}</span>
        <span v-else class="sr-only">{{ WORDS[status.name] }}</span>
      </span>
      <span class="tile-clock" data-testid="grid-tile-clock">{{ clock }}</span>
      <button
        class="tile-pick"
        type="button"
        data-testid="grid-tile-quality"
        :aria-label="'This tile plays the ' + (hd ? 'main stream' : 'sub stream') + '. Switch it.'"
        @click="emit('toggle')"
      >{{ hd ? 'HD' : 'SD' }}</button>
    </div>
  </article>
</template>
