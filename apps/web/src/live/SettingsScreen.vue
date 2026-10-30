<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Kbd } from '../components/ui/kbd'
import { api } from '../lib/api'
import { leaveSettings, setMode, setPref, showSection, signOut } from '../app/shell'
import { toggleWhiteLight } from '../app/controls'
import { live, menuOpen, type Prefs } from './store'
import AccountPanel from './AccountPanel.vue'
import AlarmSettings from './AlarmSettings.vue'
import DeviceSettings from './DeviceSettings.vue'
import ImageSettings from './ImageSettings.vue'
import SystemAbout from './SystemAbout.vue'
import { replaceHash, settingsHash } from '../shell/router'
import { openDevices, reloadSection } from '../settings/ui'

type Field = {
  key: string
  label: string
  hint?: string
  group: string
  kind: 'text' | 'number' | 'secret' | 'select' | 'toggle'
  value?: string
  default?: string
  set?: boolean
  secret?: boolean
  restart?: boolean
  min?: number | null
  max?: number | null
  options?: string[]
}
type View = { fields: Field[]; exists?: boolean; path?: string }
type Check = { text: string; ok: boolean | null }

const GROUPS = [
  {
    id: 'camera',
    name: 'Camera',
    items: [
      { id: 'camera', name: 'This camera', icon: 'cctv', testid: 'camera-settings-link' },
      { id: 'device', name: 'Video & storage', icon: 'video', testid: 'device-settings-link' },
      { id: 'image', name: 'Image, light & sound', icon: 'sun', testid: 'image-settings-link' },
      { id: 'alarm', name: 'Detection & alarms', icon: 'scan', testid: 'detect-settings-link' },
      { id: 'system', name: 'System', icon: 'cpu', testid: 'system-link' },
    ],
  },
  {
    id: 'app',
    name: 'App',
    items: [
      { id: 'playback', name: 'Playback', icon: 'play', testid: 'playback-settings-link' },
      { id: 'config', name: 'Configuration', icon: 'sliders', testid: 'config-settings-link' },
    ],
  },
  {
    id: 'account',
    name: 'Account & server',
    items: [
      { id: 'account', name: 'Account', icon: 'user', testid: 'account-settings-link' },
      { id: 'connection', name: 'Connection', icon: 'cloud', testid: 'connection-settings-link' },
      { id: 'server', name: 'Server', icon: 'activity', testid: 'server-settings-link' },
    ],
  },
]
const NOTE_SECTIONS = new Set(['device', 'image', 'alarm', 'system'])
const LENSES = [
  ['pip', 'Picture in picture'],
  ['combined', 'Both lenses'],
  ['0', 'Top lens'],
  ['1', 'Bottom lens'],
]
const THEMES = [
  ['system', 'System'],
  ['dark', 'Dark'],
  ['light', 'Light'],
]
const KEYS: [string[], string][] = [
  [['1', '9'], 'Switch camera'],
  [['G'], 'Single or grid'],
  [['F'], 'Full screen'],
  [['M'], 'Sound on or off'],
  [['T'], 'Talk, held'],
  [['R'], 'Record'],
  [['S'], 'Snapshot'],
  [['L'], 'Lens'],
  [['Q'], 'HD or SD'],
  [['V'], 'Move pad'],
  [['Space'], 'Stop moving'],
  [[','], 'Settings'],
  [['Esc'], 'Close or go back'],
  [['?'], 'Keyboard list'],
]
const OPTION_LABELS: Record<string, string> = {
  proview: 'ProView camera',
  cloud: 'Cloud account',
  local: 'This network',
  fixture: 'Built-in sample data',
}
const fieldId = (key: string) => 'setting-' + key.replace(/[^a-z0-9]+/gi, '-').toLowerCase()
const MONO_KEY = /addr|host|base|listen|dir|rtsp|onvif|ffmpeg|url|port|profile/i

const s = live.settings
const nav = ref<HTMLElement>()
const body = ref<HTMLElement>()
const back = ref<HTMLButtonElement>()
const saveButton = ref<HTMLButtonElement>()
const checksButton = ref<HTMLButtonElement>()

const view = ref<View | null>(null)
const values = reactive<Record<string, string>>({})
const initial = reactive<Record<string, string>>({})
const status = reactive({ text: 'Not loaded yet.', bad: false })
const saving = ref(false)
const restarting = ref(false)
const pendingRestart = ref(0)
const barError = ref('')
const blocked = ref(false)
const checking = ref(false)
const checks = reactive<Record<string, Check>>({
  health: { text: 'Not checked', ok: null },
  account: { text: 'Not checked', ok: null },
  probe: { text: 'Not checked', ok: null },
  ticket: { text: 'Not checked', ok: null },
})

