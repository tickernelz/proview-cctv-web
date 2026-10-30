<script setup lang="ts">
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from 'reka-ui'
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../../components/Icon.vue'
import { Button } from '../../components/ui/button'
import { IconButton } from '../../components/ui/icon-button'
import { SegmentedControl } from '../../components/ui/segmented-control'
import { state as page } from '../../app/state'
import PlaybackCalendar from '../PlaybackCalendar.vue'
import { stamp, type Clip } from '../playback-clips'
import { live, menuOpen } from '../store'
import {
  backToLive,
  dayOf,
  dayStart,
  DAY_MS,
  loadDay,
  needThumbs,
  openRecordings,
  playAt,
  playEvent,
  setDay,
  shiftDay,
  snapshot,
  stepFrame,
  tl,
  today,
  togglePause,
  type Marker,
} from './engine'
import { dayLabel, hm, hms } from './time'

const HOUR = 3600000
const ZOOMS = [
  { value: '24h', span: DAY_MS, tick: HOUR, label: 2 * HOUR, major: 6 * HOUR },
  { value: '6h', span: 6 * HOUR, tick: HOUR / 4, label: HOUR, major: 3 * HOUR },
  { value: '1h', span: HOUR, tick: HOUR / 12, label: HOUR / 6, major: HOUR / 2 },
  { value: '10m', span: HOUR / 6, tick: HOUR / 60, label: HOUR / 30, major: HOUR / 6 },
]
const ZOOM_OPTIONS = ZOOMS.map((zoom) => ({ value: zoom.value, label: zoom.value, testid: 'tl-zoom-' + zoom.value }))
const FILM = 5
const MARKER_SNAP = 6

const track = ref<HTMLElement>()
const root = ref<HTMLElement>()
const width = ref(0)
const view = reactive({ zoom: 0, start: dayStart(tl.day) })
const pointer = reactive({ at: null as number | null, x: 0, down: false, moved: false, startX: 0, keyboard: false })
const calendarOpen = ref(false)
const pointers = new Map<number, number>()
let pinch = 0
let wheel = 0
let observer: ResizeObserver | null = null
let thumbTimer = 0

const zoom = computed(() => ZOOMS[view.zoom])
const base = computed(() => dayStart(tl.day))
const playing = computed(() => live.mode === 'playback')
const isToday = computed(() => tl.day === today())
const head = computed(() => {
  const at = playing.value ? live.playback.at : isToday.value ? tl.now : null
  if (at === null || dayOf(at) !== tl.day) return null
  return at
})

const pct = (ms: number) => ((ms - view.start) / zoom.value.span) * 100

function clampStart(start: number, span = zoom.value.span) {
  return Math.min(Math.max(start, base.value), base.value + DAY_MS - span)
}

function label(ms: number) {
  return zoom.value.span === DAY_MS ? hm(ms).slice(0, 2) : hm(ms)
}

const ticks = computed(() => {
  const z = zoom.value
  const out: { key: number; left: number; major: boolean; text: string; edge: boolean }[] = []
  const first = base.value + Math.ceil((view.start - base.value) / z.tick) * z.tick
  for (let at = first; at <= view.start + z.span + 1; at += z.tick) {
    const offset = at - base.value
    const left = pct(at)
    const text = offset % z.label === 0 && offset < DAY_MS && left < 97 ? label(at) : ''
    out.push({ key: at, left, major: offset % z.major === 0, text, edge: left < 1 })
  }
  return out
})

const sorted = computed(() => {
  const list: { file: Clip; from: number; to: number }[] = []
  for (const file of tl.files.list) {
    const from = stamp(file.begin)
    const to = stamp(file.end)
    if (from !== null && to !== null && to >= from) list.push({ file, from, to })
  }
  return list.sort((a, b) => a.from - b.from)
})

const runs = computed(() => {
  const out: { from: number; to: number }[] = []
  for (const entry of sorted.value) {
    const last = out[out.length - 1]
    if (last && entry.from - last.to <= 2000) last.to = Math.max(last.to, entry.to)
    else out.push({ from: entry.from, to: entry.to })
  }
  return out
})

const bars = computed(() => {
  const end = view.start + zoom.value.span
  return runs.value
    .filter((run) => run.to > view.start && run.from < end)
    .map((run) => {
      const left = Math.max(pct(run.from), 0)
      const right = Math.min(pct(run.to), 100)
      return { key: run.from, style: { left: left + '%', width: Math.max(right - left, 0.15) + '%' } }
    })
})

