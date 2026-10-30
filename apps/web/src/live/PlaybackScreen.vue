<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { sessionLost } from '../app/shell'
import { state as page } from '../app/state'
import PlaybackCalendar from './PlaybackCalendar.vue'
import PlaybackRows from './PlaybackRows.vue'
import PlaybackStrip from './PlaybackStrip.vue'
import { decodeAlaw, PlaybackSound } from './playback-audio'
import { deviceTime, fileAtTime, mb, secondsBetween, stamp, type Clip } from './playback-clips'
import { addLauncher, live } from './store'

type Device = { id: string; name?: string }
type Listing = { files?: Clip[]; truncated?: boolean; cap?: number; elapsed_ms?: number }
type Query = { device: string; day: string; event: string }
type Frame = { type?: string; message?: string; [key: string]: unknown }
type Mark = { frame: number; at: number }
type Thumb = { name: string; image?: string | null; error?: string | null; source?: string | null }

const MEDIA_TYPE = 'video/mp4; codecs="avc1.42E01E"'
const SPEEDS = [
  { value: 0.25, label: '1/4x' },
  { value: 0.5, label: '1/2x' },
  { value: 1, label: '1x' },
  { value: 2, label: '2x' },
  { value: 4, label: '4x' },
]
const TIME_WINDOW = 3600
const THUMB_BATCH = 6

const open = computed(() => live.screen === 'playback')
const title = ref<HTMLElement>()
const video = ref<HTMLVideoElement>()
const devices = ref<Device[]>([])
const query = reactive<Query>({ device: '', day: today(), event: 'all' })
const kinds = reactive({ regular: true, alarm: true })
const list = reactive({
  phase: 'idle' as 'idle' | 'loading' | 'ok' | 'error',
  asked: { device: '', day: '', event: 'all' } as Query,
  files: [] as Clip[],
  cap: 0,
  error: '',
})
const chosen = ref<Clip | null>(null)
const scrub = reactive({ value: 0, max: 0 })
const status = reactive({ text: '', bad: false })
const player = reactive({ speed: 1, paused: false, sound: false, at: null as number | null, picture: false })
const calendarOpen = ref(false)
const thumbs = reactive<Record<string, Thumb>>({})
const sound = new PlaybackSound()

let ticket = 0
let thumbTicket = 0
let wanted: Clip[] = []
let thumbBusy = false
let ticker = 0
let reporter = 0
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
  windowSec: TIME_WINDOW,
  marks: [] as Mark[],
  origin: null as number | null,
  fps: 0,
  closed: false,
  ended: false,
}

const launcher = addLauncher({ id: 'pb-open', label: 'Playback', icon: 'history', slot: 'end', controls: 'pb', onClick: () => show() })

const cameraName = (id: string) => devices.value.find((device) => device.id === id)?.name || id
const totalBytes = computed(() => list.files.reduce((total, file) => total + file.bytes, 0))
const filterName = computed(() => (list.asked.event === 'regular' ? ' (regular only)' : list.asked.event === 'alarm' ? ' (alarm only)' : ''))
const summary = computed(() => {
  const where = cameraName(list.asked.device) + ' on ' + list.asked.day
  if (list.phase === 'loading') return 'Loading the recordings of ' + where + '…'
  if (list.phase === 'error') return ''
  if (list.phase !== 'ok') return ''
  if (!list.files.length) return 'No recordings of ' + where + filterName.value + '.'
  const cap = list.cap ? ' · the device list is capped at ' + list.cap : ''
  return list.files.length + ' files · ' + mb(totalBytes.value) + ' · ' + where + filterName.value + cap
})
const playhead = computed(() => player.at)
const soundNote = computed(() => {
  if (!chosen.value) return ''
  if (player.speed !== 1) return 'Sound plays at 1x only.'
  return ''
})
const clockText = computed(() => (player.at === null ? '' : 'At ' + deviceTime(player.at).slice(11) + (player.speed !== 1 ? ' · ' + speedLabel(player.speed) : '') + (player.paused ? ' · paused' : '')))

function speedLabel(value: number) {
  return SPEEDS.find((speed) => speed.value === value)?.label || value + 'x'
}

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
  stream.marks = []
  stream.origin = null
  stream.fps = 0
  stream.closed = false
  stream.ended = false
  sound.clear()
  player.picture = false
}

function evict() {
  const buffer = stream.buffer
  const element = video.value
  if (!buffer || buffer.updating || !element || !element.buffered.length) return false
  const behind = element.currentTime - 20
  if (element.buffered.start(0) < behind - 10) {
    try {
      buffer.remove(0, behind)
      return true
    } catch {
      return false
    }
  }
  return false
}

