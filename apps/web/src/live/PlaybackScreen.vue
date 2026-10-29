<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { sessionLost, state as page } from '../legacy/app.js'
import { addLauncher, live } from './store'

type Device = { id: string; name?: string }
type Clip = { name: string; begin: string; end: string; bytes: number }
type Listing = { files?: Clip[]; truncated?: boolean; cap?: number; elapsed_ms?: number }
type Query = { device: string; day: string }
type Frame = { type?: string; message?: string; [key: string]: unknown }

const MEDIA_TYPE = 'video/mp4; codecs="avc1.42E01E"'
const HOURS = [0, 3, 6, 9, 12, 15, 18, 21]

const open = ref(false)
const screen = ref<HTMLElement>()
const title = ref<HTMLElement>()
const timeline = ref<HTMLElement>()
const video = ref<HTMLVideoElement>()
const devices = ref<Device[]>([])
const query = reactive<Query>({ device: '', day: today() })
const list = reactive({
  phase: 'idle' as 'idle' | 'loading' | 'ok' | 'error',
  asked: { device: '', day: '' } as Query,
  files: [] as Clip[],
  cap: 0,
  error: '',
})
const chosen = ref<Clip | null>(null)
const scrub = reactive({ value: 0, max: 0 })
const status = reactive({ text: '', bad: false })
const hover = ref('')

let ticket = 0
const stream = {
  socket: null as WebSocket | null,
  media: null as MediaSource | null,
  buffer: null as SourceBuffer | null,
  queue: [] as Uint8Array[],
  objectUrl: '',
  bytes: 0,
  startedAt: 0,
  frameAt: null as number | null,
  status: {} as Record<string, any>,
  stats: null as Record<string, any> | null,
  windowSec: 60,
}

const launcher = addLauncher({ id: 'pb-open', label: 'Playback', icon: 'history', slot: 'end', controls: 'pb', onClick: () => show() })

const cameraName = (id: string) => devices.value.find((device) => device.id === id)?.name || id
const totalBytes = computed(() => list.files.reduce((total, file) => total + file.bytes, 0))
const summary = computed(() => {
  const where = cameraName(list.asked.device) + ' on ' + list.asked.day
  if (list.phase === 'loading') return 'Loading the recordings of ' + where + '…'
  if (list.phase === 'error') return ''
  if (list.phase !== 'ok') return ''
  if (!list.files.length) return 'No recordings of ' + where + '.'
  const cap = list.cap ? ' · the device list is capped at ' + list.cap : ''
  return list.files.length + ' files · ' + mb(totalBytes.value) + ' · ' + where + cap
})

function today() {
  const now = new Date()
  return now.getFullYear() + '-' + String(now.getMonth() + 1).padStart(2, '0') + '-' + String(now.getDate()).padStart(2, '0')
}

function mb(bytes: number) {
  return (bytes / 1048576).toFixed(2) + ' MB'
}

function clock(seconds: number) {
  const whole = Math.max(0, Math.round(seconds))
  return String(Math.floor(whole / 60)).padStart(2, '0') + ':' + String(whole % 60).padStart(2, '0')
}

function stamp(text: string) {
  const value = Date.parse(String(text).replace(' ', 'T'))
  return Number.isFinite(value) ? value : null
}

function secondsBetween(begin: string, end: string) {
  const a = stamp(begin)
  const b = stamp(end)
  if (a === null || b === null || b <= a) return 0
  return Math.round((b - a) / 1000)
}

const modeOf = (file: Clip) => (/\[M\]/.test(file.name) ? 'motion' : 'continuous')
const span = (file: Clip) => file.begin.slice(11) + ' – ' + file.end.slice(11)