const markers = computed(() =>
  tl.events.list
    .map((marker) => ({ marker, left: pct(marker.at) }))
    .filter((entry) => entry.left >= 0 && entry.left <= 100),
)

const headLeft = computed(() => (head.value === null ? null : pct(head.value)))
const headShown = computed(() => headLeft.value !== null && headLeft.value >= 0 && headLeft.value <= 100)
const cursor = computed(() => pointer.at)
const cursorLeft = computed(() => (cursor.value === null ? null : pct(cursor.value)))

const trackNote = computed(() => {
  if (!page.selectedId) return ''
  if (tl.files.phase === 'loading' && !tl.files.list.length) return 'Loading the recordings of ' + dayLabel(tl.day, false) + '…'
  if (tl.files.phase === 'error') return 'The recordings of ' + dayLabel(tl.day, false) + ' did not load: ' + tl.files.error
  if (tl.files.phase === 'ok' && !tl.files.list.length) return 'No recording on ' + dayLabel(tl.day, false) + (tl.kind === 'all' ? '.' : tl.kind === 'alarm' ? ' (alarm files only).' : ' (regular files only).')
  return ''
})

const valueText = computed(() => {
  const at = cursor.value ?? head.value
  const where = at === null ? dayLabel(tl.day, true) : hms(at) + ', ' + dayLabel(tl.day, true)
  return where + (playing.value ? ', playing a recording' : isToday.value ? ', live' : '')
})

const dayText = computed(() => dayLabel(tl.day, true))
const shortDay = computed(() => dayLabel(tl.day, false))

const film = computed(() => {
  const at = cursor.value
  const list = sorted.value
  if (at === null || !list.length || !width.value) return null
  let index = list.findIndex((entry) => entry.to >= at)
  if (index < 0) index = list.length - 1
  const from = Math.max(0, Math.min(index - Math.floor(FILM / 2), list.length - FILM))
  const frames = list.slice(from, from + FILM).map((entry) => ({
    file: entry.file,
    time: hm(entry.from),
    current: at >= entry.from && at <= entry.to,
    thumb: tl.thumbs[entry.file.name],
  }))
  const filmWidth = frames.length * 100 + 4
  const x = ((cursorLeft.value ?? 0) / 100) * width.value
  const left = Math.min(Math.max(x - filmWidth / 2, 0), Math.max(width.value - filmWidth, 0))
  return { frames, style: { left: left + 'px' } }
})

function timeAt(clientX: number) {
  const box = track.value?.getBoundingClientRect()
  if (!box || !box.width) return null
  const fraction = Math.min(Math.max((clientX - box.left) / box.width, 0), 1)
  return view.start + fraction * zoom.value.span
}

function markerNear(clientX: number): Marker | null {
  const box = track.value?.getBoundingClientRect()
  if (!box || !box.width) return null
  let best: Marker | null = null
  let distance = MARKER_SNAP + 1
  for (const entry of markers.value) {
    const gap = Math.abs(box.left + (entry.left / 100) * box.width - clientX)
    if (gap < distance) {
      distance = gap
      best = entry.marker
    }
  }
  return best
}

function pick(at: number) {
  if (isToday.value && at >= tl.now - 3000) {
    backToLive()
    return
  }
  const list = sorted.value
  if (tl.files.phase === 'ok' && !list.length) return
  const inside = list.find((entry) => at >= entry.from && at < entry.to)
  if (inside || !list.length) return void playAt(at)
  const next = list.find((entry) => entry.from > at)
  const prior = [...list].reverse().find((entry) => entry.to <= at)
  const near = next || prior
  if (near) void playAt(near === next ? near.from : Math.max(near.from, near.to - 10000))
}

function setZoom(index: number, anchor?: number | null) {
  const next = Math.min(Math.max(index, 0), ZOOMS.length - 1)
  if (next === view.zoom) return
  const span = ZOOMS[view.zoom].span
  const nextSpan = ZOOMS[next].span
  const fix = anchor ?? (headShown.value && head.value !== null ? head.value : view.start + span / 2)
  const ratio = (fix - view.start) / span
  view.zoom = next
  view.start = clampStart(fix - ratio * nextSpan, nextSpan)
}