const inDesktop = Boolean((window as unknown as { __TAURI__?: unknown }).__TAURI__)
const groups = computed(() => {
  const out: { name: string; fields: Field[] }[] = []
  for (const field of view.value ? view.value.fields : []) {
    if (field.group === 'Desktop' && !inDesktop) continue
    let group = out.find((entry) => entry.name === field.group)
    if (!group) out.push((group = { name: field.group, fields: [] }))
    group.fields.push(field)
  }
  return out
})
const fieldByKey = computed(() => new Map((view.value ? view.value.fields : []).map((field) => [field.key, field])))
const payload = computed(() => {
  const out: Record<string, string> = {}
  for (const [key, value] of Object.entries(values)) {
    const field = fieldByKey.value.get(key)
    if (!field) continue
    if (field.secret) {
      if (value !== '') out[key] = value
    } else if (value !== initial[key]) out[key] = value
  }
  return out
})
const changed = computed(() => Object.keys(payload.value).length)
const restartKeys = computed(() => new Set((view.value ? view.value.fields : []).filter((field) => field.restart).map((field) => field.key)))
const desktop = () => {
  const control = (window as unknown as { __PROVIEW_DESKTOP__?: { token?: unknown } }).__PROVIEW_DESKTOP__
  return control && typeof control.token === 'string' && control.token ? (control as { token: string }) : null
}
const canRestart = ref(Boolean(desktop()))
const plural = (n: number, word: string) => n + ' ' + word + (n === 1 ? '' : 's')
const barText = computed(() => {
  if (blocked.value && changed.value) return 'Save or discard ' + plural(changed.value, 'changed field') + ' before leaving.'
  if (changed.value) return 'Unsaved changes · ' + plural(changed.value, 'field')
  if (pendingRestart.value) return 'Saved. ' + (canRestart.value ? 'Restart the app' : 'Restart the server process') + ' to apply ' + plural(pendingRestart.value, 'field')
  return 'Everything saved'
})
const barTone = computed(() => (barError.value || (blocked.value && changed.value) ? 'bad' : changed.value ? 'dirty' : ''))
const barOwn = computed(() => !NOTE_SECTIONS.has(s.section) || changed.value > 0 || Boolean(barError.value) || pendingRestart.value > 0)
const sectionName = computed(() => GROUPS.flatMap((group) => group.items).find((item) => item.id === s.section)?.name || 'Settings')
const backLabel = computed(() => (s.signedIn ? 'Live' : 'Sign in'))
const modeLine = computed(() => {
  const mode = s.mode
  if (!mode) return 'The server did not report a provider mode.'
  return 'Answering: ' + (mode.source || 'source unknown') + (s.signedIn ? '' : '. Sign in to switch the provider.')
})
const modeNotice = computed(() => {
  const notice = s.mode && s.mode.notice
  if (!notice) return ''
  return (notice.message || 'this network is not reachable, so the other provider answered.') + (notice.to ? ' Answering provider: ' + notice.to + '.' : '')
})
const capabilityRows = computed(() => s.capabilities)

function sentence(text: string) {
  return text.charAt(0).toUpperCase() + text.slice(1)
}
function message(error: unknown) {
  return sentence(error instanceof Error ? error.message : String(error))
}

function fill(next: View, keepTyped: boolean) {
  const typed: Record<string, string> = {}
  if (keepTyped) Object.assign(typed, payload.value)
  for (const key of Object.keys(values)) delete values[key]
  for (const key of Object.keys(initial)) delete initial[key]
  for (const field of next.fields) {
    const value = field.kind === 'toggle' ? ((field.value || field.default || 'off') === 'on' ? 'on' : 'off')
      : field.kind === 'secret' ? ''
      : field.kind === 'select' ? field.value || (field.options && field.options[0]) || ''
      : field.value || ''
    initial[field.key] = value
    values[field.key] = value
  }
  view.value = next
  for (const [key, value] of Object.entries(typed)) if (key in values) values[key] = value
}

const credentials = () => {
  const control = desktop()
  return control ? { headers: { 'content-type': 'application/json', 'x-proview-desktop-token': control.token } } : {}
}