const segments = computed(() => {
  const base = stamp(list.asked.day + ' 00:00:00')
  if (base === null) return []
  const drawn: { key: string; left: number; width: number; mode: string; current: boolean }[] = []
  for (const file of list.files) {
    const from = stamp(file.begin)
    const to = stamp(file.end)
    if (from === null || to === null) continue
    const left = ((from - base) / 86400000) * 100
    if (left < 0 || left > 100) continue
    const width = Math.max(((to - from) / 86400000) * 100, 0.06)
    drawn.push({ key: file.name + file.begin, left, width: Math.min(width, 100 - left), mode: modeOf(file), current: chosen.value === file })
  }
  return drawn
})

function fileAt(clientX: number) {
  const track = timeline.value
  if (!track || !list.files.length) return null
  const box = track.getBoundingClientRect()
  if (!box.width) return null
  const base = stamp(list.asked.day + ' 00:00:00')
  if (base === null) return null
  const ratio = Math.min(Math.max((clientX - box.left) / box.width, 0), 1)
  const at = base + ratio * 86400000
  let next: Clip | null = null
  let prior: Clip | null = null
  for (const file of list.files) {
    const from = stamp(file.begin)
    const to = stamp(file.end)
    if (from === null || to === null) continue
    if (at >= from && at <= to) return { file, from, at }
    if (from > at && (next === null || from < (stamp(next.begin) as number))) next = file
    if (to < at && (prior === null || to > (stamp(prior.end) as number))) prior = file
  }
  const near = next || prior
  if (!near) return null
  const from = stamp(near.begin)
  return from === null ? null : { file: near, from, at }
}

function onTrackClick(event: MouseEvent) {
  const hit = fileAt(event.clientX)
  if (!hit) return
  const duration = secondsBetween(hit.file.begin, hit.file.end)
  play(hit.file, Math.min(Math.max(0, Math.round((hit.at - hit.from) / 1000)), Math.max(duration - 1, 0)))
}

function onTrackMove(event: MouseEvent) {
  const hit = fileAt(event.clientX)
  hover.value = hit ? span(hit.file) + ' · ' + mb(hit.file.bytes) + ' · ' + modeOf(hit.file) : ''
}

function say(text: string, bad = false) {
  status.text = text ? text.charAt(0).toUpperCase() + text.slice(1) : ''
  status.bad = bad
}

function describeStats() {
  const parts: string[] = []
  const s = stream.status
  if (s.openMs !== undefined) parts.push('tunnel ' + (s.openMs / 1000).toFixed(2) + 's')
  if (s.primeMs !== undefined) parts.push('prime ' + (s.primeMs / 1000).toFixed(2) + 's')
  if (s.fps) parts.push(s.fps + ' fps' + (s.fpsSource && s.fpsSource !== 'settled' ? ' (' + s.fpsSource + ')' : ''))
  if (s.codec) parts.push(s.codec)
  if (stream.frameAt !== null) parts.push('first frame ' + (stream.frameAt / 1000).toFixed(2) + 's')
  else if (stream.stats && stream.stats.firstFrameMs !== null && stream.stats.firstFrameMs !== undefined) parts.push('muxed after ' + (stream.stats.firstFrameMs / 1000).toFixed(2) + 's')
  if (stream.bytes) parts.push(mb(stream.bytes))
  if (stream.stats && stream.stats.framesWritten) parts.push(stream.stats.framesWritten + ' frames out')
  if (s.windowSec) parts.push('window ' + s.windowSec + 's' + (stream.stats && stream.stats.capped ? ' (capped)' : ''))
  return parts.join(' · ')
}

function stop() {
  if (stream.objectUrl) {
    URL.revokeObjectURL(stream.objectUrl)
    stream.objectUrl = ''
  }
  if (stream.socket) {
    stream.socket.onclose = null
    stream.socket.close()
    stream.socket = null
  }
  if (stream.media) {
    try {
      if (stream.media.readyState === 'open') stream.media.endOfStream()
    } catch {}
    stream.media = null
  }
  stream.buffer = null
  stream.queue = []
  stream.bytes = 0
  stream.stats = null
  stream.frameAt = null
}

function pump() {
  const buffer = stream.buffer
  if (!buffer || buffer.updating || !stream.queue.length) return
  try {
    buffer.appendBuffer(stream.queue.shift() as BufferSource)
  } catch (error) {
    console.error('playback append failed', error)
    stream.queue = []
  }
}