function onZoomPick(value: string) {
  setZoom(ZOOMS.findIndex((entry) => entry.value === value))
}

function onPointerDown(event: PointerEvent) {
  if (event.pointerType === 'mouse' && event.button !== 0) return
  track.value?.setPointerCapture(event.pointerId)
  pointers.set(event.pointerId, event.clientX)
  if (pointers.size === 2) {
    const [a, b] = [...pointers.values()]
    pinch = Math.abs(a - b)
    pointer.down = false
    return
  }
  pointer.keyboard = false
  pointer.down = true
  pointer.moved = false
  pointer.startX = event.clientX
  pointer.at = timeAt(event.clientX)
}

function onPointerMove(event: PointerEvent) {
  if (pointers.has(event.pointerId)) pointers.set(event.pointerId, event.clientX)
  if (pointers.size === 2) {
    const [a, b] = [...pointers.values()]
    const gap = Math.abs(a - b)
    if (pinch > 0 && (gap / pinch > 1.6 || gap / pinch < 0.6)) {
      setZoom(view.zoom + (gap > pinch ? 1 : -1), timeAt((a + b) / 2))
      pinch = gap
    }
    return
  }
  if (event.pointerType !== 'mouse' && !pointer.down) return
  pointer.keyboard = false
  pointer.at = timeAt(event.clientX)
  if (pointer.down && Math.abs(event.clientX - pointer.startX) > 4) pointer.moved = true
}

function onPointerUp(event: PointerEvent) {
  pointers.delete(event.pointerId)
  if (pointers.size) return
  if (!pointer.down) return
  pointer.down = false
  const at = timeAt(event.clientX)
  if (event.pointerType !== 'mouse') pointer.at = null
  if (at === null) return
  const marker = pointer.moved ? null : markerNear(event.clientX)
  if (marker) void playEvent(marker.at)
  else pick(at)
}

function onPointerCancel(event: PointerEvent) {
  pointers.delete(event.pointerId)
  pointer.down = false
  pointer.at = null
}

function onLeave() {
  if (!pointer.down && !pointer.keyboard) pointer.at = null
}

function onWheel(event: WheelEvent) {
  const sideways = Math.abs(event.deltaX) > Math.abs(event.deltaY) || event.shiftKey
  if (sideways) {
    const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY
    const box = track.value?.getBoundingClientRect()
    if (!box || !box.width) return
    view.start = clampStart(view.start + (delta / box.width) * zoom.value.span)
    return
  }
  wheel += event.deltaY
  if (Math.abs(wheel) < 60) return
  setZoom(view.zoom + (wheel < 0 ? 1 : -1), timeAt(event.clientX))
  wheel = 0
}

function onTrackKey(event: KeyboardEvent) {
  const step = zoom.value.tick / (event.shiftKey ? 1 : 4)
  const from = pointer.at ?? head.value ?? view.start + zoom.value.span / 2
  let next: number | null = null
  if (event.key === 'ArrowLeft') next = from - step
  else if (event.key === 'ArrowRight') next = from + step
  else if (event.key === 'Home') next = base.value
  else if (event.key === 'End') next = isToday.value ? tl.now : base.value + DAY_MS - 1000
  else if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    event.stopPropagation()
    if (pointer.at !== null) pick(pointer.at)
    pointer.at = null
    pointer.keyboard = false
    return
  } else if (event.key === 'Escape' && pointer.keyboard) {
    event.preventDefault()
    event.stopPropagation()
    pointer.at = null
    pointer.keyboard = false
    return
  } else if (event.key === '+' || event.key === '=' || event.key === '-') {
    event.preventDefault()
    event.stopPropagation()
    return setZoom(view.zoom + (event.key === '-' ? -1 : 1), pointer.at)
  }
  if (next === null) return
  event.preventDefault()
  event.stopPropagation()
  pointer.keyboard = true
  pointer.at = Math.min(Math.max(next, base.value), base.value + DAY_MS - 1000)
  keepInView(pointer.at)
}

function keepInView(at: number) {
  const span = zoom.value.span
  if (at < view.start || at > view.start + span) view.start = clampStart(at - span / 2)
}

function stepDay(delta: number) {
  setDay(shiftDay(tl.day, delta))
}

function pickDay(day: string) {
  setDay(day)
  calendarOpen.value = false
}