let loads = 0
async function load() {
  const mine = ++loads
  status.text = 'Loading…'
  status.bad = false
  try {
    const next = await api<View>('/api/settings', credentials())
    if (mine !== loads) return
    fill(next, true)
    status.text = next.exists
      ? 'Saved values come from ' + next.path + '.'
      : next.path
        ? 'Nothing saved yet. Saving writes ' + next.path + '.'
        : 'No data folder is set, so saved values apply to this run only.'
    if (s.focusKey) {
      const wanted = s.focusKey
      s.focusKey = ''
      const key = fieldByKey.value.has(wanted) ? wanted : [...fieldByKey.value.keys()].find((name) => /DEVICE_PASS/i.test(name))
      await nextTick()
      if (key) (document.querySelector('#settings-fields [data-key="' + key + '"]') as HTMLElement | null)?.focus()
    }
  } catch (error) {
    if (mine !== loads) return
    if ((error as { status?: number }).status === 401) {
      view.value = null
      status.text = 'Sign in to read and change the settings.'
      status.bad = false
      return
    }
    status.text = 'Could not read the settings: ' + message(error)
    status.bad = true
  }
}

async function save() {
  const out = payload.value
  const keys = Object.keys(out)
  if (!keys.length || saving.value) return
  saving.value = true
  barError.value = ''
  try {
    const next = await api<View>('/api/settings', { method: 'POST', ...credentials(), body: JSON.stringify({ values: out }) })
    pendingRestart.value += keys.filter((key) => restartKeys.value.has(key)).length
    fill(next, false)
    blocked.value = false
    status.text = 'Saved ' + plural(keys.length, 'field') + ' to ' + (next.path || 'this run') + '.'
    status.bad = false
  } catch (error) {
    if ((error as { status?: number }).status === 401) {
      view.value = null
      status.text = 'Sign in to change the settings.'
      status.bad = false
      return
    }
    barError.value = 'Not saved: ' + message(error)
    status.text = 'Could not save: ' + message(error)
    status.bad = true
    await nextTick()
    saveButton.value?.focus()
  } finally {
    saving.value = false
  }
}

function discard() {
  if (view.value) fill(view.value, false)
  blocked.value = false
  barError.value = ''
}

async function restart() {
  const control = desktop()
  if (!control || restarting.value) return
  restarting.value = true
  barError.value = ''
  try {
    await api('/api/desktop/restart', {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-proview-desktop-token': control.token },
    })
  } catch (error) {
    barError.value = 'Could not restart: ' + message(error)
    status.text = 'Could not restart: ' + message(error)
    status.bad = true
    restarting.value = false
  }
}

async function goBack() {
  if (changed.value) {
    blocked.value = true
    await nextTick()
    saveButton.value?.focus()
    return
  }
  blocked.value = false
  leaveSettings()
}

function show(section: string, focus: boolean) {
  showSection(section, focus)
}

function pick(section: string) {
  if (s.section === section) {
    reloadSection(section)
    if (section === 'config') load()
  }
  show(section, true)
}

function toList() {
  const link = nav.value?.querySelector<HTMLAnchorElement>('a[aria-current="page"]')
  link?.focus()
  link?.scrollIntoView({ block: 'nearest' })
}

function onNavKey(event: KeyboardEvent) {
  const step = ({ ArrowDown: 1, ArrowUp: -1, ArrowRight: 1, ArrowLeft: -1 } as Record<string, number>)[event.key]
  if (!nav.value) return
  const links = [...nav.value.querySelectorAll<HTMLAnchorElement>('a[data-section]')]
  const at = links.indexOf(event.target as HTMLAnchorElement)
  if (at < 0) return
  let next: HTMLAnchorElement | undefined
  if (step) next = links[(at + step + links.length) % links.length]
  else if (event.key === 'Home') next = links[0]
  else if (event.key === 'End') next = links[links.length - 1]
  if (!next) return
  event.preventDefault()
  show(next.dataset.section || 'playback', false)
  next.focus()
}

const modeGroup = ref<HTMLElement>()
async function chooseMode(value: string) {
  await setMode(value)
  for (const radio of modeGroup.value ? modeGroup.value.querySelectorAll<HTMLInputElement>('input[name="mode"]') : []) {
    radio.checked = Boolean(s.mode) && radio.value === s.mode!.mode
  }
}

function pref<K extends keyof Prefs>(name: K, value: Prefs[K]) {
  setPref(name, value)
}

