<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { sessionLost } from '../app/shell'
import { state } from '../app/state'
import { settingsUi } from '../settings/ui'
import Icon from '../components/Icon.vue'
import { TechDetails } from '../states'

type Gate = { supported: boolean; evidence: string }
type Rule = { rule_type: number; enabled: boolean; area: [number, number][]; line: [number, number, number, number] | null }
type Summary = {
  enable: boolean | null
  level: number | null
  sensitivity: number | null
  record: boolean | null
  snapshot: boolean | null
  push: boolean | null
  voice: boolean | null
  beep: boolean | null
  alarm_out: boolean | null
  tip: boolean | null
  voice_type: number | null
  schedule: string[][] | null
  show_track: boolean | null
  rule: Rule | null
  duration: number | null
  remote_call: boolean | null
  push_interval: number | null
  push_server: string | null
}
type Detector = { id: string; label: string; config: string; gate: Gate; state: Summary | null; error: string | null }
type Overview = {
  detectors: Detector[]
  tones_gate: Gate
  tones: { id: number; text: string }[]
  rule_limit: { area: boolean; line: boolean; sensitivity: boolean; show_track: boolean } | null
  combined: Gate
  manual_stop: Gate
}
type Field = { field: string; before: unknown; requested: unknown; after: unknown; matches: boolean }
type Report = { block: string; backup: string; fields: Field[]; reverted: boolean; restored: Field[] }
type Reply = { ok: boolean; status: number; body: any }
type Slot = { on: boolean; start: string; end: string; raw: string; held: { on: boolean; start: string; end: string } }
type Form = Omit<Summary, 'schedule' | 'push_server'> & { schedule: Slot[][] | null }
type Cell = 'on' | 'part' | 'off'
type Handle = { label: string; x: number; y: number; set: (x: number, y: number) => void }

const props = defineProps<{ shown: boolean }>()

const LINKS: { key: 'record' | 'snapshot' | 'push' | 'voice' | 'beep' | 'alarm_out' | 'tip'; label: string }[] = [
  { key: 'push', label: 'Send an alarm message to the phone' },
  { key: 'record', label: 'Record on alarm' },
  { key: 'snapshot', label: 'Take a snapshot on alarm' },
  { key: 'voice', label: 'Play the alarm tone on the camera' },
  { key: 'beep', label: 'Beep on the camera' },
  { key: 'alarm_out', label: 'Switch the alarm output' },
  { key: 'tip', label: 'Show the alarm on screen' },
]
const LEVELS = [
  { value: 1, label: 'Low' },
  { value: 3, label: 'Middle' },
  { value: 6, label: 'High' },
]
const TRACK_LEVELS = [
  { value: 0, label: 'Low' },
  { value: 1, label: 'Middle' },
  { value: 2, label: 'High' },
]
const PUSH_MIN = 30
const PUSH_MAX = 1800
const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']
const WEEK = [1, 2, 3, 4, 5, 6, 0]
const HOURS = Array.from({ length: 24 }, (_, hour) => hour)
const CELL_WORDS: Record<Cell, string> = { on: 'detect', part: 'detect part of the hour', off: 'off' }
const SLOT_RE = /^([01]) (\d\d:\d\d:\d\d)-(\d\d:\d\d:\d\d)$/
const RULE_MAX = 8192
const STEP = 64
const BIG_STEP = 512

const status = reactive({ text: 'Not loaded yet.', bad: false })
const data = ref<Overview | null>(null)
const forms = reactive<Record<string, Form>>({})
const reports = ref<Report[]>([])
const busy = ref('')
const stopNote = ref('')
const scheduleMode = reactive<Record<string, 'always' | 'custom'>>({})
const exactOpen = reactive<Record<string, boolean>>({})
const refusal = reactive<Record<string, string>>({})
const cursor = reactive<Record<string, { d: number; h: number }>>({})
const picture = reactive({ url: '', reason: '', ratio: 16 / 9, loading: false })
const stash: Record<string, { on: boolean; start: string; end: string }[][]> = {}

const supported = computed(() => (data.value ? data.value.detectors.filter((item) => item.gate.supported && item.state) : []))
const missing = computed(() => (data.value ? data.value.detectors.filter((item) => !item.gate.supported) : []))
const broken = computed(() => (data.value ? data.value.detectors.filter((item) => item.gate.supported && !item.state) : []))
const regions = computed(() => supported.value.filter((item) => forms[item.id]?.rule))
const schedules = computed(() => supported.value.filter((item) => forms[item.id]?.schedule))
const grids = computed(() => {
  const out: Record<string, number[][]> = {}
  for (const [id, form] of Object.entries(forms)) if (form.schedule) out[id] = form.schedule.map(coverage)
  return out
})

function say(line: string, bad = false) {
  status.text = line
  status.bad = bad
}

function deviceQuery() {
  return state.selectedId ? '?device=' + encodeURIComponent(state.selectedId) : ''
}

async function request(path: string, init?: RequestInit): Promise<Reply> {
  const response = await fetch(path, { credentials: 'same-origin', ...init })
  const raw = await response.text()
  let body: any = null
  try {
    body = raw ? JSON.parse(raw) : null
  } catch {
    body = { message: raw }
  }
  return { ok: response.ok, status: response.status, body }
}

const failureText = (result: Reply) => (result.body && (result.body.message || result.body.error)) || 'HTTP ' + result.status

function toSlot(raw: string): Slot {
  const match = SLOT_RE.exec(raw)
  const held = match ? { on: match[1] === '1', start: match[2], end: match[3] } : { on: false, start: '00:00:00', end: '24:00:00' }
  return { ...held, raw, held }
}

const untouched = (slot: Slot) => slot.on === slot.held.on && slot.start === slot.held.start && slot.end === slot.held.end
const fromSlot = (slot: Slot) => (untouched(slot) ? slot.raw : (slot.on ? '1 ' : '0 ') + seconds(slot.start) + '-' + seconds(slot.end))
const seconds = (time: string) => (time.length === 5 ? time + ':00' : time)
const unreadable = (slot: Slot) => !SLOT_RE.test(slot.raw)
const pad = (value: number) => String(value).padStart(2, '0')
const dayName = (day: number) => DAYS[day] || 'Day ' + (day + 1)