function typing(target: EventTarget | null) {
  return Boolean(target instanceof Element && target.closest('input, textarea, select, [contenteditable="true"]'))
}

function overlayOpen() {
  return Boolean(live.sheet || menuOpen() || document.querySelector('[role="dialog"][data-state="open"]'))
}

function onKey(event: KeyboardEvent) {
  if (live.mode !== 'playback' || event.defaultPrevented || page.screen !== 'watch' || overlayOpen()) return
  if (event.ctrlKey || event.metaKey || event.altKey || typing(event.target)) return
  const key = event.key
  if (key === ' ' && !(event.target instanceof Element && event.target.closest('button, [role="slider"]'))) {
    event.preventDefault()
    togglePause()
  } else if (key === 'ArrowLeft' || key === 'ArrowRight') {
    event.preventDefault()
    stepFrame(key === 'ArrowLeft' ? -1 : 1)
  } else if (key.toLowerCase() === 's' && live.playback.picture) {
    event.preventDefault()
    snapshot()
  } else if (key === 'Escape' && !document.fullscreenElement && !live.padOpen) {
    event.preventDefault()
    backToLive()
  }
}

watch(
  () => tl.day,
  () => {
    view.start = clampStart(head.value !== null ? head.value - zoom.value.span / 2 : view.start)
  },
)

watch(head, (at) => {
  if (at === null || pointer.down || pointer.keyboard) return
  const span = zoom.value.span
  if (at < view.start || at > view.start + span) view.start = clampStart(at - (playing.value ? span / 2 : span * 0.75))
})

watch(
  () => film.value?.frames.map((frame) => frame.file.name).join('|'),
  () => {
    window.clearTimeout(thumbTimer)
    const frames = film.value?.frames
    if (!frames) return
    thumbTimer = window.setTimeout(() => needThumbs(frames.filter((frame) => !frame.thumb).map((frame) => frame.file)), 150)
  },
)

onMounted(() => {
  window.addEventListener('keydown', onKey)
  if (typeof ResizeObserver === 'function' && root.value) {
    observer = new ResizeObserver(() => (width.value = root.value?.clientWidth || 0))
    observer.observe(root.value)
  }
  width.value = root.value?.clientWidth || 0
  if (page.selectedId) loadDay()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey)
  observer?.disconnect()
  window.clearTimeout(thumbTimer)
})
</script>