async function runChecks() {
  if (checking.value) return
  checking.value = true
  for (const key of Object.keys(checks)) Object.assign(checks[key], { text: 'Checking…', ok: null })
  const fail = (error: unknown) => {
    const failure = error as { status?: number; message?: string }
    return { text: sentence(failure.message || String(error)) + (failure.status ? ' (HTTP ' + failure.status + ')' : ''), ok: false }
  }
  try {
    const health = await api<{ ok?: boolean; service?: string }>('/api/health')
    Object.assign(checks.health, health.ok ? { text: 'Up · ' + health.service, ok: true } : { text: 'Unexpected answer: ' + JSON.stringify(health), ok: false })
  } catch (error) {
    Object.assign(checks.health, fail(error))
  }
  try {
    const me = await api<{ account?: string }>('/api/me')
    Object.assign(checks.account, { text: me.account || 'signed in', ok: true })
  } catch (error) {
    Object.assign(checks.account, fail(error))
  }
  try {
    const probe = await api<{ host?: string; video_codec?: string }>('/api/device/rtsp', { method: 'POST' })
    Object.assign(checks.probe, { text: 'Answered · ' + probe.host + ' · ' + (probe.video_codec || 'codec unknown'), ok: true })
  } catch (error) {
    Object.assign(checks.probe, fail(error))
  }
  const camera = s.camera
  if (!camera) {
    Object.assign(checks.ticket, { text: 'No camera is open.', ok: null })
  } else {
    try {
      const ticket = await api<{ kind?: string; url?: string }>(
        '/api/stream?device=' + encodeURIComponent(camera.id) + '&channel=' + encodeURIComponent(camera.channel) + '&quality=' + encodeURIComponent(s.prefs.quality),
      )
      Object.assign(checks.ticket, { text: (ticket.kind || '').toUpperCase() + ' · ' + ticket.url, ok: true })
    } catch (error) {
      Object.assign(checks.ticket, fail(error))
    }
  }
  checking.value = false
}

function typing(target: EventTarget | null) {
  const node = target as HTMLElement | null
  return Boolean(node && node.closest && node.closest('input, textarea, select, [contenteditable="true"]'))
}

function onKey(event: KeyboardEvent) {
  if (!s.shown || event.defaultPrevented || menuOpen() || live.screen) return
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
    event.preventDefault()
    save()
  } else if (event.key === 'Escape' && !typing(event.target) && !changed.value) {
    event.preventDefault()
    goBack()
  }
}

watch(() => s.section, (section) => {
  if (body.value) body.value.scrollTop = 0
  if (s.shown) replaceHash(settingsHash(section))
})
watch(() => s.focusHeading, async () => {
  await nextTick()
  ;(document.getElementById('h-' + s.section) as HTMLElement | null)?.focus()
})
watch(() => s.focusBack, async () => {
  await nextTick()
  back.value?.focus()
})
watch(payload, () => {
  barError.value = ''
})
watch(() => s.load, () => {
  canRestart.value = Boolean(desktop())
  load()
})
watch(() => s.reset, () => {
  if (view.value) fill(view.value, false)
  blocked.value = false
  barError.value = ''
})
watch(changed, (count) => {
  if (!count) blocked.value = false
})

onMounted(() => {
  ;(window as unknown as { loadSettings: () => Promise<void> }).loadSettings = load
  document.addEventListener('keydown', onKey)
})
onBeforeUnmount(() => document.removeEventListener('keydown', onKey))

</script>