function pump() {
  const buffer = stream.buffer
  if (!buffer || buffer.updating || !stream.queue.length) return
  try {
    buffer.appendBuffer(stream.queue.shift() as BufferSource)
  } catch (error) {
    if (error instanceof DOMException && error.name === 'QuotaExceededError' && evict()) return
    console.error('playback append failed', error)
    stream.queue = []
  }
}

function mediaNow() {
  const element = video.value
  if (!element || stream.origin === null) return null
  return Math.max(0, element.currentTime - stream.origin)
}

function realAt(seconds: number) {
  if (!stream.fps || !stream.marks.length) return null
  const frame = seconds * stream.fps
  let mark = stream.marks[0]
  for (const candidate of stream.marks) {
    if (candidate.frame <= frame) mark = candidate
    else break
  }
  return mark.at + ((frame - mark.frame) / stream.fps) * 1000
}

function follow() {
  const element = video.value
  const now = mediaNow()
  if (!element || now === null) return
  const at = realAt(now)
  player.paused = element.paused
  sound.tick(now, !element.paused && element.playbackRate === 1 && player.sound)
  if (at === null) return
  player.at = at
  const inside = fileAtTime(list.files, at)
  if (inside && inside !== chosen.value) chosen.value = inside
  const file = chosen.value
  if (file) {
    const from = stamp(file.begin)
    if (from !== null) scrub.value = Math.min(Math.max(0, Math.round((at - from) / 1000)), scrub.max)
    scrub.max = secondsBetween(file.begin, file.end)
  }
}

function report() {
  const socket = stream.socket
  const now = mediaNow()
  if (!socket || socket.readyState !== WebSocket.OPEN || !stream.fps) return
  socket.send(JSON.stringify({ type: 'position', frame: Math.floor((now || 0) * stream.fps) }))
}

function onFrame(frame: Frame) {
  if (frame.type === 'clock') {
    const at = stamp(String(frame.at || ''))
    if (at !== null && typeof frame.frame === 'number') stream.marks.push({ frame: frame.frame, at })
    return true
  }
  if (frame.type === 'audio') {
    if (typeof frame.data === 'string' && typeof frame.frame === 'number' && stream.fps) {
      sound.enqueue(frame.frame / stream.fps, decodeAlaw(frame.data), Number(frame.rate) || 8000)
    }
    return true
  }
  return false
}

function continueAfterEnd() {
  if (!stream.closed || !stream.stats || !stream.stats.capped || player.at === null || !open.value) return
  playAt(deviceTime(player.at + 1000))
}

function play(file: Clip, offset: number) {
  const from = stamp(file.begin)
  chosen.value = file
  const duration = secondsBetween(file.begin, file.end)
  scrub.max = duration
  scrub.value = Math.min(offset, duration)
  if (from === null) return say('the recording has no begin time', true)
  playAt(deviceTime(from + Math.min(offset, Math.max(duration - 1, 0)) * 1000))
}

function playAt(begin: string) {
  stop()
  stream.status = {}
  stream.startedAt = performance.now()
  const element = video.value
  if (!element) return
  if (!window.MediaSource) return say('this browser has no MediaSource, so the clip cannot play here.', true)
  const at = stamp(begin)
  player.at = at
  say('opening the cloud tunnel…')
  const media = new MediaSource()
  stream.media = media
  stream.objectUrl = URL.createObjectURL(media)
  element.src = stream.objectUrl
  element.defaultPlaybackRate = player.speed
  element.playbackRate = player.speed
  const device = list.asked.device
  const day = begin.slice(0, 10)
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
        if (!evict()) pump()
        if (element.buffered.length && stream.origin === null) stream.origin = element.buffered.start(0)
        if (element.buffered.length && stream.frameAt === null) {
          stream.frameAt = performance.now() - stream.startedAt
          player.picture = true
          say(describeStats())
        }
      })
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const params =
        'device=' + encodeURIComponent(device) +
        '&channel=0' +
        '&name=' +
        '&begin=' + encodeURIComponent(begin) +
        '&end=' + encodeURIComponent(day + ' 23:59:59') +
        '&start=0' +
        '&window=' + (stream.windowSec || TIME_WINDOW) +
        '&mode=time'
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
          if (onFrame(frame)) return
          if (frame.type === 'status') {
            Object.assign(stream.status, frame)
            if (typeof frame.fps === 'number' && frame.fps > 0) {
              stream.fps = frame.fps
              report()
            }
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
        stream.closed = true
        if (stream.media && stream.media.readyState === 'open') {
          try {
            stream.media.endOfStream()
          } catch {}
        }
      }
      element.play().catch(() => {})
    },
    { once: true },
  )
}

