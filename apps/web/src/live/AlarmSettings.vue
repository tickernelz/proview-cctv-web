<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { sessionLost } from '../app/shell'
import { state } from '../app/state'

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
  region: boolean[][] | null
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
  grid: { rows: number; columns: number } | null
  rule_limit: { area: boolean; line: boolean; sensitivity: boolean; show_track: boolean } | null
  combined: Gate
  manual_stop: Gate
}
type Field = { field: string; before: unknown; requested: unknown; after: unknown; matches: boolean }
type Report = { block: string; backup: string; fields: Field[]; reverted: boolean; restored: Field[] }
type Reply = { ok: boolean; status: number; body: any }
type Slot = { on: boolean; start: string; end: string }
type Form = Omit<Summary, 'schedule' | 'push_server'> & { schedule: Slot[][] | null }

const props = defineProps<{ host: HTMLElement }>()

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
const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']

const status = reactive({ text: 'Not loaded yet.', bad: false })
const data = ref<Overview | null>(null)
const forms = reactive<Record<string, Form>>({})
const reports = ref<Report[]>([])
const busy = ref('')
const stopNote = ref('')

const supported = computed(() => (data.value ? data.value.detectors.filter((item) => item.gate.supported && item.state) : []))
const missing = computed(() => (data.value ? data.value.detectors.filter((item) => !item.gate.supported) : []))
const broken = computed(() => (data.value ? data.value.detectors.filter((item) => item.gate.supported && !item.state) : []))

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
  const match = /^([01]) (\d\d:\d\d:\d\d)-(\d\d:\d\d:\d\d)$/.exec(raw)
  return match ? { on: match[1] === '1', start: match[2], end: match[3] } : { on: false, start: '00:00:00', end: '24:00:00' }
}

const fromSlot = (slot: Slot) => (slot.on ? '1 ' : '0 ') + seconds(slot.start) + '-' + seconds(slot.end)
const seconds = (time: string) => (time.length === 5 ? time + ':00' : time)

function formOf(summary: Summary): Form {
  const copy = JSON.parse(JSON.stringify(summary)) as Summary
  return { ...copy, schedule: copy.schedule ? copy.schedule.map((day) => day.map(toSlot)) : null }
}

function fill(overview: Overview) {
  for (const key of Object.keys(forms)) delete forms[key]
  for (const item of overview.detectors) if (item.state) forms[item.id] = formOf(item.state)
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
    const wanted = typeof before[key] === 'number' ? Number(value) : value
    if (!same(wanted, before[key])) patch[key] = wanted
  }
  if (form.schedule && before.schedule) {
    const schedule = form.schedule.map((day) => day.map(fromSlot))
    if (!same(schedule, before.schedule)) patch.schedule = schedule
  }
  if (form.region && !same(form.region, before.region)) patch.region = form.region
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
  if (item.state) forms[item.id] = formOf(item.state)
}

async function stopAlarm() {
  stopNote.value = 'Sending stop…'
  const result = await request('/api/device/detect/alarm/stop' + deviceQuery(), { method: 'POST' })
  stopNote.value = result.ok ? 'The camera acknowledged the stop.' : 'Stop failed: ' + failureText(result)
}

function trackBlocked(item: Detector) {
  if (item.id !== 'track' || !data.value || data.value.combined.supported) return ''
  const human = data.value.detectors.find((entry) => entry.id === 'human')
  return human && human.state && human.state.enable ? 'Human detection is on and the camera does not run both: ' + data.value.combined.evidence : ''
}

function toggleCell(id: string, row: number, column: number) {
  const region = forms[id].region
  if (region) region[row][column] = !region[row][column]
}

function levelOptions(current: number | null) {
  return current === null || LEVELS.some((option) => option.value === current) ? LEVELS : [...LEVELS, { value: current, label: 'Level ' + current }]
}

function open() {
  const page = window as unknown as { openSettings?: (section: string) => void; showSection?: (section: string, focus: boolean) => void }
  if (typeof page.openSettings === 'function') page.openSettings('alarm')
  else if (typeof page.showSection === 'function') page.showSection('alarm', true)
  else location.hash = '#settings/alarm'
}

function openFromNav() {
  const shown = !props.host.hidden
  open()
  if (shown) load()
}

const watcher = new MutationObserver(() => {
  if (!props.host.hidden) load()
})

onMounted(() => watcher.observe(props.host, { attributes: true, attributeFilter: ['hidden'] }))
onBeforeUnmount(() => watcher.disconnect())
</script>