function play(file: Clip, offset: number) {
  stop()
  chosen.value = file
  stream.status = {}
  stream.startedAt = performance.now()
  const player = video.value
  if (!player) return
  const duration = secondsBetween(file.begin, file.end)
  scrub.max = duration
  scrub.value = Math.min(offset, duration)
  if (!window.MediaSource) return say('this browser has no MediaSource, so the clip cannot play here.', true)
  say('opening the cloud tunnel…')
  const media = new MediaSource()
  stream.media = media
  stream.objectUrl = URL.createObjectURL(media)
  player.src = stream.objectUrl
  const device = list.asked.device
  media.addEventListener(
    'sourceopen',
    () => {
      let buffer: SourceBuffer
      try {
        buffer = media.addSourceBuffer(MEDIA_TYPE)
      } catch (error) {
        say('this browser cannot decode H.264: ' + (error instanceof Error ? error.message : String(error)), true)
        return
      }
      stream.buffer = buffer
      buffer.addEventListener('updateend', () => {
        pump()
        if (player.buffered.length && stream.frameAt === null) {
          stream.frameAt = performance.now() - stream.startedAt
          say(describeStats())
        }
      })
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const params =
        'device=' + encodeURIComponent(device) +
        '&channel=0' +
        '&name=' + encodeURIComponent(file.name) +
        '&begin=' + encodeURIComponent(file.begin) +
        '&end=' + encodeURIComponent(file.end) +
        '&start=' + Math.round(offset) +
        '&window=' + (stream.windowSec || 60)
      const socket = new WebSocket(scheme + '://' + location.host + '/ws/playback?' + params)
      socket.binaryType = 'arraybuffer'
      stream.socket = socket
      socket.onmessage = (event) => {
        if (typeof event.data === 'string') {
          let frame: Frame | null = null
          try {
            frame = JSON.parse(event.data)
          } catch {
            frame = null
          }
          if (!frame) return
          if (frame.type === 'status') {
            Object.assign(stream.status, frame)
            say(describeStats() || frame.message || '')
          } else if (frame.type === 'error') {
            say(frame.message || 'the playback failed', true)
          } else if (frame.type === 'stats') {
            stream.stats = frame
            say(describeStats())
          }
          return
        }
        const bytes = new Uint8Array(event.data)
        stream.bytes += bytes.length
        stream.queue.push(bytes)
        pump()
        if (stream.status.openMs !== undefined) say(describeStats())
      }
      socket.onerror = () => say('the playback socket failed to connect', true)
      socket.onclose = () => {
        if (stream.media && stream.media.readyState === 'open') {
          try {
            stream.media.endOfStream()
          } catch {}
        }
      }
      player.play().catch(() => {})
    },
    { once: true },
  )
}

async function loadFiles() {
  if (!query.device) return
  const asked: Query = { device: query.device, day: query.day || today() }
  const mine = ++ticket
  list.asked = asked
  list.phase = 'loading'
  list.files = []
  list.error = ''
  list.cap = 0
  try {
    const started = performance.now()
    const body = await api<Listing>(
      '/api/playback/files?device=' + encodeURIComponent(asked.device) + '&channel=0&begin=' +
        encodeURIComponent(asked.day + ' 00:00:00') + '&end=' + encodeURIComponent(asked.day + ' 23:59:59'),
    )
    if (mine !== ticket) return
    list.files = (body && body.files) || []
    list.cap = body && body.truncated ? body.cap || 0 : 0
    list.phase = 'ok'
    await nextTick()
    if (performance.measure) performance.measure('playback-files', { start: started, end: performance.now() })
  } catch (error) {
    if (mine !== ticket) return
    if (sessionLost(error)) return close(false)
    list.error = error instanceof Error ? error.message.charAt(0).toUpperCase() + error.message.slice(1) : String(error)
    list.phase = 'error'
  }
}

