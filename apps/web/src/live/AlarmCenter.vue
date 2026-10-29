<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { state as page } from '../app/state'
import { live } from './store'

type Picture = { object?: string; bucket?: string }
type Alarm = { id: string; name?: string; event?: string; time?: string; utcTime?: number; channel?: string; picture?: Picture }
type Listing = { entries?: Alarm[]; total?: number; finished?: boolean; pollSeconds?: number; latencyMs?: number; message?: string }
type Device = { id: string; name?: string; channels?: number }
type Query = { serial: string; hours: number }

const desktop = Boolean((window as unknown as { __TAURI__?: unknown }).__TAURI__)
const watching = ref(false)
const open = computed(() => live.sheet === 'alarms')
const bell = ref<HTMLButtonElement>()
const sheet = ref<HTMLElement>()
const devicePick = ref<HTMLSelectElement>()
const devices = ref<Device[]>([])
const extra = ref<Device | null>(null)
const query = reactive<Query>({ serial: '', hours: 24 })
const view = reactive({
  phase: 'idle' as 'idle' | 'loading' | 'ok' | 'error',
  asked: { serial: '', hours: 24 } as Query,
  entries: [] as Alarm[],
  error: '',
  total: 0,
  finished: true,
  checked: '',
  timing: '',
})
const pictures = reactive({ state: 'unknown' as 'unknown' | boolean })
const missing = reactive(new Set<string>())
const focus = reactive({ id: '', shown: '' })
const alertLabel = ref('Notify me')

let seen = new Set<string>()
let primed = ''
let timer = 0
let pollSeconds = 20
let lastCheck = 0
let ticket = 0
let inflight: Query | null = null
let alertsOn = false

const cameras = computed(() => (extra.value && !devices.value.some((device) => device.id === extra.value!.id) ? [...devices.value, extra.value] : devices.value))
const rows = computed(() => [...view.entries].sort((a, b) => (b.utcTime || 0) - (a.utcTime || 0)))
const nameOf = (serial: string) => cameras.value.find((device) => device.id === serial)?.name || serial
const span = (hours: number) => (hours === 168 ? 'the last 7 days' : 'the last 24 hours')
const plural = (count: number) => count + (count === 1 ? ' alarm' : ' alarms')

const status = computed(() => {
  if (view.phase === 'idle') return 'Not checked yet'
  if (view.phase === 'loading') return 'Checking…'
  if (view.phase === 'error') return view.checked ? 'Last checked ' + view.checked : 'Not checked'
  const shown = view.finished === false ? 'showing ' + rows.value.length + ' of ' + plural(view.total) : plural(view.total || rows.value.length)
  return shown + ' · updated ' + view.checked
})

function clock(when: Date) {
  return String(when.getHours()).padStart(2, '0') + ':' + String(when.getMinutes()).padStart(2, '0')
}

function stamp(when: Date) {
  const two = (value: number) => String(value).padStart(2, '0')
  return when.getFullYear() + '-' + two(when.getMonth() + 1) + '-' + two(when.getDate()) + ' ' + two(when.getHours()) + ':' + two(when.getMinutes()) + ':' + two(when.getSeconds())
}

function when(text?: string) {
  const match = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})$/.exec(text || '')
  if (!match) return text || '-'
  return match[2] + '-' + match[3] + ' ' + match[4] + ':' + match[5]
}

function channelOf(entry: Alarm) {
  const device = cameras.value.find((item) => item.id === view.asked.serial)
  if (!device || (device.channels || 1) < 2) return ''
  return 'channel ' + ((Number(entry.channel) || 0) + 1)
}

function pictureOf(object: string, bucket?: string) {
  return '/api/cloud/alarms/picture?sn=' + encodeURIComponent(view.asked.serial) + '&object=' + encodeURIComponent(object) + '&bucket=' + encodeURIComponent(bucket || '')
}

function srcOf(entry: Alarm) {
  if (pictures.state !== true || missing.has(entry.id)) return ''
  if (!entry.picture || !entry.picture.object) return ''
  return pictureOf(entry.picture.object, entry.picture.bucket)
}

async function probePictures() {
  const serial = view.asked.serial
  if (pictures.state !== 'unknown' || !serial) return
  const candidates = rows.value.filter((entry) => entry.picture && entry.picture.object && !missing.has(entry.id)).slice(0, 3)
  if (!candidates.length) return
  pictures.state = false
  for (const entry of candidates) {
    let reply: Response
    try {
      reply = await fetch(pictureOf(entry.picture!.object!, entry.picture!.bucket), { credentials: 'same-origin' })
    } catch {
      if (view.asked.serial === serial) pictures.state = 'unknown'
      return
    }
    if (view.asked.serial !== serial) return
    if (reply.ok) {
      pictures.state = true
      return
    }
    missing.add(entry.id)
  }
}