function secondsOf(time: string): number | null {
  const match = /^(\d\d):(\d\d)(?::(\d\d))?$/.exec(time)
  if (!match) return null
  const value = Number(match[1]) * 3600 + Number(match[2]) * 60 + Number(match[3] || 0)
  return value <= 86400 ? value : null
}

function coverage(day: Slot[]): number[] {
  const covered = HOURS.map(() => 0)
  for (const slot of day) {
    if (!slot.on) continue
    const start = secondsOf(slot.start)
    const end = secondsOf(slot.end)
    if (start === null || end === null || end <= start) continue
    for (const hour of HOURS) covered[hour] += Math.max(0, Math.min(end, (hour + 1) * 3600) - Math.max(start, hour * 3600))
  }
  return covered.map((part) => Math.min(1, part / 3600))
}

const fullDay = (day: Slot[]) => !day.length || coverage(day).every((part) => part >= 1)
const isAlways = (schedule: Slot[][]) => schedule.every(fullDay)
const needsExact = (schedule: Slot[][]) => schedule.some((day) => day.some(unreadable) || coverage(day).some((part) => part > 0 && part < 1))
const dayOrder = (count: number) => (count === 7 ? WEEK : Array.from({ length: count }, (_, day) => day))

function cellOf(id: string, day: number, hour: number): Cell {
  const part = grids.value[id]?.[day]?.[hour] ?? 0
  return part >= 1 ? 'on' : part > 0 ? 'part' : 'off'
}

function cellLabel(id: string, day: number, hour: number) {
  return dayName(day) + ' ' + pad(hour) + ':00 to ' + pad(hour + 1) + ':00, ' + CELL_WORDS[cellOf(id, day, hour)]
}

const pressed = (cell: Cell) => (cell === 'on' ? 'true' : cell === 'part' ? 'mixed' : 'false')
const hasPartial = (id: string) => (grids.value[id] || []).some((day) => day.some((part) => part > 0 && part < 1))
const hasUnreadable = (id: string) => (forms[id]?.schedule || []).some((day) => day.some(unreadable))

function runsOf(mask: boolean[]): [number, number][] {
  const runs: [number, number][] = []
  mask.forEach((on, hour) => {
    if (!on) return
    const last = runs[runs.length - 1]
    if (last && last[1] === hour) last[1] = hour + 1
    else runs.push([hour, hour + 1])
  })
  return runs
}

function paint(id: string, d: number, h: number, on: boolean) {
  const day = forms[id]?.schedule?.[d]
  if (!day) return
  const cover = coverage(day)
  if ((on && cover[h] >= 1) || (!on && cover[h] === 0)) return
  const mask = cover.map((part) => part > 0)
  mask[h] = on
  const runs = runsOf(mask)
  if (runs.length > day.length) {
    refusal[id] =
      'This camera holds ' + day.length + ' time range' + (day.length === 1 ? '' : 's') + ' a day; ' + dayName(d) + ' would need ' + runs.length + '. Join or clear a range first.'
    return
  }
  day.forEach((slot, index) => {
    const run = runs[index]
    if (run) {
      slot.on = true
      slot.start = pad(run[0]) + ':00:00'
      slot.end = pad(run[1]) + ':00:00'
    } else slot.on = false
  })
  delete stash[id]
  scheduleMode[id] = isAlways(forms[id].schedule!) ? 'always' : 'custom'
}

function toggleCell(event: MouseEvent, id: string, d: number, h: number) {
  if (event.detail !== 0) return
  refusal[id] = ''
  cursor[id] = { d, h }
  paint(id, d, h, cellOf(id, d, h) !== 'on')
}

function moveCursor(event: KeyboardEvent, id: string, d: number, h: number) {
  const order = dayOrder(forms[id]?.schedule?.length || 0)
  let row = order.indexOf(d)
  let hour = h
  if (event.key === 'ArrowLeft') hour = Math.max(0, h - 1)
  else if (event.key === 'ArrowRight') hour = Math.min(23, h + 1)
  else if (event.key === 'ArrowUp') row = Math.max(0, row - 1)
  else if (event.key === 'ArrowDown') row = Math.min(order.length - 1, row + 1)
  else if (event.key === 'Home') hour = 0
  else if (event.key === 'End') hour = 23
  else return
  event.preventDefault()
  cursor[id] = { d: order[row], h: hour }
  const grid = (event.currentTarget as HTMLElement).closest('.sg')
  grid?.querySelector<HTMLElement>('[data-day="' + order[row] + '"][data-hour="' + hour + '"]')?.focus()
}

let stroke: { id: string; on: boolean; last: string } | null = null

function startPaint(event: PointerEvent, id: string, d: number, h: number) {
  if (event.button !== 0) return
  ;(event.currentTarget as HTMLElement).focus()
  cursor[id] = { d, h }
  refusal[id] = ''
  stroke = { id, on: cellOf(id, d, h) !== 'on', last: '' }
  paintAt(d, h)
  window.addEventListener('pointermove', followPaint)
  window.addEventListener('pointerup', endPaint)
  window.addEventListener('pointercancel', endPaint)
}

function paintAt(d: number, h: number) {
  if (!stroke || stroke.last === d + ':' + h) return
  stroke.last = d + ':' + h
  paint(stroke.id, d, h, stroke.on)
}

function followPaint(event: PointerEvent) {
  if (!stroke) return
  const cell = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>('[data-hour]')
  if (!cell || cell.dataset.detector !== stroke.id) return
  paintAt(Number(cell.dataset.day), Number(cell.dataset.hour))
}

function endPaint() {
  stroke = null
  window.removeEventListener('pointermove', followPaint)
  window.removeEventListener('pointerup', endPaint)
  window.removeEventListener('pointercancel', endPaint)
}