function setSpeed(value: number) {
  player.speed = value
  const element = video.value
  if (!element) return
  element.defaultPlaybackRate = value
  element.playbackRate = value
}

function togglePause() {
  const element = video.value
  if (!element || !chosen.value) return
  if (element.paused) element.play().catch(() => {})
  else element.pause()
  player.paused = element.paused
}

function toggleSound() {
  if (player.sound) {
    player.sound = false
    sound.disable()
    return
  }
  if (!sound.enable()) return say('this browser has no Web Audio, so the recording plays without sound', true)
  player.sound = true
}

function snapshot() {
  const element = video.value
  if (!element || !element.videoWidth) return say('the snapshot needs a picture on screen', true)
  const canvas = document.createElement('canvas')
  canvas.width = element.videoWidth
  canvas.height = element.videoHeight
  const context = canvas.getContext('2d')
  if (!context) return
  context.drawImage(element, 0, 0)
  const at = player.at === null ? 'playback' : deviceTime(player.at).replace(/[: ]/g, '-')
  canvas.toBlob((blob) => {
    if (!blob) return say('the snapshot could not be encoded', true)
    const link = document.createElement('a')
    link.href = URL.createObjectURL(blob)
    link.download = list.asked.device + '_playback_' + at + '.png'
    link.click()
    window.setTimeout(() => URL.revokeObjectURL(link.href), 4000)
    say('saved a snapshot of ' + (player.at === null ? 'the playback' : deviceTime(player.at)))
  }, 'image/png')
}

function setKind(kind: 'regular' | 'alarm', on: boolean) {
  kinds[kind] = on
  if (!kinds.regular && !kinds.alarm) kinds[kind === 'regular' ? 'alarm' : 'regular'] = true
  query.event = kinds.regular && kinds.alarm ? 'all' : kinds.regular ? 'regular' : 'alarm'
}

async function loadFiles() {
  if (!query.device) return
  const asked: Query = { device: query.device, day: query.day || today(), event: query.event }
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
        encodeURIComponent(asked.day + ' 00:00:00') + '&end=' + encodeURIComponent(asked.day + ' 23:59:59') +
        (asked.event !== 'all' ? '&event=' + asked.event : ''),
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

function needThumbs(files: Clip[]) {
  for (const file of files) {
    if (!thumbs[file.name] && !wanted.includes(file)) wanted.push(file)
  }
  void fetchThumbs()
}

async function fetchThumbs() {
  if (thumbBusy || !wanted.length || !list.asked.device) return
  thumbBusy = true
  const mine = thumbTicket
  const device = list.asked.device
  const batch = wanted.splice(0, THUMB_BATCH)
  for (const file of batch) thumbs[file.name] = { name: file.name }
  try {
    const body = await api<{ thumbs?: Thumb[] }>('/api/playback/thumbnails', {
      method: 'POST',
      body: JSON.stringify({ device, channel: 0, files: batch.map((file) => ({ name: file.name, begin: file.begin, end: file.end })) }),
    })
    if (mine === thumbTicket) for (const thumb of (body && body.thumbs) || []) thumbs[thumb.name] = thumb
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (mine === thumbTicket) for (const file of batch) thumbs[file.name] = { name: file.name, error: message }
  } finally {
    thumbBusy = false
  }
  if (mine === thumbTicket) void fetchThumbs()
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
  if (list.phase === 'idle' || list.asked.device !== query.device || list.asked.day !== query.day || list.asked.event !== query.event) loadFiles()
}

function release() {
  stop()
  player.at = null
  if (player.sound) toggleSound()
  ticket += 1
  if (list.phase === 'loading') list.phase = 'idle'
  calendarOpen.value = false
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
  if (calendarOpen.value) {
    calendarOpen.value = false
    return
  }
  close()
}

watch(() => [query.device, query.day, query.event], () => {
  if (open.value) loadFiles()
})

watch(() => [list.asked.device, list.asked.day, list.asked.event], () => {
  thumbTicket += 1
  wanted = []
  thumbBusy = false
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
      Object.assign(list, { phase: 'idle', files: [], error: '', cap: 0, asked: { device: '', day: '', event: 'all' } })
      query.device = ''
    }
  })
  const signin = document.getElementById('signin')
  if (signin) screens.observe(signin, { attributes: true, attributeFilter: ['hidden'] })
  document.addEventListener('keydown', onKey)
  ticker = window.setInterval(() => {
    if (open.value) follow()
  }, 100)
  reporter = window.setInterval(() => {
    if (open.value) report()
  }, 500)
})

