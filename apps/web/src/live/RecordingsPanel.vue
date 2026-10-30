<script setup lang="ts">
import { computed, nextTick, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { IconButton } from '../components/ui/icon-button'
import { SegmentedControl } from '../components/ui/segmented-control'
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from '../components/ui/sheet'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs'
import { api } from '../lib/api'
import { sessionLost } from '../app/shell'
import { state as page } from '../app/state'
import { shell } from '../shell/layout'
import PlaybackRows from './PlaybackRows.vue'
import { mb, stamp, type Clip } from './playback-clips'
import { live } from './store'
import { downloadFile, loadFiles, needThumbs, playAt, setDay, setKind, shiftDay, takeOpener, tl, today, type Kind } from './timeline/engine'
import { dayLabel } from './timeline/time'

type Camera = { id: string; name?: string }
type Recording = { start: string; duration_secs?: number; size_bytes?: number; index_file?: string; storage_bucket?: string }
type Listing = { recordings?: Recording[]; notice?: string }
type PlanState = { plan: 'open' | 'none' | 'expired' | 'unsupported' | 'unknown'; reason: string }

const KINDS = [
  { value: 'all', label: 'All', testid: 'pb-kind-all' },
  { value: 'regular', label: 'Regular', testid: 'pb-kind-regular' },
  { value: 'alarm', label: 'Alarm', testid: 'pb-kind-alarm' },
]
const NO_PLAN = ['none', 'expired', 'unsupported']

const open = computed(() => live.sheet === 'recordings')
const dayInput = ref<HTMLInputElement>()
const cameraTab = ref<HTMLElement>()
const cameras = ref<Camera[]>([])
const form = reactive({ day: today(), device: '' })
const result = reactive({
  phase: 'idle' as 'idle' | 'loading' | 'done' | 'error',
  message: '',
  recordings: [] as Recording[],
  device: '',
  plan: '' as PlanState['plan'] | '',
})
const clip = reactive({
  phase: 'idle' as 'idle' | 'loading' | 'playing' | 'error',
  start: '',
  message: '',
  url: '',
})

const cameraName = (id: string) => (page.devices.find((device) => device.id === id)?.name || id) as string
const day = computed({
  get: () => tl.day,
  set: (value: string) => setDay(value || today()),
})
const files = tl.files
const totalBytes = computed(() => files.list.reduce((total, file) => total + file.bytes, 0))
const filterName = computed(() => (files.asked.kind === 'regular' ? ' (regular only)' : files.asked.kind === 'alarm' ? ' (alarm only)' : ''))
const summary = computed(() => {
  const where = cameraName(files.asked.device) + ' on ' + files.asked.day
  if (files.phase === 'loading') return 'Loading the recordings of ' + where + '…'
  if (files.phase !== 'ok') return ''
  if (!files.list.length) return 'No recordings of ' + where + filterName.value + '.'
  const cap = files.cap ? ' · the device list is capped at ' + files.cap : ''
  return files.list.length + ' files · ' + mb(totalBytes.value) + ' · ' + where + filterName.value + cap
})

watch(open, async (shown) => {
  if (!shown) {
    stopClip()
    return
  }
  cameras.value = page.devices as Camera[]
  const wanted = cameras.value.find((camera) => camera.id === page.selectedId) || cameras.value[0]
  if (!cameras.value.some((camera) => camera.id === form.device)) form.device = wanted ? wanted.id : ''
  if (tl.tab === 'camera') void loadFiles()
})

watch(
  () => live.settings.signedIn,
  (signed) => {
    if (!signed && open.value) live.sheet = ''
  },
)

function setOpen(value: boolean) {
  if (value) returnTo = 'opener'
  if (!value && live.sheet === 'recordings') live.sheet = ''
}

let returnTo: 'opener' | 'none' = 'opener'

function close(focus = true) {
  returnTo = focus ? 'opener' : 'none'
  setOpen(false)
}

async function focusFirst(event: Event) {
  event.preventDefault()
  await nextTick()
  if (tl.tab === 'cloud') dayInput.value?.focus()
  else cameraTab.value?.querySelector<HTMLElement>('[data-state="active"]')?.focus()
}

function returnFocus(event: Event) {
  const element = takeOpener()
  if (returnTo === 'none') return event.preventDefault()
  if (!element) return
  event.preventDefault()
  element.focus()
}

function onTab(value: string | number) {
  tl.tab = value === 'cloud' ? 'cloud' : 'camera'
  if (tl.tab === 'camera') void loadFiles()
}

function playFile(file: Clip) {
  const from = stamp(file.begin)
  if (from === null) return
  close(false)
  void playAt(from)
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
  result.plan = ''
  stopClip()
  const asked = form.day
  result.device = form.device
  const camera = cameras.value.find((entry) => entry.id === form.device)?.name || form.device
  const query = new URLSearchParams({ day: asked, channel: '0' })
  if (form.device) query.set('device', form.device)
  const planQuery = new URLSearchParams({ device: form.device, channel: '0', day: asked })
  const planned = form.device ? api<PlanState>('/api/cloud/clips/state?' + planQuery.toString()).catch(() => null) : Promise.resolve(null)
  try {
    const [listing, plan] = await Promise.all([api<Listing>('/api/recordings?' + query.toString()), planned])
    result.recordings = listing.recordings || []
    result.plan = plan ? plan.plan : ''
    const count = result.recordings.length
    const noPlan = plan && NO_PLAN.includes(plan.plan) ? plan.reason : ''
    result.message = count
      ? [count + (count === 1 ? ' clip' : ' clips'), camera, asked].filter(Boolean).join(' · ') + (noPlan ? '. ' + noPlan : '')
      : noPlan || listing.notice || 'No cloud clips for ' + camera + ' on ' + asked + '.'
    result.phase = 'done'
  } catch (error) {
    if (sessionLost(error)) return close(false)
    fail(error)
  }
}

function playable(item: Recording) {
  return Boolean(item.index_file && item.storage_bucket) && !NO_PLAN.includes(result.plan)
}

function clipPath(kind: 'play' | 'download', item: Recording) {
  const query = new URLSearchParams({ device: result.device, object: item.index_file || '', bucket: item.storage_bucket || '' })
  return '/api/cloud/clips/' + kind + '?' + query.toString()
}

function stopClip() {
  if (clip.url) URL.revokeObjectURL(clip.url)
  clip.phase = 'idle'
  clip.url = ''
  clip.start = ''
  clip.message = ''
}

async function fetchClip(kind: 'play' | 'download', item: Recording) {
  const response = await fetch(clipPath(kind, item), { credentials: 'same-origin' })
  if (!response.ok) {
    const body = await response.json().catch(() => null)
    const error: Error & { status?: number } = new Error(body && body.message ? body.message : response.status + ' ' + response.statusText)
    error.status = response.status
    throw error
  }
  return response.blob()
}

async function takeClip(kind: 'play' | 'download', item: Recording) {
  if (clip.phase === 'loading') return
  stopClip()
  clip.phase = 'loading'
  clip.start = item.start
  clip.message = kind === 'play' ? 'Fetching the clip from the cloud…' : 'Downloading the clip from the cloud…'
  try {
    const blob = await fetchClip(kind, item)
    const url = URL.createObjectURL(blob)
    if (kind === 'download') {
      const link = document.createElement('a')
      link.href = url
      link.download = (result.device || 'clip') + '-' + item.start.replace(/[^0-9]/g, '') + '.ts'
      document.body.append(link)
      link.click()
      link.remove()
      setTimeout(() => URL.revokeObjectURL(url), 60_000)
      clip.phase = 'idle'
      clip.message = ''
      clip.start = ''
      return
    }
    clip.url = url
    clip.phase = 'playing'
    clip.message = 'Clip of ' + item.start
  } catch (error) {
    if (sessionLost(error)) return close(false)
    clip.phase = 'error'
    const text = error instanceof Error ? error.message : String(error)
    clip.message = text.charAt(0).toUpperCase() + text.slice(1)
  }
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
  <Sheet :open="open" :modal="false" @update:open="setOpen">
    <SheetContent
      id="recordings-panel"
      :class="['rec-sheet', shell.narrow ? '' : 'top-(--header) bottom-(--dock-clear) rounded-bl-lg border-b']"
      data-testid="recordings-panel"
      :side="shell.narrow ? 'bottom' : 'right'"
      @open-auto-focus="focusFirst"
      @close-auto-focus="returnFocus"
    >
      <SheetHeader>
        <SheetTitle id="recordings-title">Recordings</SheetTitle>
        <SheetDescription>{{ cameraName(tl.device) }}</SheetDescription>
      </SheetHeader>
      <Tabs :model-value="tl.tab" class="rec-tabs" @update:model-value="onTab">
        <div ref="cameraTab">
          <TabsList aria-label="Where the recordings are">
            <TabsTrigger value="camera" data-testid="rec-tab-camera">On the camera</TabsTrigger>
            <TabsTrigger value="cloud" data-testid="rec-tab-cloud">Cloud</TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="camera">
          <div id="pb" class="rec-pane" data-testid="playback">
          <div class="pb-day" role="group" aria-label="Day">
            <IconButton label="Previous day" icon="chevron-left" size="sm" @click="setDay(shiftDay(tl.day, -1))" />
            <input id="pb-day" v-model.lazy="day" class="rec-input" type="date" aria-label="Day" data-testid="pb-day">
            <IconButton label="Next day" icon="chevron-right" size="sm" :disabled="tl.day === today()" @click="setDay(shiftDay(tl.day, 1))" />
            <span class="pb-day-name">{{ dayLabel(tl.day, true) }}</span>
          </div>
          <div class="pb-filter">
            <SegmentedControl data-testid="pb-kinds" :options="KINDS" :model-value="tl.kind" label="Record type" size="sm" @update:model-value="(value) => setKind(value as Kind)" />
            <Button size="sm" variant="ghost" icon="refresh" data-testid="pb-refresh" :loading="files.phase === 'loading'" :disabled="!page.selectedId" @click="loadFiles(true)">Refresh</Button>
          </div>
          <p v-if="!page.devices.length" class="pb-summary" data-testid="pb-empty-account">No cameras on this account yet.</p>
          <p v-if="files.phase === 'error'" class="pb-err" role="alert" data-testid="pb-error"><Icon name="warning" small /><span>{{ files.error }}</span></p>
          <p v-else id="pb-summary" class="pb-summary" data-testid="pb-summary" role="status">{{ summary }}</p>
          <PlaybackRows :files="files.list" :current="live.mode === 'playback' ? tl.file : null" :thumbs="tl.thumbs" @play="playFile" @download="downloadFile" @need="needThumbs" />
          </div>
        </TabsContent>
        <TabsContent value="cloud">
          <div class="rec-pane">
          <form class="rec-form" @submit.prevent="load">
            <label>Day <input ref="dayInput" v-model="form.day" class="rec-input" type="date" name="day" data-testid="recordings-day" required></label>
            <label>Camera
              <select v-model="form.device" class="rec-input" name="device" data-testid="recordings-camera" :disabled="!cameras.length">
                <option v-for="camera in cameras" :key="camera.id" :value="camera.id">{{ camera.name || camera.id }}</option>
              </select>
            </label>
            <Button type="submit" variant="primary" data-testid="recordings-load" :aria-busy="result.phase === 'loading'">
              {{ result.phase === 'loading' ? 'Loading…' : 'Load' }}
            </Button>
          </form>
          <p v-if="result.phase === 'error'" class="pb-err" data-status data-testid="recordings-error" role="alert">
            <Icon name="warning" small /><span>{{ result.message }}</span>
          </p>
          <p v-else class="pb-summary" data-status data-testid="recordings-status" role="status">
            {{ result.phase === 'idle' ? 'Choose a day and load the cloud list.' : result.phase === 'loading' ? 'Loading the cloud list…' : result.message }}
          </p>
          <ul v-if="result.recordings.length" class="rec-list" data-testid="recordings-list">
            <li v-for="item in result.recordings" :key="item.start" class="rec-item" data-testid="recordings-item">
              <span class="rec-start">{{ item.start }}</span><span class="rec-meta">{{ duration(item.duration_secs) }} · {{ size(item.size_bytes) }}</span>
              <div v-if="playable(item)" class="rec-actions">
                <Button size="sm" data-testid="recordings-play" :aria-busy="clip.phase === 'loading' && clip.start === item.start" @click="takeClip('play', item)">Play</Button>
                <Button size="sm" icon="download" data-testid="recordings-download" @click="takeClip('download', item)">Download</Button>
              </div>
            </li>
          </ul>
          <p v-if="clip.phase === 'error'" class="pb-err" data-status data-testid="recordings-clip-error" role="alert">
            <Icon name="warning" small /><span>{{ clip.message }}</span>
          </p>
          <p v-else-if="clip.message" class="pb-summary" data-status data-testid="recordings-clip-status" role="status">{{ clip.message }}</p>
          <video v-if="clip.url" class="rec-player" :src="clip.url" controls autoplay playsinline data-testid="recordings-player" />
          </div>
        </TabsContent>
      </Tabs>
    </SheetContent>
  </Sheet>
</template>