function setMode(id: string, mode: 'always' | 'custom') {
  const schedule = forms[id]?.schedule
  if (!schedule) return
  if (mode === 'always' && !isAlways(schedule)) {
    stash[id] = schedule.map((day) => day.map((slot) => ({ on: slot.on, start: slot.start, end: slot.end })))
    for (const day of schedule) {
      if (fullDay(day)) continue
      day.forEach((slot, index) => {
        if (index === 0) {
          slot.on = true
          slot.start = '00:00:00'
          slot.end = '24:00:00'
        } else slot.on = false
      })
    }
  }
  if (mode === 'custom' && stash[id]) {
    stash[id].forEach((day, d) => day.forEach((held, index) => Object.assign(schedule[d][index], held)))
    delete stash[id]
  }
  refusal[id] = ''
  scheduleMode[id] = mode
}

const clampRule = (value: number) => Math.min(RULE_MAX, Math.max(0, Math.round(value)))
const spot = (value: number) => (clampRule(Number(value)) / RULE_MAX) * 100 + '%'

function handlesOf(rule: Rule): Handle[] {
  if (rule.rule_type === 1) {
    return rule.area.map((pt, index) => ({
      label: 'Corner ' + (index + 1),
      x: Number(pt[0]),
      y: Number(pt[1]),
      set: (x, y) => {
        pt[0] = x
        pt[1] = y
      },
    }))
  }
  const line = rule.line
  if (!line) return []
  return [
    { label: 'Line start', x: Number(line[0]), y: Number(line[1]), set: (x, y) => { line[0] = x; line[1] = y } },
    { label: 'Line end', x: Number(line[2]), y: Number(line[3]), set: (x, y) => { line[2] = x; line[3] = y } },
  ]
}

const pointsOf = (rule: Rule) => rule.area.map(([x, y]) => clampRule(Number(x)) + ',' + clampRule(Number(y))).join(' ')

function grab(event: PointerEvent, handle: Handle) {
  if (event.button !== 0) return
  const target = event.currentTarget as HTMLElement
  const box = target.closest<HTMLElement>('.rg-box')
  if (!box) return
  event.preventDefault()
  target.focus()
  target.setPointerCapture(event.pointerId)
  const follow = (move: PointerEvent) => {
    const rect = box.getBoundingClientRect()
    if (!rect.width || !rect.height) return
    handle.set(clampRule(((move.clientX - rect.left) / rect.width) * RULE_MAX), clampRule(((move.clientY - rect.top) / rect.height) * RULE_MAX))
  }
  const drop = () => {
    target.removeEventListener('pointermove', follow)
    target.removeEventListener('pointerup', drop)
    target.removeEventListener('pointercancel', drop)
  }
  target.addEventListener('pointermove', follow)
  target.addEventListener('pointerup', drop)
  target.addEventListener('pointercancel', drop)
}

function nudge(event: KeyboardEvent, handle: Handle) {
  const step = event.shiftKey ? BIG_STEP : STEP
  let x = handle.x
  let y = handle.y
  if (event.key === 'ArrowLeft') x -= step
  else if (event.key === 'ArrowRight') x += step
  else if (event.key === 'ArrowUp') y -= step
  else if (event.key === 'ArrowDown') y += step
  else return
  event.preventDefault()
  handle.set(clampRule(x), clampRule(y))
}

let pictureTicket = 0

function dropPicture() {
  if (picture.url) URL.revokeObjectURL(picture.url)
  picture.url = ''
}

function noPicture(reason: string) {
  dropPicture()
  picture.reason = reason
}

async function loadPicture() {
  const mine = ++pictureTicket
  picture.loading = true
  picture.reason = ''
  try {
    const response = await fetch('/api/device/controls/snapshot' + deviceQuery(), { credentials: 'same-origin' })
    if (mine !== pictureTicket) return
    const type = response.headers.get('content-type') || ''
    if (!response.ok || !type.startsWith('image/')) {
      const raw = await response.text()
      let body: any = null
      try {
        body = raw ? JSON.parse(raw) : null
      } catch {
        body = null
      }
      if (mine !== pictureTicket) return
      noPicture(response.ok ? 'the camera answered without a picture.' : failureText({ ok: false, status: response.status, body }))
      return
    }
    const blob = await response.blob()
    if (mine !== pictureTicket) return
    dropPicture()
    picture.url = URL.createObjectURL(blob)
  } catch (error) {
    if (mine === pictureTicket) noPicture(error instanceof Error ? error.message : String(error))
  } finally {
    if (mine === pictureTicket) picture.loading = false
  }
}

function pictureLoaded(event: Event) {
  const image = event.target as HTMLImageElement
  if (image.naturalWidth && image.naturalHeight) picture.ratio = image.naturalWidth / image.naturalHeight
}

function formOf(summary: Summary): Form {
  const copy = JSON.parse(JSON.stringify(summary)) as Summary
  return { ...copy, schedule: copy.schedule ? copy.schedule.map((day) => day.map(toSlot)) : null }
}

function prime(id: string) {
  delete stash[id]
  refusal[id] = ''
  const schedule = forms[id]?.schedule
  if (!schedule) return
  scheduleMode[id] = isAlways(schedule) ? 'always' : 'custom'
  exactOpen[id] = exactOpen[id] || needsExact(schedule)
  if (!cursor[id]) cursor[id] = { d: dayOrder(schedule.length)[0] ?? 0, h: 0 }
}

function fill(overview: Overview) {
  for (const key of Object.keys(forms)) delete forms[key]
  for (const item of overview.detectors) if (item.state) forms[item.id] = formOf(item.state)
  for (const id of Object.keys(forms)) prime(id)
}

let ticket = 0

async function load(afterWrite = false) {
  const mine = ++ticket
  if (!afterWrite) say('Reading the alarm settings from the camera…')
  const result = await request('/api/device/detect' + deviceQuery())
  if (mine !== ticket) return
  if (result.status === 401) {
    sessionLost({ status: 401 })
    return
  }
  if (!result.ok || !result.body) {
    say('Could not read the alarm settings: ' + failureText(result), true)
    return
  }
  data.value = result.body.overview
  fill(result.body.overview)
  if (!afterWrite) say('Read from the camera just now.')
  if (Object.values(forms).some((form) => form.rule) && (!afterWrite || !picture.url)) void loadPicture()
}