<template>
  <div class="set">
    <nav ref="nav" class="set-nav" aria-label="Settings sections" @keydown="onNavKey">
      <div class="set-top">
        <button id="settings-back" ref="back" class="set-back" data-testid="settings-back" type="button" @click="goBack"><Icon name="arrow-left" small /><span id="settings-back-label">{{ backLabel }}</span></button>
        <h1 id="settings-title" class="set-title" tabindex="-1">Settings</h1>
      </div>
      <template v-for="group in GROUPS" :key="group.id">
        <p :id="'set-nav-' + group.id" class="set-nav-g">{{ group.name }}</p>
        <p v-if="group.id === 'camera'" class="set-nav-cam" data-testid="settings-camera">
          <span :class="['dot', { off: live.top.dotOff || !live.top.dotShown }]" /><span class="name">{{ s.camera ? s.camera.name : 'No camera' }}</span><span class="sr">, {{ live.top.dotShown ? live.top.dotLabel : 'status not read' }}</span>
        </p>
        <a
          v-for="item in group.items"
          :key="item.id"
          :href="settingsHash(item.id)"
          :data-section="item.id"
          :data-testid="item.testid"
          :aria-current="s.section === item.id ? 'page' : undefined"
          :tabindex="s.section === item.id ? 0 : -1"
          @click.prevent="pick(item.id)"
        ><Icon :name="item.icon" small />{{ item.name }}</a>
      </template>
      <div class="set-nav-foot">
        <button class="btn" type="button" data-testid="settings-cameras" :disabled="!s.signedIn" @click="openDevices"><Icon name="cctv" small />Manage cameras</button>
        <p v-if="!s.signedIn" class="hint">Sign in to manage the cameras on the account.</p>
      </div>
    </nav>
    <div id="set-body" ref="body" class="set-body">
      <section data-section="playback" aria-labelledby="h-playback" :hidden="s.section !== 'playback'">
        <button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button>
        <header class="set-head">
          <div>
            <h2 id="h-playback" class="set-h" tabindex="-1">Playback</h2>
            <p class="set-sub">How a camera opens and looks. Saved in this browser and applied at once; camera settings are under Camera.</p>
          </div>
        </header>
        <div class="set-cols">
          <div class="set-col">
            <div class="set-card">
              <div class="set-card-h"><div><h3 class="set-card-t">Appearance</h3></div></div>
              <div class="row">
                <div id="lab-theme" class="lab">Theme<span class="hint">System follows the computer setting.</span></div>
                <div class="ctl-col">
                  <div class="seg" role="radiogroup" aria-labelledby="lab-theme">
                    <label v-for="[value, name] in THEMES" :key="value"><input type="radio" name="theme" :value="value" :checked="s.prefs.theme === value" @change="pref('theme', value)">{{ name }}</label>
                  </div>
                </div>
              </div>
              <div class="row">
                <div class="lab">Caption in full screen<span class="hint">Camera name and clock at the bottom of the picture. The window keeps them under the picture either way.</span></div>
                <div class="ctl-col"><label class="switchrow"><input id="set-osd" type="checkbox" role="switch" aria-label="Camera name and clock in full screen" :checked="s.prefs.osd" @change="pref('osd', ($event.target as HTMLInputElement).checked)"></label></div>
              </div>
              <div class="row">
                <div class="lab">Upside down<span class="hint">Turn the picture 180° for a camera mounted on the ceiling.</span></div>
                <div class="ctl-col"><label class="switchrow"><input id="set-flip" type="checkbox" role="switch" data-testid="set-flip" aria-label="Turn the picture 180° (a camera mounted upside down)" :checked="s.prefs.flip" @change="pref('flip', ($event.target as HTMLInputElement).checked)"></label></div>
              </div>
            </div>
            <div class="set-card">
              <div class="set-card-h"><div><h3 class="set-card-t">Keyboard shortcuts</h3><p class="set-card-d">Letter keys do nothing while you type in a field.</p></div></div>
              <dl class="keylist">
                <div v-for="[combo, meaning] in KEYS" :key="meaning">
                  <dt>{{ meaning }}</dt>
                  <dd>
                    <template v-if="combo.length === 2"><Kbd>{{ combo[0] }}</Kbd>–<Kbd>{{ combo[1] }}</Kbd></template>
                    <Kbd v-else>{{ combo[0] }}</Kbd>
                  </dd>
                </div>
              </dl>
            </div>
          </div>
          <div class="set-col">
            <div class="set-card">
              <div class="set-card-h"><div><h3 class="set-card-t">Playback</h3></div></div>
              <div class="row">
                <div id="lab-quality" class="lab">Stream quality<span class="hint">SD is the sub stream, lighter on the network.</span></div>
                <div class="ctl-col">
                  <div class="seg" role="radiogroup" aria-labelledby="lab-quality">
                    <label><input type="radio" name="quality" value="main" aria-label="HD — the main stream" :checked="s.prefs.quality === 'main'" @change="pref('quality', 'main')">HD</label>
                    <label><input type="radio" name="quality" value="sub" aria-label="SD — the sub stream, lighter on the network" :checked="s.prefs.quality === 'sub'" @change="pref('quality', 'sub')">SD</label>
                  </div>
                </div>
              </div>
              <div class="row">
                <div id="lab-lens" class="lab">Lens on open<span class="hint">Only cameras with two lenses use this.</span></div>
                <div class="ctl-col">
                  <div class="seg" role="radiogroup" aria-labelledby="lab-lens">
                    <label v-for="[value, name] in LENSES" :key="value"><input type="radio" name="lens" :value="value" :checked="s.prefs.lens === value" @change="pref('lens', value)">{{ name }}</label>
                  </div>
                </div>
              </div>
              <div class="row">
                <div class="lab">Start muted<span class="hint">Sound stays off until you turn it on with M or the Sound button.</span></div>
                <div class="ctl-col"><label class="switchrow"><input id="set-muted" type="checkbox" role="switch" aria-label="Start muted" :checked="s.prefs.muted" @change="pref('muted', ($event.target as HTMLInputElement).checked)"></label></div>
              </div>
            </div>
            <p class="set-aside"><Icon name="info" small /><span>These choices stay in this browser. Another browser or the desktop app keeps its own.</span></p>
          </div>
        </div>
      </section>

      <section data-section="camera" aria-labelledby="h-camera" :hidden="s.section !== 'camera'">
        <button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button>
        <header class="set-head">
          <div>
            <h2 id="h-camera" class="set-h" tabindex="-1">This camera</h2>
            <p id="camera-sub" class="set-sub">What the device says about itself, as the server last read it. An unavailable row means the camera returned no evidence, not that the app forgot to look.</p>
          </div>
        </header>
        <div v-if="!s.camera" id="camera-none" class="set-card">
          <div class="row"><div class="lab">No camera</div><div class="ctl-col"><p class="val">Sign in and pick a camera first.</p></div></div>
        </div>
        <template v-else>
          <div id="camera-rows" class="set-cols">
            <div class="set-col">
              <div class="set-card">
                <div class="set-card-h"><div><h3 class="set-card-t">Identity</h3></div></div>
                <div class="row"><div class="lab">Name</div><div class="ctl-col"><p id="cam-name" class="val">{{ s.camera.name }}</p></div></div>
                <div class="row">
                  <div class="lab">Device id<span class="hint">The same id the phone app shows.</span></div>
                  <div class="ctl-col"><p class="codes"><span id="cam-id" class="code">{{ s.camera.id }}</span><span id="cam-channel" class="code">channel {{ s.camera.channel }}</span><span id="cam-model" class="code">{{ s.camera.model }}</span></p></div>
                </div>
                <div class="row"><div class="lab">Connection<span id="cam-stream" class="hint">{{ s.camera.stream }}</span></div><div class="ctl-col"><p id="cam-connection" class="val">{{ s.camera.connection }}</p></div></div>
                <div class="row stack">
                  <div class="lab">Lens layout</div>
                  <p id="lens-source" class="hint" :title="s.lensTitle || undefined">{{ s.lensNote }}</p>
                  <p v-if="!s.lensRolesKnown" id="lens-roles-note" class="hint">Which half moves comes from the device report. Until the server sends it, the lenses are named by position, not by role.</p>
                </div>
              </div>
            </div>
            <div class="set-col">
              <div class="set-card">
                <div class="set-card-h"><div><h3 class="set-card-t">White light</h3><p class="set-card-d">Switches the spotlight between its modes on the camera.</p></div></div>
                <div class="row">
                  <div class="lab">Spotlight<span v-if="!s.light.ok && s.light.why" class="hint" data-testid="light-why">{{ s.light.why }}</span></div>
                  <div class="ctl-col">
                    <button id="light" class="btn" data-testid="ctl-light" type="button" :disabled="!s.light.ok" :title="s.light.why || undefined" @click="toggleWhiteLight">Switch the white light</button>
                    <p id="light-status" class="hint" data-testid="light-status" role="status">{{ s.lightStatus }}</p>
                  </div>
                </div>
              </div>
              <div class="set-card">
                <div class="set-card-h"><div><h3 class="set-card-t">Cameras on this account</h3><p class="set-card-d">Rename, group, share with permissions or by QR code, transfer and remove.</p></div></div>
                <div class="row">
                  <div class="lab">Manage cameras<span v-if="!s.signedIn" class="hint">Sign in to manage the cameras on the account.</span></div>
                  <div class="ctl-col"><button class="btn" type="button" data-testid="camera-manage" :disabled="!s.signedIn" @click="openDevices"><Icon name="cctv" small />Open the camera list</button></div>
                </div>
              </div>
            </div>
          </div>
          <div class="set-stack">
            <div class="set-card">
              <div class="set-card-h"><div><h3 class="set-card-t">What it can do</h3><p id="device-note" class="set-card-d">{{ s.controlNote }}</p></div></div>
              <div class="row stack">
                <table class="tbl caps">
                  <caption class="sr">Device capability report from the server</caption>
                  <thead><tr><th scope="col">Feature</th><th scope="col">Lens</th><th scope="col">Backend</th><th scope="col">Reported</th><th scope="col">Evidence</th></tr></thead>
                  <tbody id="capability-rows">
                    <tr v-for="(row, index) in capabilityRows" :key="index">
                      <td class="feature" data-label="Feature">{{ row[0] }}</td>
                      <td class="lens" data-label="Lens">{{ row[1] }}</td>
                      <td class="backend" data-label="Backend">{{ row[2] }}</td>
                      <td class="state" data-label="Reported"><span :class="['pill', row[3] === 'supported' ? 'ok' : 'no']"><Icon :name="row[3] === 'supported' ? 'check' : 'x'" small />{{ row[3] === 'supported' ? 'supported' : row[3] || 'unavailable' }}</span></td>
                      <td class="evidence" data-label="Evidence">{{ row[4] }}</td>
                    </tr>
                  </tbody>
                </table>
                <p v-if="!capabilityRows.length" class="hint">No capability report yet.</p>
              </div>
            </div>
          </div>
        </template>
      </section>

      <section data-section="connection" aria-labelledby="h-connection" :hidden="s.section !== 'connection'">
        <button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button>
        <header class="set-head">
          <div>
            <h2 id="h-connection" class="set-h" tabindex="-1">Connection</h2>
            <p class="set-sub">Which provider answers: the vendor cloud, or the camera on this network. Switching re-opens the camera list through the new provider.</p>
          </div>
        </header>
        <div class="set-stack set-narrow">
          <div class="set-card">
            <div class="set-card-h"><div><h3 class="set-card-t">Provider</h3><p id="mode-status" class="set-card-d" role="status">{{ modeLine }}</p></div></div>
            <div class="row">
              <div id="lab-mode" class="lab">Answering provider<span v-if="!s.signedIn" class="hint">Sign in to switch the provider.</span></div>
              <div ref="modeGroup" class="ctl-col">
                <div class="seg" role="radiogroup" aria-labelledby="lab-mode">
                  <label v-for="[value, name] in [['cloud', 'Cloud account'], ['local', 'This network']]" :key="value"><input type="radio" name="mode" :value="value" :disabled="!s.signedIn" :checked="Boolean(s.mode) && s.mode!.mode === value" @change="chooseMode(value)">{{ name }}</label>
                </div>
              </div>
            </div>
            <div v-if="s.modeError || modeNotice" class="row stack">
              <p v-if="s.modeError" class="err" data-testid="mode-error" role="alert"><Icon name="warning" small /><span>{{ s.modeError }}</span></p>
              <p v-if="modeNotice" id="mode-notice" class="notice mode-notice"><Icon name="info" small /><span>{{ modeNotice }}</span></p>
            </div>
          </div>
          <div v-if="s.signedIn" id="account-row" class="set-card">
            <div class="set-card-h"><div><h3 class="set-card-t">Session</h3></div></div>
            <div class="row">
              <div class="lab">Signed in as<span id="account" class="hint">{{ s.account || 'Signed in' }}</span></div>
              <div class="ctl-col"><button id="signout" class="btn" type="button" data-testid="settings-signout" @click="signOut">Sign out</button></div>
            </div>
          </div>
        </div>
      </section>

      <section data-section="config" aria-labelledby="h-config" :hidden="s.section !== 'config'">
        <button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button>
        <header class="set-head">
          <div>
            <h2 id="h-config" class="set-h" tabindex="-1">Configuration</h2>
            <p class="set-sub">Server settings, stored on this machine in <code>settings.json</code>. Saving sends only the fields you changed; secrets are never read back. A field that needs a restart carries a Restart tag.</p>
            <p id="settings-status" :class="['set-sub', 'settings-status', { 'warn-text': status.bad }]" data-testid="settings-status" role="status">{{ status.text }}</p>
          </div>
        </header>
        <div id="settings-fields" class="set-stack set-narrow" data-testid="settings-fields">
          <div v-for="group in groups" :key="group.name" class="set-card">
            <div class="set-card-h"><div><h3 class="set-card-t">{{ group.name }}</h3></div></div>
            <div v-for="field in group.fields" :key="field.key" class="row" :data-changed="field.key in payload ? 'true' : undefined">
              <template v-if="field.kind === 'toggle'">
                <div class="lab">
                  {{ field.label }}
                  <span v-if="field.restart" class="tag-restart"><Icon name="refresh" small />Restart</span>
                  <span v-if="field.hint" class="hint">{{ field.hint }}</span>
                </div>
                <div class="ctl-col">
                  <label class="switchrow">
                    <input
                      :id="fieldId(field.key)"
                      type="checkbox"
                      role="switch"
                      :aria-label="field.label"
                      :data-key="field.key"
                      :data-testid="'setting-' + field.key"
                      data-secret="false"
                      :checked="values[field.key] === 'on'"
                      @change="values[field.key] = ($event.target as HTMLInputElement).checked ? 'on' : 'off'"
                    ><span>{{ values[field.key] === 'on' ? 'On' : 'Off' }}</span>
                  </label>
                </div>
              </template>
              <template v-else>
                <label class="lab" :for="fieldId(field.key)">
                  {{ field.label }}
                  <span v-if="field.restart" class="tag-restart"><Icon name="refresh" small />Restart</span>
                  <span v-if="field.hint" class="hint">{{ field.hint }}</span>
                </label>
                <div class="ctl-col">
                  <select
                    v-if="field.kind === 'select'"
                    :id="fieldId(field.key)"
                    v-model="values[field.key]"
                    class="text-input"
                    :data-key="field.key"
                    :data-testid="'setting-' + field.key"
                    :data-secret="field.secret ? 'true' : 'false'"
                  >
                    <option v-for="option in field.options" :key="option" :value="option">{{ OPTION_LABELS[option] || option }}</option>
                  </select>
                  <input
                    v-else
                    :id="fieldId(field.key)"
                    v-model="values[field.key]"
                    :class="['text-input', { 'mono-input': field.kind !== 'secret' && MONO_KEY.test(field.key) }]"
                    :type="field.kind === 'secret' ? 'password' : field.kind === 'number' ? 'number' : 'text'"
                    :min="field.kind === 'number' && field.min != null ? String(field.min) : undefined"
                    :max="field.kind === 'number' && field.max != null ? String(field.max) : undefined"
                    :placeholder="field.kind === 'secret' ? (field.set ? 'stored, type to replace' : 'not set') : field.kind === 'number' ? field.default || undefined : undefined"
                    :autocomplete="field.kind === 'secret' ? 'new-password' : undefined"
                    spellcheck="false"
                    :data-key="field.key"
                    :data-testid="'setting-' + field.key"
                    :data-secret="field.secret ? 'true' : 'false'"
                  >
                </div>
              </template>
            </div>
          </div>
        </div>
      </section>

      <section data-section="server" aria-labelledby="h-server" :hidden="s.section !== 'server'">
        <button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button>
        <header class="set-head">
          <div>
            <h2 id="h-server" class="set-h" tabindex="-1">Server</h2>
            <p class="set-sub">Facts read from this server right now; nothing is cached. A failure is the server's real answer for this machine.</p>
          </div>
        </header>
        <div class="set-stack set-narrow">
          <div class="set-card">
            <div class="set-card-h">
              <div><h3 class="set-card-t">Checks</h3><p class="set-card-d">GET /api/health · GET /api/me · POST /api/device/rtsp · GET /api/stream for the open camera.</p></div>
              <div class="set-card-a"><button id="run-checks" ref="checksButton" class="btn" type="button" data-testid="run-checks" :aria-busy="checking" @click="runChecks"><Icon name="refresh" small />{{ checking ? 'Checking…' : 'Run checks' }}</button></div>
            </div>
            <dl class="report checks" data-testid="checks">
              <div v-for="[key, name] in [['health', 'Health'], ['account', 'Account'], ['probe', 'Camera probe'], ['ticket', 'Stream ticket']]" :key="key">
                <dt>{{ name }}</dt>
                <dd :id="'check-' + key" :data-ok="checks[key].ok === null ? '' : String(checks[key].ok)">
                  <Icon v-if="checks[key].ok !== null" :name="checks[key].ok ? 'check' : 'warning'" small /><span>{{ checks[key].text }}</span>
                </dd>
              </div>
            </dl>
          </div>
        </div>
      </section>
      <section data-section="device" aria-labelledby="h-device" :hidden="s.section !== 'device'"><button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button><DeviceSettings :shown="s.section === 'device'" /></section>
      <section data-section="image" aria-labelledby="h-image" :hidden="s.section !== 'image'"><button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button><ImageSettings :shown="s.section === 'image'" /></section>
      <section data-section="alarm" aria-labelledby="h-alarm" :hidden="s.section !== 'alarm'"><button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button><AlarmSettings :shown="s.section === 'alarm'" /></section>
      <section data-section="system" aria-labelledby="h-system" :hidden="s.section !== 'system'"><button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button><SystemAbout :shown="s.section === 'system'" /></section>
      <section data-section="account" aria-labelledby="h-account" :hidden="s.section !== 'account'"><button class="set-crumb" type="button" @click="toList"><Icon name="chevron-left" small />All settings</button><AccountPanel :shown="s.section === 'account'" /></section>
    </div>
  </div>
  <div class="savebar" :data-tone="barTone || undefined" :aria-label="'Save bar, ' + sectionName" role="region">
    <p v-show="barOwn" id="save-state" class="state" data-testid="save-state" :role="barTone === 'bad' ? 'alert' : 'status'">
      <Icon v-if="barTone === 'bad'" name="warning" small /><Icon v-else-if="!changed && !pendingRestart" name="check" small /><span>{{ barError || barText }}</span>
    </p>
    <div v-show="!barOwn" id="savebar-note" class="savebar-note" />
    <div class="spacer" />
    <button id="settings-discard" class="btn" type="button" :hidden="changed === 0" @click="discard">Discard</button>
    <button id="settings-save" ref="saveButton" class="btn btn-fill" data-testid="settings-save" type="button" aria-keyshortcuts="Control+S" :aria-busy="saving" :hidden="changed === 0" @click="save">{{ saving ? 'Saving…' : 'Save changes' }}</button>
    <button id="settings-restart" class="btn btn-fill" data-testid="settings-restart" type="button" :aria-busy="restarting" :hidden="!(changed === 0 && pendingRestart > 0 && canRestart)" @click="restart">{{ restarting ? 'Restarting…' : 'Restart the app' }}</button>
  </div>
</template>
