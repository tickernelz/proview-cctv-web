<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { sessionLost } from '../app/shell'
import { state as page } from '../app/state'
import PlaybackRows from './PlaybackRows.vue'
import PlaybackStrip from './PlaybackStrip.vue'
import { mb, secondsBetween, stamp, type Clip } from './playback-clips'
import { addLauncher, live } from './store'
import { AVC_TYPE, HEVC_FIRST_FRAME_MS, decodes, mayAskForHevc, rememberHevc, type StreamShape } from '../lib/hevc'

type Device = { id: string; name?: string }
type Listing = { files?: Clip[]; truncated?: boolean; cap?: number; elapsed_ms?: number }
type Query = { device: string; day: string }
type Frame = { type?: string; message?: string; [key: string]: unknown }

const open = computed(() => live.screen === 'playback')
const title = ref<HTMLElement>()
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
  transport: '' as '' | 'hevc' | 'avc',
  hevcTimer: 0,
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

function clock(seconds: number) {
  const whole = Math.max(0, Math.round(seconds))
  return String(Math.floor(whole / 60)).padStart(2, '0') + ':' + String(whole % 60).padStart(2, '0')
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
  stream.transport = ''
  if (stream.hevcTimer) window.clearTimeout(stream.hevcTimer)
  stream.hevcTimer = 0
}

function pump() {
  const buffer = stream.buffer
  if (!buffer || buffer.updating || !stream.queue.length) return
  try {
    buffer.appendBuffer(stream.queue.shift() as BufferSource)
  } catch (error) {
    if (stream.transport === 'hevc' && stream.hevcTimer) return fallBack('the H.265 fragment was refused')
    console.error('playback append failed', error)
    stream.queue = []
  }
}

let retryAvc: (() => void) | null = null

function fallBack(reason: string) {
  rememberHevc('no')
  console.warn('H.265 playback failed, switching to the H.264 transcode:', reason)
  const again = retryAvc
  if (again) again()
}

function openBuffer(media: MediaSource, type: string, transport: 'hevc' | 'avc', shape?: StreamShape) {
  if (stream.buffer || stream.media !== media) return
  stream.transport = transport
  let buffer: SourceBuffer
  try {
    buffer = media.addSourceBuffer(type)
  } catch (error) {
    if (transport === 'hevc') return fallBack('the browser refused ' + type)
    say('this browser cannot decode the clip: ' + (error instanceof Error ? error.message : String(error)), true)
    return
  }
  stream.buffer = buffer
  buffer.addEventListener('updateend', () => {
    pump()
    if (video.value && video.value.buffered.length && stream.frameAt === null) {
      stream.frameAt = performance.now() - stream.startedAt
      say(describeStats())
    }
  })
  buffer.addEventListener('error', () => {
    if (stream.transport === 'hevc' && stream.buffer === buffer) fallBack('the source buffer reported an error')
  })
  if (transport === 'hevc' && shape && video.value) {
    const player = video.value as HTMLVideoElement & { requestVideoFrameCallback?: (callback: () => void) => number }
    const confirm = () => {
      if (stream.media !== media || stream.transport !== 'hevc') return
      if (stream.hevcTimer) window.clearTimeout(stream.hevcTimer)
      stream.hevcTimer = 0
      rememberHevc('yes', shape)
    }
    if (player.requestVideoFrameCallback) player.requestVideoFrameCallback(confirm)
    else player.addEventListener('loadeddata', confirm, { once: true })
    stream.hevcTimer = window.setTimeout(() => {
      if (stream.media === media && stream.transport === 'hevc') fallBack('no frame decoded within ' + HEVC_FIRST_FRAME_MS / 1000 + ' s')
    }, HEVC_FIRST_FRAME_MS)
  }
  pump()
}

function chooseBuffer(media: MediaSource, frame: Frame) {
  if (stream.buffer || typeof frame.mime !== 'string') return
  if (frame.transport !== 'hevc') return openBuffer(media, frame.mime, 'avc')
  const shape: StreamShape = { codec: String(frame.codecString || ''), width: Number(frame.width) || null, height: Number(frame.height) || null }
  decodes(shape).then((ok) => {
    if (stream.media !== media) return
    if (!ok) return fallBack('the browser cannot decode ' + shape.codec + ' at ' + shape.width + 'x' + shape.height)
    openBuffer(media, String(frame.mime), 'hevc', shape)
  })
}

function play(file: Clip, offset: number, askHevc = mayAskForHevc()) {
  stop()
  retryAvc = askHevc ? () => play(file, offset, false) : null
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
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const params =
        'device=' + encodeURIComponent(device) +
        '&channel=0' +
        '&name=' + encodeURIComponent(file.name) +
        '&begin=' + encodeURIComponent(file.begin) +
        '&end=' + encodeURIComponent(file.end) +
        '&start=' + Math.round(offset) +
        '&window=' + (stream.windowSec || 60) +
        (askHevc ? '&codec=hevc' : '')
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
            chooseBuffer(media, frame)
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
        if (!stream.buffer && !stream.transport) openBuffer(media, AVC_TYPE, 'avc')
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
  const stem = (String(file.name).split('/').pop() || '').replace(/\.[^.]*$/, '') || 'clip'
  const day = /^\d{4}-\d{2}-\d{2}/.test(file.begin) ? file.begin.slice(0, 10) + '_' : ''
  anchor.download = day + stem + '.mp4'
  document.body.append(anchor)
  anchor.click()
  anchor.remove()
  say('the device is streaming ' + mb(file.bytes) + '; the browser saves the clip when it lands')
}

function show() {
  live.screen = 'playback'
}

async function opened() {
  live.sheet = ''
  devices.value = (page.devices || []) as Device[]
  const wanted = devices.value.find((device) => device.id === page.selectedId) || devices.value[0]
  query.device = wanted ? wanted.id : ''
  await nextTick()
  title.value?.focus()
  if (!query.device) return
  if (list.phase === 'idle' || list.asked.device !== query.device || list.asked.day !== query.day) loadFiles()
}

function release() {
  stop()
  ticket += 1
  if (list.phase === 'loading') list.phase = 'idle'
}

function close(returnFocus = true) {
  if (!open.value) return
  live.screen = ''
  if (location.hash === '#playback') history.replaceState(null, '', '#' + (page.view === 'grid' ? 'grid' : 'live'))
  if (returnFocus) document.getElementById('pb-open')?.focus()
}

function onKey(event: KeyboardEvent) {
  if (!open.value || event.key !== 'Escape' || event.defaultPrevented) return
  event.preventDefault()
  close()
}

watch(() => [query.device, query.day], () => {
  if (open.value) loadFiles()
})

watch(open, (shown) => {
  launcher.pressed = shown
  if (shown) opened()
  else release()
})

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
  document.addEventListener('keydown', onKey)
})
</script>

<template>
  <div id="pb" class="pb-screen" data-testid="playback" role="dialog" aria-labelledby="pb-title" :hidden="!open">
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
      <PlaybackStrip :files="list.files" :day="list.asked.day" :current="chosen" :busy="list.phase === 'loading'" @pick="play" />
      <PlaybackRows :files="list.files" :current="chosen" @play="(file) => play(file, 0)" />
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