function same(a: unknown, b: unknown) {
  return JSON.stringify(a) === JSON.stringify(b)
}

function patchOf(item: Detector): Record<string, unknown> {
  const before = item.state as Summary
  const form = forms[item.id]
  const patch: Record<string, unknown> = {}
  const keys = ['enable', 'level', 'sensitivity', 'record', 'snapshot', 'push', 'voice', 'beep', 'alarm_out', 'tip', 'voice_type', 'show_track', 'duration', 'remote_call', 'push_interval'] as const
  for (const key of keys) {
    const value = form[key]
    if (value === null || value === undefined || value === ('' as unknown)) continue
    if (item.id === 'push' && key !== 'push_interval') continue
    let wanted = typeof before[key] === 'number' ? Number(value) : value
    if (key === 'push_interval') {
      if (Number(value) === before.push_interval) continue
      wanted = Math.min(PUSH_MAX, Math.max(PUSH_MIN, Math.round(Number(value))))
      form.push_interval = wanted as number
    }
    if (!same(wanted, before[key])) patch[key] = wanted
  }
  if (form.schedule && before.schedule) {
    const schedule = form.schedule.map((day) => day.map(fromSlot))
    if (!same(schedule, before.schedule)) patch.schedule = schedule
  }
  if (form.rule && before.rule) {
    const rule: Record<string, unknown> = {}
    if (form.rule.rule_type !== before.rule.rule_type) rule.rule_type = Number(form.rule.rule_type)
    if (form.rule.enabled !== before.rule.enabled) rule.enabled = form.rule.enabled
    const area = form.rule.area.map(([x, y]) => [Number(x), Number(y)])
    if (!same(area, before.rule.area)) rule.area = area
    const line = form.rule.line ? form.rule.line.map(Number) : null
    if (line && !same(line, before.rule.line)) rule.line = line
    if (Object.keys(rule).length) patch.rule = rule
  }
  return patch
}

async function save(item: Detector) {
  if (busy.value) {
    say('Wait for the ' + busy.value + ' write to finish.')
    return
  }
  const patch = patchOf(item)
  if (!Object.keys(patch).length) {
    say('Nothing changed on ' + item.label.toLowerCase() + '.')
    return
  }
  busy.value = item.label
  say('Writing ' + item.label.toLowerCase() + '…')
  try {
    const result = await request('/api/device/detect/' + item.id + deviceQuery(), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(patch),
    })
    if (result.status === 401) {
      sessionLost({ status: 401 })
      return
    }
    if (!result.ok) {
      say('Could not write ' + item.label.toLowerCase() + ': ' + failureText(result), true)
      return
    }
    const report: Report = result.body.report
    reports.value = [report]
    if (report.reverted) say('The camera returned a different value, so the saved copy was put back.', true)
    else say(item.label + ' read back after the write and kept.')
    await load(true)
  } finally {
    busy.value = ''
  }
}

function reset(item: Detector) {
  if (!item.state) return
  forms[item.id] = formOf(item.state)
  prime(item.id)
}

async function stopAlarm() {
  stopNote.value = 'Sending stop…'
  try {
    const result = await request('/api/device/detect/alarm/stop' + deviceQuery(), { method: 'POST' })
    stopNote.value = result.ok ? 'The camera acknowledged the stop.' : 'Stop failed: ' + failureText(result)
  } catch (error) {
    stopNote.value = 'Stop failed: ' + (error instanceof Error ? error.message : String(error))
  }
}

function trackBlocked(item: Detector) {
  if (item.id !== 'track' || !data.value || data.value.combined.supported) return ''
  const human = data.value.detectors.find((entry) => entry.id === 'human')
  return human && human.state && human.state.enable ? 'Human detection is on, and this camera cannot run both at once.' : ''
}

function levelOptions(current: number | null) {
  return current === null || LEVELS.some((option) => option.value === current) ? LEVELS : [...LEVELS, { value: current, label: 'Level ' + current }]
}

const sensitivityFixed = (item: Detector) => item.id === 'human' && !data.value?.rule_limit?.sensitivity

watch(
  () => props.shown,
  (shown) => {
    if (shown) load()
  },
)

watch(
  () => settingsUi.reload.alarm,
  () => {
    if (props.shown) load()
  },
)

onBeforeUnmount(() => {
  endPaint()
  pictureTicket++
  dropPicture()
})
</script>