function shiftDay(delta: number) {
  const from = stamp((query.day || today()) + ' 00:00:00')
  if (from === null) return
  const moved = new Date(from + delta * 86400000)
  query.day = moved.getFullYear() + '-' + String(moved.getMonth() + 1).padStart(2, '0') + '-' + String(moved.getDate()).padStart(2, '0')
}

function downloadChosen() {
  const file = chosen.value
  if (!file) return
  const anchor = document.createElement('a')
  anchor.href =
    '/api/playback/download?device=' + encodeURIComponent(list.asked.device) +
    '&channel=0&name=' + encodeURIComponent(file.name) +
    '&begin=' + encodeURIComponent(file.begin) +
    '&end=' + encodeURIComponent(file.end) +
    '&bytes=' + encodeURIComponent(String(file.bytes || 0))
  anchor.download = String(file.name).split('/').pop() || 'clip'
  document.body.append(anchor)
  anchor.click()
  anchor.remove()
  say('the device is streaming ' + mb(file.bytes) + '; the browser saves the clip when it lands')
}

function syncDevices() {
  devices.value = (page.devices || []) as Device[]
  if (devices.value.some((device) => device.id === query.device)) return false
  const wanted = devices.value.find((device) => device.id === page.selectedId) || devices.value[0]
  query.device = wanted ? wanted.id : ''
  return true
}

async function show() {
  if (open.value) return
  live.sheet = ''
  open.value = true
  syncDevices()
  if (devices.value.length && page.selectedId && devices.value.some((device) => device.id === page.selectedId)) query.device = page.selectedId
  await nextTick()
  title.value?.focus()
  if (!devices.value.length) return
  if (list.phase === 'idle' || list.asked.device !== query.device || list.asked.day !== query.day) loadFiles()
}

function close(returnFocus = true) {
  if (!open.value) return
  stop()
  ticket += 1
  if (list.phase === 'loading') list.phase = 'idle'
  open.value = false
  if (returnFocus) document.getElementById('pb-open')?.focus()
}

function onKey(event: KeyboardEvent) {
  if (event.key === 'Escape' && !event.defaultPrevented) {
    event.preventDefault()
    close()
  }
  event.stopPropagation()
}

watch(() => [query.device, query.day], () => {
  if (open.value) loadFiles()
})

watch(open, (shown) => (launcher.pressed = shown))