onBeforeUnmount(() => {
  window.clearInterval(ticker)
  window.clearInterval(reporter)
  document.removeEventListener('keydown', onKey)
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
          <button class="iconbtn" type="button" aria-label="Days with recordings" title="Days with recordings" data-testid="pb-cal-open" :aria-expanded="calendarOpen" aria-controls="pb-calendar" :disabled="!query.device" @click="calendarOpen = !calendarOpen"><Icon name="calendar" /></button>
          <PlaybackCalendar :device="query.device" :day="query.day" :open="calendarOpen" @pick="(day) => (query.day = day)" @close="calendarOpen = false" />
        </div>
        <div class="pb-kinds" role="group" aria-label="Record type" data-testid="pb-kinds">
          <label><input type="checkbox" :checked="kinds.regular" data-testid="pb-kind-regular" @change="setKind('regular', ($event.target as HTMLInputElement).checked)">Regular</label>
          <label><input type="checkbox" :checked="kinds.alarm" data-testid="pb-kind-alarm" @change="setKind('alarm', ($event.target as HTMLInputElement).checked)">Alarm</label>
        </div>
        <button class="btn btn-line" type="button" data-testid="pb-refresh" :aria-busy="list.phase === 'loading'" :disabled="!query.device" @click="loadFiles"><Icon name="refresh" small />{{ list.phase === 'loading' ? 'Loading…' : 'Refresh' }}</button>
      </div>
      <p v-if="!devices.length && open" class="pb-summary" data-testid="pb-empty-account">No cameras on this account yet.</p>
      <p v-if="list.phase === 'error'" class="err pb-err" role="alert" data-testid="pb-error"><Icon name="warning" small /><span>{{ list.error }}</span></p>
      <p id="pb-summary" class="pb-summary" data-testid="pb-summary" role="status">{{ summary }}</p>
      <PlaybackStrip :files="list.files" :day="list.asked.day" :current="chosen" :busy="list.phase === 'loading'" :playhead="playhead" @pick="play" />
      <PlaybackRows :files="list.files" :current="chosen" :thumbs="thumbs" @play="(file) => play(file, 0)" @need="needThumbs" />
      <div class="pb-stage">
        <video id="pb-video" ref="video" playsinline muted controls @ended="continueAfterEnd" @waiting="continueAfterEnd" />
        <p id="pb-playing" class="pb-playing" data-testid="pb-playing">{{ chosen ? chosen.begin + ' → ' + chosen.end : 'Pick a recording from the list or the strip.' }}</p>
        <p class="pb-clock" data-testid="pb-clock" role="status">{{ clockText }}</p>
        <div class="pb-scrub-row">
          <label class="pb-scrub">
            <input id="pb-scrub" v-model.number="scrub.value" type="range" min="0" :max="scrub.max" step="1" :disabled="!chosen" aria-label="Start at">
            <output for="pb-scrub">{{ clock(scrub.value) }}</output>
          </label>
          <button class="btn btn-line" type="button" data-testid="pb-seek" :disabled="!chosen" @click="chosen && play(chosen, scrub.value)">Play from here</button>
          <button id="pb-download" class="btn btn-line" type="button" :disabled="!chosen" :title="chosen ? '' : 'Pick a recording first'" @click="downloadChosen"><Icon name="download" small />Download</button>
        </div>
        <div class="pb-player-row">
          <button class="btn btn-line" type="button" data-testid="pb-pause" :disabled="!chosen" :aria-pressed="player.paused" @click="togglePause"><Icon :name="player.paused ? 'play' : 'pause'" small />{{ player.paused ? 'Resume' : 'Pause' }}</button>
          <label class="pb-speed">Speed
            <select data-testid="pb-speed" :value="player.speed" aria-label="Playback speed" @change="setSpeed(Number(($event.target as HTMLSelectElement).value))">
              <option v-for="speed in SPEEDS" :key="speed.value" :value="speed.value">{{ speed.label }}</option>
            </select>
          </label>
          <button class="btn btn-line" type="button" data-testid="pb-sound" :disabled="!chosen || player.speed !== 1" :aria-pressed="player.sound" @click="toggleSound"><Icon :name="player.sound ? 'speaker' : 'speaker-off'" small />{{ player.sound ? 'Mute' : 'Sound' }}</button>
          <button class="btn btn-line" type="button" data-testid="pb-snapshot" :disabled="!player.picture" @click="snapshot"><Icon name="camera" small />Snapshot</button>
          <span v-if="soundNote" class="pb-note" data-testid="pb-sound-note">{{ soundNote }}</span>
          <span v-else-if="chosen && !player.picture" class="pb-note" data-testid="pb-snapshot-note">Snapshot waits for the first picture.</span>
        </div>
        <p id="pb-status" :class="['pb-status', { bad: status.bad }]" data-testid="pb-status" :role="status.bad ? 'alert' : 'status'">{{ status.text }}</p>
      </div>
    </div>
  </div>
</template>