<template>
  <header class="set-head">
    <div>
      <h2 id="h-alarm" class="set-h" tabindex="-1">Detection &amp; alarms</h2>
      <p class="set-sub">Detection, schedule and alarm actions as the camera holds them. Each save is checked with the camera.</p>
    </div>
  </header>
  <Teleport defer to="#savebar-note">
    <p v-show="shown" id="detect-status" :class="['set-status', { 'warn-text': status.bad }]" data-testid="detect-status" role="status">
      <Icon v-if="status.bad" name="warning" small /><span>{{ status.text }}</span>
    </p>
  </Teleport>
  <p v-if="!data" class="set-empty">Nothing read from this camera yet.</p>
  <div v-else :class="regions.length || schedules.length ? 'set-cols' : 'set-stack'">
    <div class="set-col">
      <div v-for="report in reports" :key="report.backup" class="set-card" data-testid="detect-report">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t"><Icon :name="report.reverted ? 'warning' : 'check'" small />Last write</h3>
            <p class="set-card-d">{{ report.block }}</p>
          </div>
        </div>
        <p class="hint">{{ (report.reverted ? 'Put back from the saved copy. ' : 'Read back and kept. ') + 'Copy: ' + report.backup }}</p>
        <div class="row stack">
          <div class="ctl-col">
            <table class="tbl">
              <thead>
                <tr>
                  <th scope="col">Field</th>
                  <th scope="col">Before</th>
                  <th scope="col">After</th>
                  <th scope="col">State</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="field in report.fields" :key="field.field">
                  <th scope="row">{{ field.field }}</th>
                  <td class="code">{{ JSON.stringify(field.before) }}</td>
                  <td class="code">{{ JSON.stringify(field.after) }}</td>
                  <td>{{ field.matches ? 'matches' : 'differs' }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
      <div v-for="item in supported" :key="item.id" class="set-card" :data-testid="'detect-' + item.id">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">{{ item.label }}</h3>
          </div>
          <TechDetails :data-testid="'detect-' + item.id + '-details'" :text="item.gate.evidence" />
        </div>
        <div v-if="forms[item.id].enable !== null && item.id !== 'push'" class="row">
          <label class="lab" :for="'detect-' + item.id + '-enable'">Detection<span v-if="trackBlocked(item)" class="hint warn-text">{{ trackBlocked(item) }}</span></label>
          <div class="ctl-col">
            <input :id="'detect-' + item.id + '-enable'" v-model="forms[item.id].enable" type="checkbox" role="switch" :disabled="Boolean(trackBlocked(item))" :data-testid="'detect-' + item.id + '-enable'">
            <TechDetails v-if="trackBlocked(item)" :data-testid="'detect-' + item.id + '-blocked-details'" :text="data.combined.evidence" />
          </div>
        </div>
        <div v-if="forms[item.id].level !== null" class="row">
          <label class="lab" :for="'detect-' + item.id + '-level'">Sensitivity</label>
          <div class="ctl-col">
            <select :id="'detect-' + item.id + '-level'" v-model.number="forms[item.id].level">
              <option v-for="option in levelOptions(item.state!.level)" :key="option.value" :value="option.value">{{ option.label }}</option>
            </select>
          </div>
        </div>
        <div v-if="forms[item.id].sensitivity !== null" class="row">
          <label :id="'detect-' + item.id + '-sensitivity-label'" class="lab" :for="item.id === 'track' || sensitivityFixed(item) ? undefined : 'detect-' + item.id + '-sensitivity'">
            Sensitivity<span v-if="sensitivityFixed(item)" class="hint">Fixed by this camera.</span>
          </label>
          <div class="ctl-col">
            <div v-if="item.id === 'track'" :id="'detect-' + item.id + '-sensitivity'" class="seg" role="radiogroup" :aria-labelledby="'detect-' + item.id + '-sensitivity-label'">
              <label v-for="option in TRACK_LEVELS" :key="option.value">
                <input v-model="forms[item.id].sensitivity" type="radio" :name="'detect-' + item.id + '-sensitivity'" :value="option.value">{{ option.label }}
              </label>
            </div>
            <template v-else-if="sensitivityFixed(item)">
              <p class="val">{{ item.state!.sensitivity }}</p>
              <TechDetails :data-testid="'detect-' + item.id + '-sensitivity-details'" text="HumanRuleLimit Sensitivity=false" />
            </template>
            <input v-else :id="'detect-' + item.id + '-sensitivity'" v-model.number="forms[item.id].sensitivity" type="number" step="1" :min="item.id === 'volume' ? 70 : 0" :max="item.id === 'volume' ? 110 : 100">
          </div>
        </div>
        <div v-if="forms[item.id].show_track !== null" class="row">
          <label class="lab" :for="'detect-' + item.id + '-track-box'">
            Track box<span class="hint">Draw the track box on people</span>
            <span v-if="item.id === 'human' && !data.rule_limit?.show_track" class="hint">Fixed by this camera.</span>
          </label>
          <div class="ctl-col">
            <input :id="'detect-' + item.id + '-track-box'" v-model="forms[item.id].show_track" type="checkbox" role="switch" :disabled="item.id === 'human' && !data.rule_limit?.show_track">
            <TechDetails v-if="item.id === 'human' && !data.rule_limit?.show_track" :data-testid="'detect-' + item.id + '-track-box-details'" text="HumanRuleLimit ShowTrack=false" />
          </div>
        </div>
        <template v-for="link in LINKS" :key="link.key">
          <div v-if="forms[item.id][link.key] !== null" class="row">
            <label class="lab" :for="'detect-' + item.id + '-' + link.key + '-switch'">{{ link.label }}</label>
            <div class="ctl-col">
              <input :id="'detect-' + item.id + '-' + link.key + '-switch'" v-model="forms[item.id][link.key]" type="checkbox" role="switch" :data-testid="'detect-' + item.id + '-' + link.key">
            </div>
          </div>
        </template>
        <div v-if="forms[item.id].voice_type !== null" class="row">
          <label class="lab" :for="'detect-' + item.id + '-tone'">
            Alarm tone<span class="hint">Only the tone id is written; nothing plays from here.</span>
          </label>
          <div class="ctl-col">
            <select v-if="data.tones_gate.supported && data.tones.length" :id="'detect-' + item.id + '-tone'" v-model.number="forms[item.id].voice_type">
              <option v-for="tone in data.tones" :key="tone.id" :value="tone.id">{{ tone.text }}</option>
            </select>
            <template v-else>
              <p class="hint">Fixed at tone {{ item.state!.voice_type }} on this camera.</p>
              <TechDetails :data-testid="'detect-' + item.id + '-tone-details'" :text="data.tones_gate.evidence" />
            </template>
          </div>
        </div>
        <div v-if="forms[item.id].duration !== null" class="row">
          <label class="lab" :for="'detect-' + item.id + '-duration'">Alert length (s)</label>
          <div class="ctl-col"><input :id="'detect-' + item.id + '-duration'" v-model.number="forms[item.id].duration" type="number" min="1" max="600" step="1"></div>
        </div>
        <div v-if="forms[item.id].remote_call !== null" class="row">
          <label class="lab" :for="'detect-' + item.id + '-call'">One-key call</label>
          <div class="ctl-col">
            <input :id="'detect-' + item.id + '-call'" v-model="forms[item.id].remote_call" type="checkbox" role="switch">
          </div>
        </div>
        <div v-if="forms[item.id].push_interval !== null" class="row">
          <label class="lab" :for="'detect-' + item.id + '-interval'">
            Push interval (s)<span class="hint">{{ item.state!.enable ? 'Push service on' : 'Push service off' }} · {{ item.state!.push_server || 'no server' }}</span>
          </label>
          <div class="ctl-col">
            <input :id="'detect-' + item.id + '-interval'" v-model.number="forms[item.id].push_interval" :min="PUSH_MIN" :max="PUSH_MAX" type="number" step="1" data-testid="detect-push-interval">
            <p v-if="item.state!.push_interval !== null && (item.state!.push_interval < PUSH_MIN || item.state!.push_interval > PUSH_MAX)" class="hint" data-testid="detect-push-outside">
              The camera holds {{ item.state!.push_interval }} s, outside the {{ PUSH_MIN }} to {{ PUSH_MAX }} s the app offers; it stays until you save a new value.
            </p>
            <p class="hint">The app offers 30 to 1800 seconds. Whether an alarm pushes at all is the message switch on each detector.</p>
          </div>
        </div>
        <p v-if="forms[item.id].rule || forms[item.id].schedule" class="hint">
          {{ forms[item.id].rule && forms[item.id].schedule ? 'The detection region and schedule are' : forms[item.id].rule ? 'The detection region is' : 'The schedule is' }} saved with this card.
        </p>
        <div class="set-card-f">
          <button class="btn" type="button" @click="reset(item)">Undo edits</button>
          <button class="btn btn-fill" type="button" :data-testid="'detect-save-' + item.id" :aria-busy="busy === item.label" @click="save(item)">
            {{ busy === item.label ? 'Saving…' : 'Save ' + item.label.toLowerCase() }}
          </button>
        </div>
      </div>
      <div class="set-card" data-testid="detect-stop">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Stop sounding alarm</h3></div>
        </div>
        <div class="row">
          <div class="lab">
            Manual alarm<span class="hint" data-testid="detect-stop-reason">{{ data.manual_stop.supported ? stopNote || 'Sends only the stop command; nothing here starts a sound.' : 'Not offered by this camera.' }}</span>
          </div>
          <div class="ctl-col">
            <button class="btn" type="button" :disabled="!data.manual_stop.supported" data-testid="detect-stop-button" @click="stopAlarm">Stop the alarm</button>
            <TechDetails v-if="!data.manual_stop.supported" data-testid="detect-stop-details" :text="data.manual_stop.evidence" />
          </div>
        </div>
      </div>
      <div v-if="missing.length || broken.length" class="set-card">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">Not on this camera</h3>
            <p class="set-card-d">The phone app hides these for this camera too.</p>
          </div>
        </div>
        <div v-for="item in missing" :key="item.id" class="row" :data-testid="'detect-missing-' + item.id">
          <div class="lab">{{ item.label }}</div>
          <div class="ctl-col">
            <p class="val">Not offered by this camera.</p>
            <TechDetails :data-testid="'detect-missing-' + item.id + '-details'" :text="item.gate.evidence" />
          </div>
        </div>
        <div v-for="item in broken" :key="item.id" class="row">
          <div class="lab">{{ item.label }}<span class="hint warn-text">{{ item.error }}</span></div>
          <div class="ctl-col"><p class="val warn-text"><Icon name="warning" small /> Could not read</p></div>
        </div>
      </div>
    </div>
    <div v-if="regions.length || schedules.length" class="set-col">
      <div v-for="item in regions" :key="item.id + '-region'" class="set-card" :data-testid="'detect-' + item.id + '-region'">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">Detection region · {{ item.label }}</h3>
            <p class="set-card-d">{{ forms[item.id].rule!.rule_type === 1 ? 'Area' : 'Line' }} on the whole camera picture. Coordinates run 0 to 8192 across and down the picture.</p>
          </div>
        </div>
        <div class="row">
          <label class="lab" :for="'detect-' + item.id + '-rule-on'">Detection rule</label>
          <div class="ctl-col">
            <label class="switchrow"><input :id="'detect-' + item.id + '-rule-on'" v-model="forms[item.id].rule!.enabled" type="checkbox" role="switch"><span>Rule on</span></label>
          </div>
        </div>
        <div class="row">
          <div :id="'detect-' + item.id + '-rule-type-label'" class="lab">
            Rule type<span v-if="!data.rule_limit?.area || !data.rule_limit?.line" class="hint">This camera does not offer {{ !data.rule_limit?.area && !data.rule_limit?.line ? 'either type' : !data.rule_limit?.area ? 'an area' : 'a line' }}.</span>
          </div>
          <div class="ctl-col">
            <div class="seg" role="radiogroup" :aria-labelledby="'detect-' + item.id + '-rule-type-label'" :data-testid="'detect-' + item.id + '-rule-type'">
              <label>
                <input v-model="forms[item.id].rule!.rule_type" type="radio" :name="'detect-' + item.id + '-rule-type'" :value="1" :disabled="!data.rule_limit?.area">Area{{ data.rule_limit?.area ? '' : ' (not offered)' }}
              </label>
              <label>
                <input v-model="forms[item.id].rule!.rule_type" type="radio" :name="'detect-' + item.id + '-rule-type'" :value="0" :disabled="!data.rule_limit?.line">Line{{ data.rule_limit?.line ? '' : ' (not offered)' }}
              </label>
            </div>
          </div>
        </div>
        <div class="row stack">
          <div class="ctl-col">
            <div class="rg-box" :style="{ aspectRatio: String(picture.ratio) }">
              <img v-if="picture.url" class="rg-img" :src="picture.url" alt="" @load="pictureLoaded" @error="noPicture('the picture did not decode.')">

              <svg class="rg-svg" viewBox="0 0 8192 8192" preserveAspectRatio="none" aria-hidden="true">
                <polygon v-if="forms[item.id].rule!.rule_type === 1 && forms[item.id].rule!.area.length" class="rg-area" :points="pointsOf(forms[item.id].rule!)" />
                <line
                  v-else-if="forms[item.id].rule!.rule_type !== 1 && forms[item.id].rule!.line"
                  class="rg-line"
                  :x1="clampRule(Number(forms[item.id].rule!.line![0]))"
                  :y1="clampRule(Number(forms[item.id].rule!.line![1]))"
                  :x2="clampRule(Number(forms[item.id].rule!.line![2]))"
                  :y2="clampRule(Number(forms[item.id].rule!.line![3]))"
                />
              </svg>
              <button
                v-for="(handle, index) in handlesOf(forms[item.id].rule!)"
                :key="handle.label"
                class="rg-handle"
                type="button"
                :style="{ left: spot(handle.x), top: spot(handle.y) }"
                :aria-label="handle.label + ': ' + handle.x + ', ' + handle.y"
                :aria-describedby="'detect-' + item.id + '-region-help'"
                :data-testid="'detect-' + item.id + '-handle-' + (index + 1)"
                @pointerdown="grab($event, handle)"
                @keydown="nudge($event, handle)"
              />
            </div>
<p v-if="!picture.url" class="rg-empty hint" :data-testid="'detect-' + item.id + '-picture-note'">
              {{ picture.loading ? 'Taking a snapshot from the camera…' : 'No picture to draw on: ' + (picture.reason || 'not loaded yet.') }}
            </p>
            <p :id="'detect-' + item.id + '-region-help'" class="hint">
              {{ forms[item.id].rule!.rule_type !== 1 && !forms[item.id].rule!.line ? 'The camera holds no line for this rule.' : 'Drag a handle, or focus it and press the arrow keys: 64 per press, 512 with Shift.' }}
            </p>
          </div>
        </div>
        <div class="row stack">
          <details class="detect-exact">
            <summary>Exact coordinates</summary>
            <div v-if="forms[item.id].rule!.rule_type === 1" class="detect-points">
              <label v-for="(pt, index) in forms[item.id].rule!.area" :key="index" class="field">
                <span>Corner {{ index + 1 }} (x, y)</span>
                <span class="detect-pair">
                  <input v-model.number="pt[0]" type="number" min="0" max="8192" step="1" :aria-label="'Corner ' + (index + 1) + ' x'">
                  <input v-model.number="pt[1]" type="number" min="0" max="8192" step="1" :aria-label="'Corner ' + (index + 1) + ' y'">
                </span>
              </label>
            </div>
            <div v-else-if="forms[item.id].rule!.line" class="detect-points">
              <label v-for="(name, index) in ['Start x', 'Start y', 'End x', 'End y']" :key="name" class="field">
                <span>{{ name }}</span>
                <input v-model.number="forms[item.id].rule!.line![index]" type="number" min="0" max="8192" step="1">
              </label>
            </div>
          </details>
        </div>
        <div class="set-card-f">
          <p class="hint">Saved with Save {{ item.label.toLowerCase() }}.</p>
        </div>
      </div>
      <div v-for="item in schedules" :key="item.id + '-schedule'" class="set-card" :data-testid="'detect-' + item.id + '-schedule'">
        <div class="set-card-h">
          <div>
            <h3 :id="'detect-' + item.id + '-schedule-title'" class="set-card-t">Schedule · {{ item.label }}</h3>
            <p class="set-card-d">Each day holds {{ forms[item.id].schedule![0]?.length || 0 }} time slots; a slot arms detection when it is on.</p>
          </div>
          <div class="set-card-a">
            <div class="seg" role="radiogroup" :aria-label="'Schedule for ' + item.label.toLowerCase()">
              <label><input type="radio" :name="'detect-' + item.id + '-mode'" value="always" :checked="scheduleMode[item.id] === 'always'" @change="setMode(item.id, 'always')">Always</label>
              <label><input type="radio" :name="'detect-' + item.id + '-mode'" value="custom" :checked="scheduleMode[item.id] === 'custom'" @change="setMode(item.id, 'custom')">Custom</label>
            </div>
          </div>
        </div>
        <div class="row stack">
          <div class="ctl-col">
            <div class="sg" role="group" :aria-labelledby="'detect-' + item.id + '-schedule-title'" :aria-describedby="'detect-' + item.id + '-schedule-help'">
              <span class="sg-corner" aria-hidden="true" />
              <span v-for="hour in HOURS" :key="'h' + hour" class="sg-hour" aria-hidden="true">{{ hour % 6 === 0 ? pad(hour) : '' }}</span>
              <template v-for="d in dayOrder(forms[item.id].schedule!.length)" :key="'d' + d">
                <span class="sg-day" aria-hidden="true">{{ dayName(d).slice(0, 3) }}</span>
                <button
                  v-for="hour in HOURS"
                  :key="d + ':' + hour"
                  class="sg-cell"
                  type="button"
                  :data-detector="item.id"
                  :data-day="d"
                  :data-hour="hour"
                  :aria-pressed="pressed(cellOf(item.id, d, hour))"
                  :aria-label="cellLabel(item.id, d, hour)"
                  :tabindex="cursor[item.id]?.d === d && cursor[item.id]?.h === hour ? 0 : -1"
                  @pointerdown="startPaint($event, item.id, d, hour)"
                  @click="toggleCell($event, item.id, d, hour)"
                  @keydown="moveCursor($event, item.id, d, hour)"
                />
              </template>
            </div>
            <div class="sg-legend">
              <span><i class="sg-swatch on" aria-hidden="true" />Detect</span>
              <span v-if="hasPartial(item.id)"><i class="sg-swatch part" aria-hidden="true" />Part of the hour</span>
              <span><i class="sg-swatch" aria-hidden="true" />Off</span>
            </div>
            <p :id="'detect-' + item.id + '-schedule-help'" class="hint">Drag across hours to paint or erase. With the keyboard, arrows move and Space switches the hour.</p>
            <p v-if="hasPartial(item.id)" class="hint">Half-filled hours detect for part of the hour; Exact times shows the minutes. Painting a day rounds that day to whole hours.</p>
            <p v-if="hasUnreadable(item.id)" class="hint">Some slots hold text the app cannot read; they are sent back as read until you change them.</p>
            <p class="hint warn-text sg-refusal" aria-live="polite" :data-testid="'detect-' + item.id + '-schedule-refusal'">
              <template v-if="refusal[item.id]"><Icon name="warning" small />{{ refusal[item.id] }}</template>
            </p>
          </div>
        </div>
        <div class="row stack">
          <details class="detect-exact" :open="exactOpen[item.id]" @toggle="exactOpen[item.id] = ($event.target as HTMLDetailsElement).open">
            <summary>Exact times</summary>
            <table class="tbl detect-schedule">
              <thead>
                <tr>
                  <th scope="col">Day</th>
                  <th v-for="(slot, index) in forms[item.id].schedule![0]" :key="index" scope="col">Slot {{ index + 1 }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="(day, d) in forms[item.id].schedule!" :key="d">
                  <th scope="row">{{ DAYS[d] || 'Day ' + (d + 1) }}</th>
                  <td v-for="(slot, index) in day" :key="index">
                    <label class="switchrow"><input v-model="slot.on" type="checkbox" :aria-label="(DAYS[d] || 'Day') + ' slot ' + (index + 1)"><span>On</span></label>
                    <input v-model="slot.start" type="text" inputmode="numeric" pattern="\d\d:\d\d:\d\d" class="detect-time" :aria-label="(DAYS[d] || 'Day') + ' slot ' + (index + 1) + ' start'">
                    <input v-model="slot.end" type="text" inputmode="numeric" pattern="\d\d:\d\d:\d\d" class="detect-time" :aria-label="(DAYS[d] || 'Day') + ' slot ' + (index + 1) + ' end'">
                    <span v-if="unreadable(slot) && untouched(slot)" class="hint detect-raw">Kept as read: {{ slot.raw }}</span>
                  </td>
                </tr>
              </tbody>
            </table>
          </details>
        </div>
        <div class="set-card-f">
          <p class="hint">Saved with Save {{ item.label.toLowerCase() }}.</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.detect-pair { display: flex; gap: var(--s2); }
.detect-pair input { width: 6rem; }
.detect-points { display: grid; gap: var(--s2); padding-top: var(--s3); }
.detect-time { width: 6.5rem; font-family: var(--f-mono); }
.detect-schedule td { vertical-align: top; }
.detect-schedule td > * + * { margin-top: var(--s1); }
.detect-raw { display: block; font-family: var(--f-mono); }
.detect-exact { min-width: 0; }
.detect-exact > summary { display: flex; align-items: center; min-height: var(--hit); font-size: var(--t-13); font-weight: 500; color: var(--ink-2); cursor: pointer; }
.detect-exact > summary:hover { color: var(--ink); }
.detect-exact > summary:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; border-radius: var(--r-sm); }
.detect-exact[open] > summary { margin-bottom: var(--s2); }

.sg { display: grid; grid-template-columns: 2.25rem repeat(24, minmax(0, 1fr)); gap: 3px; width: 100%; touch-action: none; user-select: none; -webkit-user-select: none; }
.sg-hour { font-family: var(--f-mono); font-size: var(--t-12); color: var(--ink-3); white-space: nowrap; overflow: visible; line-height: 1.6; }
.sg-day { font-size: var(--t-12); color: var(--ink-3); align-self: center; }
.sg-cell { position: relative; min-width: 0; height: 18px; padding: 0; border: 0; border-radius: 3px; background: var(--bg-3); cursor: pointer; overflow: hidden; }
.sg-cell:hover { box-shadow: inset 0 0 0 1px var(--line); }
.sg-cell[aria-pressed="true"] { background: var(--accent); }
.sg-cell[aria-pressed="mixed"]::after { content: ""; position: absolute; inset: 50% 0 0 0; background: var(--accent); }
.sg-cell:focus-visible { outline: 2px solid var(--focus); outline-offset: 1px; z-index: 1; }
.sg-legend { display: flex; flex-wrap: wrap; gap: var(--s4); align-items: center; font-size: var(--t-12); color: var(--ink-2); }
.sg-legend > span { display: inline-flex; align-items: center; gap: var(--s2); }
.sg-swatch { position: relative; width: 12px; height: 12px; border-radius: 3px; background: var(--bg-3); box-shadow: inset 0 0 0 1px var(--line-soft); overflow: hidden; }
.sg-swatch.on { background: var(--accent); }
.sg-swatch.part::after { content: ""; position: absolute; inset: 50% 0 0 0; background: var(--accent); }
.sg-refusal:empty { display: none; }
.sg-refusal { display: flex; gap: var(--s2); align-items: flex-start; }
@media (pointer: coarse) { .sg-cell { height: 28px; } }

.rg-box { position: relative; width: 100%; background: var(--bg-3); border-radius: var(--r-md); touch-action: none; }
.rg-img { position: absolute; inset: 0; display: block; width: 100%; height: 100%; object-fit: fill; border-radius: inherit; }
.rg-empty { margin: 0; max-width: none; overflow-wrap: anywhere; }
.rg-svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; pointer-events: none; }
.rg-area { fill: var(--accent); fill-opacity: 0.18; stroke: var(--accent); stroke-width: 2; stroke-linejoin: round; vector-effect: non-scaling-stroke; }
.rg-line { stroke: var(--accent); stroke-width: 3; stroke-linecap: round; vector-effect: non-scaling-stroke; }
.rg-handle { position: absolute; display: grid; place-items: center; width: 44px; height: 44px; margin: -22px 0 0 -22px; padding: 0; border: 0; border-radius: 50%; background: transparent; cursor: grab; touch-action: none; }
.rg-handle::before { content: ""; width: 14px; height: 14px; border-radius: 50%; background: var(--accent); box-shadow: 0 0 0 2px var(--stage-ink); }
.rg-handle:active { cursor: grabbing; }
.rg-handle:focus-visible { outline: 2px solid var(--focus); outline-offset: -10px; }
</style>
