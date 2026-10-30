<script setup lang="ts">
import { computed, onMounted, reactive, ref, shallowRef, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { IconButton } from '../components/ui/icon-button'
import { SegmentedControl } from '../components/ui/segmented-control'
import { api } from '../lib/api'
import { hm } from '../app/format'
import { sessionLost } from '../app/shell'
import { state as page } from '../app/state'
import { loadDevices, openCamera, toggleFullscreen } from '../app/view'
import GridTile from './GridTile.vue'
import { runGridClock } from './grid-clock'
import { live } from './store'

type Quality = 'main' | 'sub'
type View = { device: string; channel: number; quality: Quality }
type Spec = { layout: string; tiles: View[] }
type Camera = { key: string; name: string; channel: number; device: { id: string; online?: boolean } }

const MAX = 9
const LAYOUTS = [
  { value: 'auto', label: 'Auto', columns: 0 },
  { value: '1', label: '1', columns: 1 },
  { value: '2', label: '2', columns: 2 },
  { value: '3', label: '3', columns: 3 },
]
const OPTIONS = ['1', '2', '3', 'auto'].map((value) => {
  const entry = LAYOUTS.find((layout) => layout.value === value)!
  return { value, label: entry.label, testid: 'grid-layout-' + value }
})

const spec = ref<Spec | null>(null)
const cameras = shallowRef<Camera[]>([])
const shown = ref<View[]>([])
const states = reactive<Record<string, string>>({})
const save = reactive({ busy: false, queued: false, at: 0, error: '' })
const run = ref(0)
const wide = ref(false)
const medium = ref(false)
const short = ref(false)
let token = 0
let loading: Promise<Spec | null> | null = null

const keyOf = (view: View) => view.device + ':' + view.channel
const layoutOf = (value?: string) => LAYOUTS.find((entry) => entry.value === value) || LAYOUTS[0]
const cameraOf = (view: View) => cameras.value.find((camera) => camera.key === keyOf(view)) as Camera

const layout = computed(() => layoutOf(spec.value?.layout))
const columns = computed(() => {
  const count = Math.max(1, shown.value.length)
  if (!medium.value) return 1
  return layout.value.columns || Math.min(count, wide.value ? 4 : 2)
})
const tilesStyle = computed(() => ({
  gridTemplateColumns: 'repeat(' + columns.value + ', minmax(0, 1fr))',
  gridTemplateRows: medium.value && !short.value ? 'repeat(' + Math.ceil(Math.max(1, shown.value.length) / columns.value) + ', minmax(0, 1fr))' : undefined,
}))

const note = computed(() => {
  if (save.error) return save.error
  if (save.busy) return 'Saving the layout…'
  return save.at ? 'Layout and streams saved at ' + hm(save.at) : 'Layout and streams are kept on this server'
})

const count = computed(() => {
  const tally = (name: string) => shown.value.filter((view) => states[keyOf(view)] === name).length
  const total = cameras.value.length
  const parts = [total + ' camera' + (total === 1 ? '' : 's')]
  for (const [name, word] of [['playing', 'live'], ['loading', 'connecting'], ['stalled', 'reconnecting'], ['error', 'no picture'], ['offline', 'offline']]) {
    if (tally(name)) parts.push(tally(name) + ' ' + word)
  }
  if (total > MAX) parts.push('showing the first ' + MAX)
  return parts.join(' · ')
})

function specFrom(payload: any): Spec {
  const tiles: View[] = []
  const seen = new Set<string>()
  for (const tile of Array.isArray(payload?.tiles) ? payload.tiles : []) {
    const device = typeof tile.device === 'string' ? tile.device.trim() : ''
    const channel = Number(tile.channel) || 0
    if (!device || channel < 1 || seen.has(device + ':' + channel)) continue
    seen.add(device + ':' + channel)
    tiles.push({ device, channel, quality: tile.quality === 'sub' ? 'sub' : 'main' })
    if (tiles.length >= MAX) break
  }
  return { layout: layoutOf(payload?.layout).value, tiles }
}

function reconcile(stored: Spec): Spec {
  const known = new Set(cameras.value.map((camera) => camera.key))
  const seen = new Set<string>()
  const tiles: View[] = []
  for (const tile of stored.tiles) {
    if (!known.has(keyOf(tile)) || seen.has(keyOf(tile))) continue
    seen.add(keyOf(tile))
    tiles.push({ ...tile })
  }
  for (const camera of cameras.value) {
    if (tiles.length >= MAX) break
    if (seen.has(camera.key)) continue
    seen.add(camera.key)
    tiles.push({ device: camera.device.id, channel: camera.channel, quality: tiles.length === 0 && page.prefs.quality === 'sub' ? 'sub' : tiles.length === 0 ? 'main' : 'sub' })
  }
  return { layout: stored.layout, tiles }
}

function readSpec() {
  if (!loading) {
    loading = api<any>('/api/grid')
      .then((payload) => {
        save.error = ''
        return specFrom(payload)
      })
      .catch((error) => {
        if (sessionLost(error)) return specFrom(null)
        save.error = 'Could not read the saved grid: ' + (error instanceof Error ? error.message : String(error))
        return null
      })
      .finally(() => (loading = null))
  }
  return loading
}

async function persist() {
  if (!spec.value) return
  if (save.busy) {
    save.queued = true
    return
  }
  save.busy = true
  save.queued = false
  try {
    await api('/api/grid', { method: 'POST', body: JSON.stringify({ layout: spec.value.layout, tiles: spec.value.tiles }) })
    save.error = ''
    save.at = Date.now()
  } catch (error) {
    if (!sessionLost(error)) save.error = 'The grid was not saved: ' + (error instanceof Error ? error.message : String(error))
  } finally {
    save.busy = false
  }
  if (save.queued) persist()
}

async function start() {
  const mine = ++token
  cameras.value = page.cameras.slice()
  const stored = spec.value ? null : await readSpec()
  if (mine !== token || !live.grid.open) return
  spec.value = reconcile(spec.value || stored || specFrom(null))
  shown.value = spec.value.tiles.filter((view) => cameras.value.some((camera) => camera.key === keyOf(view)))
  run.value += 1
}

function setLayout(value: string) {
  if (!spec.value) return
  spec.value.layout = layoutOf(value).value
  persist()
}

function select(view: View) {
  live.grid.selected = keyOf(view)
}

function toggle(view: View) {
  view.quality = view.quality === 'main' ? 'sub' : 'main'
  persist()
}

function moveFocus(key: string) {
  const buttons = [...document.querySelectorAll<HTMLButtonElement>('#tiles .tile-open')]
  if (!buttons.length) return
  const index = Math.max(0, buttons.indexOf(document.activeElement as HTMLButtonElement))
  const step = ({ ArrowLeft: -1, ArrowRight: 1, ArrowUp: -columns.value, ArrowDown: columns.value } as Record<string, number>)[key] || 0
  buttons[Math.min(buttons.length - 1, Math.max(0, index + step))].focus()
}

watch(() => [live.grid.open, live.grid.generation], () => {
  runGridClock(live.grid.open)
  if (live.grid.open) start()
  else token += 1
})

onMounted(() => {
  for (const [query, flag] of [['(min-width: 1024px)', wide], ['(min-width: 641px)', medium], ['(max-height: 600px)', short]] as const) {
    const media = window.matchMedia(query)
    flag.value = media.matches
    media.addEventListener('change', () => (flag.value = media.matches))
  }
  live.grid.moveFocus = moveFocus
})
</script>

<template>
  <div id="grid-bar" class="grid-bar">
    <div class="grid-title">
      <h2 id="grid-heading">All cameras</h2>
      <p id="grid-count" class="grid-count" data-testid="grid-count">{{ count }}</p>
      <p id="grid-note" class="grid-note" data-testid="grid-note" :data-state="save.error ? 'error' : 'ok'" :role="save.error ? 'alert' : 'status'">
        <Icon v-if="save.error" name="warning" small /><span>{{ note }}</span>
      </p>
    </div>
    <div class="grid-tools">
      <div v-if="medium" class="grid-layout">
        <span id="grid-layout-label" class="grid-label">Columns</span>
        <SegmentedControl
          data-testid="grid-layout"
          size="sm"
          label="Grid columns"
          :options="OPTIONS"
          :model-value="layout.value"
          :disabled="!spec"
          @update:model-value="setLayout"
        />
      </div>
      <Button
        id="grid-sound"
        variant="ghost"
        size="sm"
        data-testid="grid-sound"
        aria-keyshortcuts="M"
        :icon="live.muted ? 'speaker-off' : 'speaker'"
        :aria-pressed="live.muted ? 'false' : 'true'"
        :title="live.muted ? 'Sound is off (M)' : 'Sound follows the selected tile (M)'"
        @click="live.sound === 'blocked' ? null : live.actions.toggleMute()"
      >{{ live.muted ? 'Sound off' : 'Sound on' }}</Button>
      <IconButton id="grid-fullscreen" icon="expand" label="Full screen" shortcut="F" side="bottom" @click="toggleFullscreen" />
    </div>
  </div>
  <p id="grid-open-hint" class="sr-only">Click selects the tile and its sound. Enter or double-click opens the camera in Live.</p>
  <div id="tiles" class="tiles" data-testid="grid-tiles" :data-count="shown.length" :data-layout="layout.value" :style="tilesStyle" aria-labelledby="grid-heading" role="group">
    <template v-if="live.grid.open">
      <GridTile
        v-for="(view, index) in shown"
        :key="keyOf(view)"
        :view="view"
        :camera="cameraOf(view)"
        :index="index"
        :selected="keyOf(view) === live.grid.selected"
        :run="run"
        @state="(name) => (states[keyOf(view)] = name)"
        @toggle="toggle(view)"
        @select="select(view)"
        @refresh="loadDevices()"
        @open="openCamera(cameras.indexOf(cameraOf(view)))"
      />
    </template>
  </div>
</template>