function notify(entries: Alarm[]) {
  if (desktop || !alertsOn || typeof Notification === 'undefined') return
  if (Notification.permission !== 'granted' || document.visibilityState === 'visible') return
  const entry = entries.find((item) => !seen.has(item.id))
  if (!entry) return
  try {
    const note = new Notification('Alarm · ' + (entry.name || entry.event || 'camera'), {
      body: (entry.time || '') + ' · ' + nameOf(view.asked.serial),
      tag: 'pvw-alarm-' + entry.id,
    })
    note.onclick = () => window.focus()
  } catch {
    return
  }
}

function fail(message: string) {
  view.phase = 'error'
  view.error = message.charAt(0).toUpperCase() + message.slice(1)
}

async function poll() {
  if (!watching.value || !query.serial) return
  if (document.hidden && !alertsOn) return
  const asked: Query = { serial: query.serial, hours: query.hours }
  if (inflight && inflight.serial === asked.serial && inflight.hours === asked.hours) return
  const mine = ++ticket
  inflight = asked
  clearTimeout(timer)
  timer = 0
  if (view.asked.serial !== asked.serial || view.asked.hours !== asked.hours || view.phase === 'idle') {
    view.asked = asked
    view.entries = []
    view.checked = ''
    view.phase = 'loading'
  }
  const started = performance.now()
  try {
    const until = new Date()
    const since = new Date(until.getTime() - asked.hours * 3600 * 1000)
    const url = '/api/cloud/alarms?sn=' + encodeURIComponent(asked.serial) + '&start=' + encodeURIComponent(stamp(since)) + '&end=' + encodeURIComponent(stamp(until))
    const reply = await fetch(url, { credentials: 'same-origin' })
    const body: Listing = await reply.json().catch(() => ({}))
    if (mine !== ticket) return
    lastCheck = Date.now()
    if (!reply.ok) {
      fail(body.message || (reply.status === 401 ? 'alarms come from the vendor cloud; sign in with the cloud account' : 'the alarm centre answered ' + reply.status))
      return
    }
    pollSeconds = body.pollSeconds || pollSeconds
    const entries = body.entries || []
    const first = primed !== asked.serial
    primed = asked.serial
    if (!first) notify(entries)
    for (const entry of entries) seen.add(entry.id)
    view.entries = entries
    view.total = body.total || entries.length
    view.finished = body.finished !== false
    view.checked = clock(new Date())
    view.timing = 'vendor ' + Math.round(body.latencyMs || 0) + ' ms · here ' + Math.round(performance.now() - started) + ' ms · every ' + pollSeconds + ' s'
    view.error = ''
    view.phase = 'ok'
    if (pictures.state === 'unknown') probePictures()
  } catch (error) {
    if (mine === ticket) lastCheck = Date.now()
    if (mine === ticket) fail('the alarm centre could not be reached: ' + (error instanceof Error ? error.message : String(error)))
  } finally {
    if (mine === ticket) {
      inflight = null
      schedule()
    }
  }
}

function schedule(delay = Math.max(5, pollSeconds) * 1000) {
  clearTimeout(timer)
  timer = 0
  if (!watching.value || !query.serial) return
  if (document.hidden && !alertsOn) return
  timer = window.setTimeout(poll, delay)
}

function resume() {
  if (timer || inflight) return
  const due = lastCheck + Math.max(5, pollSeconds) * 1000 - Date.now()
  if (!lastCheck || due <= 0) poll()
  else schedule(due)
}

function forget() {
  seen = new Set()
  primed = ''
  missing.clear()
  pictures.state = 'unknown'
}

function syncDevices() {
  devices.value = (page.devices || []) as Device[]
  if (cameras.value.some((device) => device.id === query.serial)) return
  const wanted = devices.value.find((device) => device.id === page.selectedId) || devices.value[0]
  query.serial = wanted ? wanted.id : ''
}

async function show() {
  live.sheet = 'alarms'
  syncDevices()
  await nextTick()
  devicePick.value?.focus()
  if (view.phase !== 'ok') poll()
}

function close(returnFocus = true) {
  if (live.sheet === 'alarms') live.sheet = ''
  if (returnFocus) bell.value?.focus()
}

function onKey(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented) return
  event.preventDefault()
  close()
}

function focusAlarm(detail: { id?: string; serial?: string; device?: string } = {}) {
  if (detail.serial) {
    extra.value = { id: detail.serial, name: detail.device || detail.serial }
    query.serial = detail.serial
  }
  focus.id = detail.id || ''
  focus.shown = ''
  show()
}

async function enableAlerts() {
  if (typeof Notification === 'undefined') {
    alertLabel.value = 'Notifications unavailable'
    return
  }
  const permission = await Notification.requestPermission()
  alertsOn = permission === 'granted'
  alertLabel.value = alertsOn ? 'Notifications on' : 'Notifications blocked'
  schedule()
}

function reset() {
  ticket += 1
  inflight = null
  lastCheck = 0
  devices.value = []
  extra.value = null
  query.serial = ''
  view.phase = 'idle'
  view.asked = { serial: '', hours: query.hours }
  view.entries = []
  view.checked = ''
  forget()
}