onMounted(() => {
  const screens = new MutationObserver(() => {
    const signin = document.getElementById('signin')
    if (signin && !signin.hidden) {
      close(false)
      chosen.value = null
      say('')
      Object.assign(list, { phase: 'idle', files: [], error: '', cap: 0, asked: { device: '', day: '' } })
      query.device = ''
    }
  })
  const signin = document.getElementById('signin')
  if (signin) screens.observe(signin, { attributes: true, attributeFilter: ['hidden'] })
  const route = () => {
    if ((location.hash || '').replace(/^#/, '') === 'playback') show()
  }
  window.addEventListener('hashchange', route)
  route()
})
</script>

<template>
  <div id="pb" ref="screen" class="pb-screen" data-testid="playback" role="dialog" aria-labelledby="pb-title" :hidden="!open" @keydown="onKey">
    <header class="topbar">
      <button class="ctl back" type="button" data-testid="pb-back" @click="close()"><Icon name="arrow-left" small /><span>Live</span></button>
      <h1 id="pb-title" ref="title" class="screen-title" tabindex="-1">Playback</h1>
      <div class="spacer" />
    </header>
    <div class="pb-body">
      <div class="pb-controls">
        <select id="pb-device" v-model="query.device" aria-label="Camera" data-testid="pb-device" :disabled="!devices.length">
          <option v-for="device in devices" :key="device.id" :value="device.id">{{ device.name || device.id }}</option>
        </select>
        <div class="pb-day" role="group" aria-label="Day">
          <button class="iconbtn" type="button" aria-label="Previous day" title="Previous day" @click="shiftDay(-1)"><Icon name="chevron-left" /></button>
          <input id="pb-day" v-model.lazy="query.day" type="date" aria-label="Day" data-testid="pb-day">
          <button class="iconbtn" type="button" aria-label="Next day" title="Next day" @click="shiftDay(1)"><Icon name="chevron-right" /></button>
        </div>
        <button class="btn btn-line" type="button" data-testid="pb-refresh" :aria-busy="list.phase === 'loading'" :disabled="!query.device" @click="loadFiles"><Icon name="refresh" small />{{ list.phase === 'loading' ? 'Loading…' : 'Refresh' }}</button>
      </div>
      <p v-if="!devices.length && open" class="pb-summary" data-testid="pb-empty-account">No cameras on this account yet.</p>
      <p v-if="list.phase === 'error'" class="err pb-err" role="alert" data-testid="pb-error"><Icon name="warning" small /><span>{{ list.error }}</span></p>
      <p id="pb-summary" class="pb-summary" data-testid="pb-summary" role="status">{{ summary }}</p>
      <div class="pb-strip">
        <div
          id="pb-timeline"
          ref="timeline"
          class="pb-timeline"
          role="group"
          aria-label="Recordings across the day; click to play from that time"
          :title="hover"
          :aria-busy="list.phase === 'loading'"
          @click="onTrackClick"
          @mousemove="onTrackMove"
          @mouseleave="hover = ''"
        >
          <span v-for="hour in HOURS" :key="'t' + hour" class="pb-tick" :style="{ left: (hour * 100) / 24 + '%' }" aria-hidden="true" />
          <span
            v-for="segment in segments"
            :key="segment.key"
            class="pb-seg"
            :data-mode="segment.mode"
            :data-current="segment.current || null"
            :style="{ left: segment.left + '%', width: segment.width + '%' }"
            aria-hidden="true"
          />
        </div>
        <div class="pb-hours" aria-hidden="true">
          <span v-for="hour in HOURS" :key="'h' + hour" :style="{ left: (hour * 100) / 24 + '%' }">{{ String(hour).padStart(2, '0') }}:00</span>
        </div>
        <p class="pb-legend"><span class="pb-key" data-mode="continuous" />Continuous<span class="pb-key" data-mode="motion" />Motion</p>
      </div>
      <div id="pb-files" class="pb-list" data-testid="pb-files">
        <button
          v-for="file in list.files"
          :key="file.name + file.begin"
          class="pb-row"
          type="button"
          :data-mode="modeOf(file)"
          :aria-pressed="chosen === file"
          @click="play(file, 0)"
        >
          <span class="pb-when">{{ span(file) }}</span><span class="pb-size">{{ mb(file.bytes) }}<template v-if="modeOf(file) === 'motion'"> · motion</template></span>
        </button>
      </div>
      <div class="pb-stage">
        <video id="pb-video" ref="video" playsinline muted controls />
        <p id="pb-playing" class="pb-playing" data-testid="pb-playing">{{ chosen ? chosen.begin + ' → ' + chosen.end : 'Pick a recording from the list or the strip.' }}</p>
        <div class="pb-scrub-row">
          <label class="pb-scrub">
            <input id="pb-scrub" v-model.number="scrub.value" type="range" min="0" :max="scrub.max" step="1" :disabled="!chosen" aria-label="Start at">
            <output for="pb-scrub">{{ clock(scrub.value) }}</output>
          </label>
          <button class="btn btn-line" type="button" data-testid="pb-seek" :disabled="!chosen" @click="chosen && play(chosen, scrub.value)">Play from here</button>
          <button id="pb-download" class="btn btn-line" type="button" :disabled="!chosen" :title="chosen ? '' : 'Pick a recording first'" @click="downloadChosen"><Icon name="download" small />Download</button>
        </div>
        <p id="pb-status" :class="['pb-status', { bad: status.bad }]" data-testid="pb-status" :role="status.bad ? 'alert' : 'status'">{{ status.text }}</p>
      </div>
    </div>
  </div>
</template>