<template>
  <div id="timeline" ref="root" class="tl" data-testid="timeline" :data-mode="live.mode" role="group" aria-label="Timeline">
    <div class="tl-ruler" aria-hidden="true">
      <template v-for="tick in ticks" :key="tick.key">
        <i :class="['tl-tick', { major: tick.major }]" :style="{ left: tick.left + '%' }" />
        <span v-if="tick.text" :class="['tl-hour', { edge: tick.edge }]" :style="{ left: tick.left + '%' }">{{ tick.text }}</span>
      </template>
    </div>
    <div
      ref="track"
      class="tl-track"
      data-testid="tl-track"
      role="slider"
      tabindex="0"
      :aria-label="'Recordings on ' + dayText + '; Enter plays from the chosen time'"
      aria-valuemin="0"
      aria-valuemax="86399"
      :aria-valuenow="Math.round((((cursor ?? head ?? base) as number) - base) / 1000)"
      :aria-valuetext="valueText"
      :aria-busy="tl.files.phase === 'loading'"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerCancel"
      @pointerleave="onLeave"
      @wheel.prevent="onWheel"
      @keydown="onTrackKey"
      @blur="pointer.keyboard = false; pointer.at = null"
    >
      <span v-for="bar in bars" :key="bar.key" class="tl-bar" data-testid="tl-bar" :style="bar.style" />
      <span
        v-for="entry in markers"
        :key="entry.marker.id"
        class="tl-mark"
        data-testid="tl-mark"
        :data-kind="entry.marker.kind"
        :data-at="entry.marker.at"
        :style="{ left: entry.left + '%' }"
      />
      <p v-if="trackNote" :class="['tl-note', { bad: tl.files.phase === 'error' }]" data-testid="tl-note" :role="tl.files.phase === 'error' ? 'alert' : 'status'">
        <Icon v-if="tl.files.phase === 'error'" name="warning" small />{{ trackNote }}
        <button v-if="tl.files.phase === 'error'" class="tl-retry" type="button" @pointerdown.stop @click.stop="loadDay(true)">Try again</button>
      </p>
      <span v-if="cursorLeft !== null" class="tl-hover" :style="{ left: cursorLeft + '%' }" />
      <template v-if="headShown">
        <span class="tl-head" data-testid="tl-head" :style="{ left: headLeft + '%' }" />
        <span class="tl-pill" data-testid="tl-pill" :data-live="!playing || null" :style="{ left: headLeft + '%' }">{{ hms(head as number) }}</span>
      </template>
    </div>
    <div v-if="film" class="tl-film" data-testid="tl-film" :style="film.style" aria-hidden="true">
      <div v-for="frame in film.frames" :key="frame.file.name" :class="['tl-frame', { current: frame.current }]">
        <span class="tl-thumb" :data-state="frame.thumb?.image ? 'ok' : frame.thumb?.error ? 'none' : 'wait'">
          <img v-if="frame.thumb?.image" :src="frame.thumb.image" alt="">
          <Icon v-else-if="frame.thumb?.error" name="no-picture" small />
        </span>
        <span class="tl-frame-time">{{ frame.time }}</span>
      </div>
    </div>
    <div class="tl-bar-row">
      <div class="tl-zoom">
        <SegmentedControl class="tl-zoom-seg" :options="ZOOM_OPTIONS" :model-value="zoom.value" label="Timeline span" size="sm" @update:model-value="onZoomPick" />
        <IconButton label="Zoom out" icon="zoom-out" size="sm" data-testid="tl-zoom-out" :disabled="view.zoom === 0" @click="setZoom(view.zoom - 1)" />
        <IconButton label="Zoom in" icon="zoom-in" size="sm" data-testid="tl-zoom-in" :disabled="view.zoom === ZOOMS.length - 1" @click="setZoom(view.zoom + 1)" />
        <p :class="['tl-status', { bad: tl.status.bad }]" data-testid="pb-status" :role="tl.status.bad ? 'alert' : 'status'">
          <template v-if="playing">
            <span v-if="live.playback.speed !== 1" data-testid="pb-sound-note">Sound plays at 1x only. </span>
            <span v-if="!live.playback.picture && !tl.status.bad" data-testid="pb-snapshot-note">Snapshot and frame step wait for the first picture. </span>
            <span v-if="live.playback.picture && !tl.file" data-testid="pb-download-note">Download clip needs the file list of the day. </span>
          </template>
          <span :title="tl.status.text">{{ playing || tl.status.bad ? tl.status.text : '' }}</span>
        </p>
      </div>
      <div class="tl-day" role="group" aria-label="Day">
        <IconButton label="Previous day" icon="chevron-left" size="sm" data-testid="tl-prev-day" @click="stepDay(-1)" />
        <PopoverRoot v-model:open="calendarOpen">
          <PopoverTrigger as-child>
            <Button size="sm" variant="ghost" icon="calendar" data-testid="tl-day" :aria-label="'Day: ' + dayText + '. Open the calendar'">
              <span class="tl-day-long">{{ dayText }}</span><span class="tl-day-short">{{ shortDay }}</span>
            </Button>
          </PopoverTrigger>
          <PopoverPortal>
            <PopoverContent class="menu-panel tl-calendar" side="top" align="center" :side-offset="8" :collision-padding="8">
              <PlaybackCalendar :device="page.selectedId || ''" :day="tl.day" :open="calendarOpen" @pick="pickDay" @close="calendarOpen = false" />
            </PopoverContent>
          </PopoverPortal>
        </PopoverRoot>
        <IconButton label="Next day" icon="chevron-right" size="sm" data-testid="tl-next-day" :disabled="isToday" @click="stepDay(1)" />
      </div>
      <div class="tl-end">
        <Button size="sm" variant="ghost" icon="history" data-testid="tl-recordings" aria-label="Recordings" @click="openRecordings('camera')"><span class="tl-label">Recordings</span></Button>
        <Button v-if="playing" size="sm" icon="radio" data-testid="tl-back-live" aria-keyshortcuts="Escape" @click="backToLive()"><span class="tl-label">Back to live</span><span class="tl-label-short">Live</span></Button>
        <span v-else class="tl-live" data-testid="tl-live"><span class="tl-live-dot" aria-hidden="true" />LIVE</span>
      </div>
    </div>
  </div>
</template>