<template>
  <Teleport to=".set-nav">
    <a href="#settings/alarm" data-section="alarm" data-testid="detect-settings-link" @click.prevent="openFromNav">Alarm and detection</a>
  </Teleport>
  <h2 id="h-alarm" class="set-h" tabindex="-1">Alarm and detection</h2>
  <p class="set-sub">
    Detection, schedule and alarm actions as the camera holds them, offered only where the camera reports the ability the phone app checks. Every
    write saves a copy first, reads the block back, and puts the copy back when the camera returns a different value.
  </p>
  <p id="detect-status" :class="['hint', { 'warn-text': status.bad }]" data-testid="detect-status" role="status">{{ status.text }}</p>
  <div v-for="report in reports" :key="report.backup" class="row" data-testid="detect-report">
    <div class="lab">{{ report.block }}</div>
    <div class="ctl-col">
      <p class="hint">{{ (report.reverted ? 'Put back from the saved copy. ' : 'Read back and kept. ') + 'Copy: ' + report.backup }}</p>
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
  <template v-if="data">
    <div v-for="item in supported" :key="item.id" :data-testid="'detect-' + item.id">
      <h3 class="set-h">{{ item.label }}</h3>
      <p class="hint">{{ item.gate.evidence }}</p>
      <div v-if="forms[item.id].enable !== null" class="row">
        <div class="lab">Detection</div>
        <div class="ctl-col">
          <label class="switchrow">
            <input v-model="forms[item.id].enable" type="checkbox" role="switch" :disabled="Boolean(trackBlocked(item))" :data-testid="'detect-' + item.id + '-enable'">
            <span>On</span>
          </label>
          <p v-if="trackBlocked(item)" class="hint warn-text">{{ trackBlocked(item) }}</p>
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
        <label class="lab" :for="'detect-' + item.id + '-sensitivity'">Sensitivity</label>
        <div class="ctl-col">
          <select v-if="item.id === 'track'" :id="'detect-' + item.id + '-sensitivity'" v-model.number="forms[item.id].sensitivity">
            <option v-for="option in TRACK_LEVELS" :key="option.value" :value="option.value">{{ option.label }}</option>
          </select>
          <template v-else-if="item.id === 'human' && !data.rule_limit?.sensitivity">
            <p class="val">{{ item.state!.sensitivity }}</p>
            <p class="hint">Fixed: the camera reports HumanRuleLimit Sensitivity=false, so the app offers no slider.</p>
          </template>
          <input v-else :id="'detect-' + item.id + '-sensitivity'" v-model.number="forms[item.id].sensitivity" type="number" step="1" :min="item.id === 'volume' ? 70 : 0" :max="item.id === 'volume' ? 110 : 100">
        </div>
      </div>
      <div v-if="forms[item.id].show_track !== null" class="row">
        <div class="lab">Track box</div>
        <div class="ctl-col">
          <label class="switchrow">
            <input v-model="forms[item.id].show_track" type="checkbox" role="switch" :disabled="item.id === 'human' && !data.rule_limit?.show_track">
            <span>Draw the track box on people</span>
          </label>
          <p v-if="item.id === 'human' && !data.rule_limit?.show_track" class="hint">Fixed: the camera reports HumanRuleLimit ShowTrack=false.</p>
        </div>
      </div>
      <div v-if="forms[item.id].rule" class="row">
        <div class="lab">Detection rule<div class="hint">Coordinates run 0 to 8192 across and down the picture.</div></div>
        <div class="ctl-col">
          <label class="switchrow"><input v-model="forms[item.id].rule!.enabled" type="checkbox" role="switch"><span>Rule on</span></label>
          <label class="field">
            <span>Rule type</span>
            <select v-model.number="forms[item.id].rule!.rule_type" :data-testid="'detect-' + item.id + '-rule-type'">
              <option :value="1" :disabled="!data.rule_limit?.area">Area{{ data.rule_limit?.area ? '' : ' (not offered)' }}</option>
              <option :value="0" :disabled="!data.rule_limit?.line">Line{{ data.rule_limit?.line ? '' : ' (not offered)' }}</option>
            </select>
          </label>
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
        </div>
      </div>
      <div v-if="forms[item.id].region && data.grid" class="row">
        <div class="lab">Detection area<div class="hint">{{ data.grid.rows }} by {{ data.grid.columns }} grid the camera reports in MotionArea. Filled cells detect.</div></div>
        <div class="ctl-col">
          <div class="detect-grid" role="group" aria-label="Motion detection area" :style="{ gridTemplateColumns: 'repeat(' + data.grid.columns + ', 2rem)' }">
            <template v-for="(row, r) in forms[item.id].region!" :key="r">
              <button
                v-for="(cell, c) in row"
                :key="r + '-' + c"
                type="button"
                :class="['detect-cell', { on: cell }]"
                :aria-pressed="cell"
                :aria-label="'Row ' + (r + 1) + ', column ' + (c + 1)"
                :data-testid="'detect-cell-' + r + '-' + c"
                @click="toggleCell(item.id, r, c)"
              />
            </template>
          </div>
        </div>
      </div>
      <div v-if="LINKS.some((link) => forms[item.id][link.key] !== null)" class="row">
        <div class="lab">On alarm</div>
        <div class="ctl-col">
          <template v-for="link in LINKS" :key="link.key">
            <label v-if="forms[item.id][link.key] !== null" class="switchrow">
              <input v-model="forms[item.id][link.key]" type="checkbox" role="switch" :data-testid="'detect-' + item.id + '-' + link.key">
              <span>{{ link.label }}</span>
            </label>
          </template>
        </div>
      </div>
      <div v-if="forms[item.id].voice_type !== null" class="row">
        <label class="lab" :for="'detect-' + item.id + '-tone'">Alarm tone</label>
        <div class="ctl-col">
          <select v-if="data.tones_gate.supported && data.tones.length" :id="'detect-' + item.id + '-tone'" v-model.number="forms[item.id].voice_type">
            <option v-for="tone in data.tones" :key="tone.id" :value="tone.id">{{ tone.text }}</option>
          </select>
          <p v-else class="hint">Fixed at tone {{ item.state!.voice_type }}: {{ data.tones_gate.evidence }}</p>
          <p class="hint">Only the tone id is written; nothing plays from here.</p>
        </div>
      </div>
      <div v-if="forms[item.id].duration !== null" class="row">
        <label class="lab" :for="'detect-' + item.id + '-duration'">Alert length (s)</label>
        <div class="ctl-col"><input :id="'detect-' + item.id + '-duration'" v-model.number="forms[item.id].duration" type="number" min="1" max="600" step="1"></div>
      </div>
      <div v-if="forms[item.id].remote_call !== null" class="row">
        <div class="lab">One-key call</div>
        <div class="ctl-col">
          <label class="switchrow"><input v-model="forms[item.id].remote_call" type="checkbox" role="switch"><span>On</span></label>
        </div>
      </div>
      <div v-if="forms[item.id].push_interval !== null" class="row">
        <label class="lab" :for="'detect-' + item.id + '-interval'">Push interval (s)<div class="hint">{{ item.state!.enable ? 'Push service on' : 'Push service off' }} · {{ item.state!.push_server || 'no server' }}</div></label>
        <div class="ctl-col">
          <input :id="'detect-' + item.id + '-interval'" v-model.number="forms[item.id].push_interval" type="number" min="30" max="1800" step="1" data-testid="detect-push-interval">
          <p class="hint">The app offers 30 to 1800 seconds. Whether an alarm pushes at all is the message switch on each detector.</p>
        </div>
      </div>
      <div v-if="forms[item.id].schedule" class="row">
        <div class="lab">Schedule<div class="hint">Each day holds {{ forms[item.id].schedule![0]?.length || 0 }} time slots; a slot arms detection when it is on.</div></div>
        <div class="ctl-col">
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
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
      <div class="row">
        <div class="lab" />
        <div class="ctl-col detect-actions">
          <button class="btn btn-fill" type="button" :data-testid="'detect-save-' + item.id" :aria-busy="busy === item.label" @click="save(item)">
            {{ busy === item.label ? 'Saving…' : 'Save ' + item.label.toLowerCase() }}
          </button>
          <button class="btn" type="button" @click="reset(item)">Undo edits</button>
        </div>
      </div>
    </div>
    <h3 class="set-h">Stop sounding alarm</h3>
    <div class="row" data-testid="detect-stop">
      <div class="lab">Manual alarm</div>
      <div class="ctl-col">
        <button class="btn" type="button" :disabled="!data.manual_stop.supported" data-testid="detect-stop-button" @click="stopAlarm">Stop the alarm</button>
        <p class="hint">{{ data.manual_stop.supported ? stopNote || 'Sends only the stop command; nothing here starts a sound.' : 'Not offered: ' + data.manual_stop.evidence }}</p>
      </div>
    </div>
    <template v-if="missing.length || broken.length">
      <h3 class="set-h">Not on this camera</h3>
      <div v-for="item in missing" :key="item.id" class="row" :data-testid="'detect-missing-' + item.id">
        <div class="lab">{{ item.label }}</div>
        <div class="ctl-col"><p class="hint">{{ item.gate.evidence }}</p></div>
      </div>
      <div v-for="item in broken" :key="item.id" class="row">
        <div class="lab">{{ item.label }}</div>
        <div class="ctl-col"><p class="hint warn-text">{{ item.error }}</p></div>
      </div>
    </template>
  </template>
</template>

<style scoped>
.detect-grid { display: grid; gap: var(--s1); }
.detect-cell { width: 2rem; height: 2rem; border-radius: var(--r-sm); border: 1px solid var(--line); background: var(--bg-2); cursor: pointer; }
.detect-cell.on { background: var(--accent-soft); border-color: var(--accent); }
.detect-cell:focus-visible { outline: 2px solid var(--focus); outline-offset: 1px; }
.detect-pair { display: flex; gap: var(--s2); }
.detect-pair input { width: 6rem; }
.detect-points { display: grid; gap: var(--s2); }
.detect-time { width: 6.5rem; font-family: var(--f-mono); }
.detect-schedule td { vertical-align: top; }
.detect-actions { display: flex; gap: var(--s2); flex-wrap: wrap; }
</style>