function syncScreen() {
  if (!page.signedIn) reset()
  const watchView = document.getElementById('watch')
  watching.value = Boolean(watchView) && !watchView!.hidden
  if (watching.value) {
    syncDevices()
    resume()
    return
  }
  close(false)
  clearTimeout(timer)
  timer = 0
}

watch(
  () => query.serial,
  (serial, before) => {
    if (serial === before) return
    forget()
    poll()
  },
)

watch(
  () => query.hours,
  () => poll(),
)

watch(
  () => [rows.value, focus.id] as const,
  async () => {
    if (!focus.id || focus.shown === focus.id) return
    await nextTick()
    const row = sheet.value?.querySelector('[data-id="' + CSS.escape(focus.id) + '"]')
    if (!row) return
    focus.shown = focus.id
    requestAnimationFrame(() => row.scrollIntoView({ block: 'nearest' }))
  },
)

onMounted(() => {
  const screens = new MutationObserver(syncScreen)
  for (const id of ['watch', 'signin']) {
    const screen = document.getElementById(id)
    if (screen) screens.observe(screen, { attributes: true, attributeFilter: ['hidden'] })
  }
  window.addEventListener('proview:devices', () => {
    if (!watching.value) return
    syncDevices()
    resume()
  })
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible' && watching.value) resume()
    else schedule()
  })
  window.addEventListener('desktop-alarm', (event) => focusAlarm((event as CustomEvent).detail || {}))
  syncScreen()
})
</script>

<template>
  <button
    ref="bell"
    class="iconbtn alarm-bell"
    type="button"
    data-testid="alarm-bell"
    :hidden="!watching"
    aria-haspopup="dialog"
    aria-controls="alarm-sheet"
    :aria-expanded="open"
    aria-label="Alarm messages"
    title="Alarm messages"
    @click="open ? close() : show()"
  >
    <Icon name="bell" />
    <span class="badge" data-testid="alarm-badge" :hidden="!rows.length">{{ rows.length }}</span>
  </button>
  <Teleport to="body">
    <section
      id="alarm-sheet"
      ref="sheet"
      class="alarm-sheet"
      data-testid="alarm-center"
      role="dialog"
      aria-labelledby="alarm-title"
      :hidden="!open"
      @keydown="onKey"
    >
      <div class="alarm-head">
        <h2 id="alarm-title">Alarm messages</h2>
        <button class="iconbtn" type="button" data-close data-testid="alarm-close" aria-label="Close" @click="close()">
          <Icon name="x" />
        </button>
      </div>
      <div class="alarm-bar">
        <select ref="devicePick" v-model="query.serial" data-device data-testid="alarm-camera" aria-label="Camera" :disabled="!cameras.length">
          <option v-for="device in cameras" :key="device.id" :value="device.id">{{ device.name || device.id }}</option>
        </select>
        <select v-model.number="query.hours" data-hours data-testid="alarm-window" aria-label="Window">
          <option :value="24">Last 24 hours</option>
          <option :value="168">Last 7 days</option>
        </select>
      </div>
      <p v-if="view.phase === 'error'" class="err alarm-err" data-testid="alarm-error" role="alert">
        <Icon name="warning" small /><span>{{ view.error }}</span>
      </p>
      <div class="alarm-list" data-list role="list" :aria-busy="view.phase === 'loading'">
        <p v-if="view.phase === 'loading' && !rows.length" data-testid="alarm-loading">Loading alarms for {{ nameOf(view.asked.serial) }}…</p>
        <p v-else-if="view.phase === 'ok' && !rows.length" data-testid="alarm-empty">No alarms for {{ nameOf(view.asked.serial) }} in {{ span(view.asked.hours) }}.</p>
        <div
          v-for="entry in rows"
          :key="entry.id"
          :class="['alarm-row', { focus: focus.id && focus.id === entry.id }]"
          data-testid="alarm-row"
          :data-id="entry.id"
          role="listitem"
        >
          <img v-if="srcOf(entry)" class="thumb" alt="" loading="lazy" :src="srcOf(entry)" @error="missing.add(entry.id)">
          <div v-else class="thumb"><Icon name="no-picture" small /></div>
          <div>
            <div class="what">{{ entry.name || entry.event || 'Alarm' }}</div>
            <div class="when"><time :datetime="entry.time">{{ when(entry.time) }}</time></div>
            <div v-if="channelOf(entry)" class="chan">{{ channelOf(entry) }}</div>
          </div>
        </div>
      </div>
      <div class="alarm-foot">
        <span data-status data-testid="alarm-status" :title="view.timing" role="status">{{ status }}</span>
        <button v-if="!desktop" class="btn btn-line" type="button" data-alerts data-testid="alarm-notify" @click="enableAlerts">{{ alertLabel }}</button>
      </div>
    </section>
  </Teleport>
</template>
