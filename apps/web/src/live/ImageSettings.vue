<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { sessionLost } from '../app/shell'
import { state } from '../app/state'
import { settingsUi } from '../settings/ui'
import Icon from '../components/Icon.vue'
import { TechDetails } from '../states'

type Feature<T> = { supported: boolean; evidence: string; value: T | null }
type Choice = { code: number; key: string; label: string }
type DayNight = { code: number; key: string; label: string; options: Choice[] }
type Period = { enable: boolean; start: string; end: string }
type Trigger = { level: number; duration: number }
type Light = { mode: string | null; brightness: number | null; period: Period | null; trigger: Trigger | null }
type Sleep = { enable: boolean; sleeping: boolean; repeat_daily: boolean; start: string; end: string }
type Volume = { left: number; right: number; block: string }
type LightMode = { mode: string; label: string; options: { mode: string; label: string }[] }
type Report = {
  day_night: Feature<DayNight>
  light_mode: Feature<LightMode>
  scene: Feature<number>
  low_lux: Feature<boolean>
  white_light: Feature<Light>
  privacy: Feature<boolean>
  sleep: Feature<Sleep>
  status_led: Feature<boolean>
  voice_tip: Feature<boolean>
  speaker_volume: Feature<Volume>
  mic_volume: Feature<Volume>
}
type Reply = { ok: boolean; status: number; body: any }
type Job = '' | 'day' | 'mode' | 'scene' | 'lux' | 'light' | 'privacy' | 'sleep' | 'wake' | 'led' | 'tip' | 'speaker' | 'mic'

const props = defineProps<{ shown: boolean }>()

const LEVELS = [
  { value: 1, label: 'Low' },
  { value: 3, label: 'Middle' },
  { value: 5, label: 'High' },
]
const DURATIONS = [5, 10, 30, 60, 90, 120]

const report = ref<Report | null>(null)
const status = reactive({ text: 'Not loaded yet.', bad: false })
const busy = ref<Job>('')
const loading = ref(false)
const form = reactive({
  day: 0,
  lightMode: '',
  scene: 0,
  periodOn: false,
  start: '18:00',
  end: '06:00',
  level: 3,
  duration: 60,
  sleepOn: false,
  sleepStart: '00:00',
  sleepEnd: '06:00',
  sleepRepeat: false,
  speaker: 50,
  mic: 50,
})

const light = computed(() => report.value?.white_light.value || null)

function say(line: string, bad = false) {
  status.text = line
  status.bad = bad
}

function deviceQuery(separator: '?' | '&') {
  const id = state.selectedId || ''
  return id ? separator + 'device=' + encodeURIComponent(id) : ''
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

const failureText = (result: Reply) => (result.body && result.body.message) || 'HTTP ' + result.status

function fill(data: Report) {
  form.day = data.day_night.value?.code ?? 0
  form.lightMode = data.light_mode.value?.mode ?? ''
  form.scene = data.scene.value ?? 0
  const schedule = data.white_light.value
  form.periodOn = schedule?.period?.enable ?? false
  form.start = schedule?.period?.start ?? '18:00'
  form.end = schedule?.period?.end ?? '06:00'
  form.level = schedule?.trigger?.level ?? 3
  form.duration = schedule?.trigger?.duration ?? 60
  form.sleepOn = data.sleep.value?.enable ?? false
  form.sleepStart = data.sleep.value?.start ?? '00:00'
  form.sleepEnd = data.sleep.value?.end ?? '06:00'
  form.sleepRepeat = data.sleep.value?.repeat_daily ?? false
  form.speaker = data.speaker_volume.value?.left ?? 50
  form.mic = data.mic_volume.value?.left ?? 50
}

async function load() {
  if (loading.value) return
  loading.value = true
  say('Reading the camera…')
  const result = await request('/api/device/image' + deviceQuery('?'))
  loading.value = false
  if (result.status === 401 && sessionLost({ status: 401 })) return
  if (!result.ok) {
    say('The camera did not answer: ' + failureText(result), true)
    return
  }
  report.value = result.body as Report
  fill(report.value)
  say('Read from the camera just now.')
}

async function write(job: Exclude<Job, ''>, path: string, query: Record<string, string | number | boolean>, done: string) {
  if (busy.value) return
  busy.value = job
  say('Writing to the camera…')
  const params = new URLSearchParams(Object.entries(query).map(([key, value]) => [key, String(value)]))
  const result = await request('/api/device/image/' + path + '?' + params.toString() + deviceQuery('&'), { method: 'POST' })
  busy.value = ''
  if (result.status === 401 && sessionLost({ status: 401 })) return
  if (result.status === 409) {
    say('The camera kept a different value: it holds ' + JSON.stringify(result.body.after) + '.', true)
  } else if (!result.ok) {
    say('The camera refused it: ' + failureText(result), true)
    return
  } else {
    say(done + ' The camera read it back.')
  }
  await reload()
}

async function reload() {
  const text = status.text
  const bad = status.bad
  const result = await request('/api/device/image' + deviceQuery('?'))
  if (result.ok) {
    report.value = result.body as Report
    fill(report.value)
  }
  say(text, bad)
}

const saveDay = () => write('day', 'day-night', { code: form.day }, 'Day/night mode saved.')
const saveLightMode = () => write('mode', 'light-mode', { mode: form.lightMode }, 'Day/night mode saved.')
const saveScene = () => write('scene', 'scene', { scene: form.scene }, 'Scene saved.')
const saveLux = (on: boolean) => write('lux', 'low-lux', { enable: on }, 'Low-lux mode saved.')
function saveLight() {
  const query: Record<string, string | number | boolean> = {}
  if (light.value?.period) Object.assign(query, { period: form.periodOn, start: form.start, end: form.end })
  if (light.value?.trigger) Object.assign(query, { level: form.level, duration: form.duration })
  if (!Object.keys(query).length) {
    say('This camera reports neither a light schedule nor a motion trigger to save.', true)
    return
  }
  return write('light', 'light', query, 'Light schedule saved.')
}
const savePrivacy = (on: boolean) => write('privacy', 'privacy', { enable: on }, on ? 'The lens is covered.' : 'The lens is uncovered.')
const saveSleep = () =>
  write('sleep', 'sleep', { enable: form.sleepOn, start: form.sleepStart, end: form.sleepEnd, repeat: form.sleepRepeat }, 'Timed sleep saved.')
const wake = () => write('wake', 'sleep', { wake: true }, 'The camera was asked to wake up.')
const saveLed = (on: boolean) => write('led', 'indicators', { led: on }, 'Status light saved.')
const saveTip = (on: boolean) => write('tip', 'indicators', { voice_tip: on }, 'Voice prompts saved.')
const saveSpeaker = () => write('speaker', 'volume', { target: 'speaker', level: form.speaker }, 'Speaker volume saved.')
const saveMic = () => write('mic', 'volume', { target: 'mic', level: form.mic }, 'Microphone volume saved.')

const checked = (event: Event) => (event.target as HTMLInputElement).checked

watch(
  () => props.shown,
  (shown) => {
    if (shown) load()
  },
)

watch(
  () => settingsUi.reload.image,
  () => {
    if (props.shown) load()
  },
)
</script>

<template>
  <header class="set-head">
    <div>
      <h2 id="h-image" class="set-h" tabindex="-1">Image, light &amp; sound</h2>
      <p class="set-sub">Day and night, spotlight, privacy, indicators and volumes as the camera holds them. Each save is checked with the camera.</p>
    </div>
  </header>
  <Teleport defer to="#savebar-note">
    <p v-show="shown" id="image-status" :class="['set-status', { 'warn-text': status.bad }]" data-testid="image-status" role="status">
      <Icon v-if="status.bad" name="warning" small /><span>{{ status.text }}</span>
    </p>
  </Teleport>
  <p v-if="!report" class="set-empty">Nothing read from this camera yet.</p>
  <div v-else class="set-cols" data-testid="image-report">
    <div class="set-col">
      <div class="set-card">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Picture</h3></div>
        </div>
        <div v-if="report.light_mode.supported" class="row">
          <label class="lab" for="image-light-mode">Day and night<span class="hint">Now: {{ report.light_mode.value?.label }}. On this camera the light mode sets the picture colour.</span></label>
          <div class="ctl-col">
            <select id="image-light-mode" v-model="form.lightMode" data-testid="image-light-mode">
              <option v-for="choice in report.light_mode.value?.options || []" :key="choice.mode" :value="choice.mode">{{ choice.label }}</option>
            </select>
            <button class="btn btn-fill" type="button" data-testid="image-save-light-mode" :aria-busy="busy === 'mode'" @click="saveLightMode">Save</button>
          </div>
        </div>
        <div v-else class="row">
          <label class="lab" for="image-day-night">Day and night<span class="hint" data-testid="image-day-night-reason">{{ report.day_night.supported ? 'Now: ' + report.day_night.value?.label : 'Not offered by this camera.' }}</span></label>
          <div class="ctl-col">
            <select id="image-day-night" v-model.number="form.day" data-testid="image-day-night" :disabled="!report.day_night.supported">
              <option v-for="choice in report.day_night.value?.options || []" :key="choice.code" :value="choice.code">{{ choice.label }}</option>
            </select>
            <button class="btn btn-fill" type="button" data-testid="image-save-day" :disabled="!report.day_night.supported" :aria-busy="busy === 'day'" @click="saveDay">Save</button>
            <TechDetails v-if="!report.day_night.supported" data-testid="image-day-night-details" :text="report.day_night.evidence" />
          </div>
        </div>
        <div class="row">
          <label class="lab" for="image-scene">Scene</label>
          <div class="ctl-col">
            <template v-if="report.scene.supported">
              <input id="image-scene" v-model.number="form.scene" data-testid="image-scene" type="number" min="0" max="255" step="1">
              <button class="btn btn-fill" type="button" :aria-busy="busy === 'scene'" @click="saveScene">Save</button>
            </template>
            <template v-else>
              <p class="hint" data-testid="image-scene-reason">Not offered by this camera.</p>
              <TechDetails data-testid="image-scene-details" :text="report.scene.evidence" />
            </template>
          </div>
        </div>
        <div class="row">
          <div class="lab">Low light</div>
          <div class="ctl-col">
            <label class="switchrow">
              <input
                id="image-low-lux"
                data-testid="image-low-lux"
                type="checkbox"
                role="switch"
                :checked="Boolean(report.low_lux.value)"
                :disabled="!report.low_lux.supported || busy !== ''"
                @change="saveLux(checked($event))"
              ><span>Starlight low-lux mode</span>
            </label>
            <p v-if="!report.low_lux.supported" class="hint" data-testid="image-low-lux-reason">Not offered by this camera.</p>
            <TechDetails v-if="!report.low_lux.supported" data-testid="image-low-lux-details" :text="report.low_lux.evidence" />
          </div>
        </div>
      </div>

      <div class="set-card">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Spotlight</h3></div>
        </div>
        <template v-if="report.white_light.supported && light">
          <div class="row">
            <div class="lab">Mode<span class="hint">{{ report.light_mode.supported ? 'Set with Day and night above.' : 'Set with the white light switch under This camera.' }}</span></div>
            <div class="ctl-col"><p class="val"><span class="code">{{ light.mode || 'not reported' }}</span></p></div>
          </div>
          <div v-if="light.period" class="row">
            <div class="lab">Schedule</div>
            <div class="ctl-col">
              <label class="switchrow"><input id="image-period" v-model="form.periodOn" data-testid="image-period" type="checkbox" role="switch"><span>Only light up inside this window</span></label>
              <div class="ctl-line">
                <label class="field"><span>From</span><input id="image-start" v-model="form.start" data-testid="image-start" type="time"></label>
                <label class="field"><span>Until</span><input id="image-end" v-model="form.end" data-testid="image-end" type="time"></label>
              </div>
            </div>
          </div>
          <div v-if="light.trigger" class="row">
            <div class="lab">On motion</div>
            <div class="ctl-col">
              <div class="ctl-line">
                <label class="field">
                  <span>Sensitivity</span>
                  <select id="image-level" v-model.number="form.level" data-testid="image-level">
                    <option v-for="level in LEVELS" :key="level.value" :value="level.value">{{ level.label }}</option>
                  </select>
                </label>
                <label class="field">
                  <span>Stay lit for</span>
                  <select id="image-duration" v-model.number="form.duration" data-testid="image-duration">
                    <option v-for="seconds in DURATIONS" :key="seconds" :value="seconds">{{ seconds }} s</option>
                  </select>
                </label>
              </div>
            </div>
          </div>
          <div class="set-card-f">
            <button class="btn btn-fill" type="button" data-testid="image-save-light" :aria-busy="busy === 'light'" @click="saveLight">Save spotlight</button>
          </div>
        </template>
        <template v-else>
          <p class="hint" data-testid="image-light-reason">Not offered by this camera.</p>
          <div class="tech-pad"><TechDetails data-testid="image-light-details" align="start" :text="report.white_light.evidence" /></div>
        </template>
      </div>
    </div>

    <div class="set-col">
      <div class="set-card">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Privacy</h3></div>
        </div>
        <div class="row">
          <div class="lab">Cover the lens<span class="hint">While on, the camera sends no picture.</span></div>
          <div class="ctl-col">
            <label class="switchrow">
              <input
                id="image-privacy"
                data-testid="image-privacy"
                type="checkbox"
                role="switch"
                :checked="Boolean(report.privacy.value)"
                :disabled="!report.privacy.supported || busy !== ''"
                @change="savePrivacy(checked($event))"
              ><span>Privacy shading</span>
            </label>
            <p v-if="!report.privacy.supported" class="hint" data-testid="image-privacy-reason">Not offered by this camera.</p>
            <TechDetails v-if="!report.privacy.supported" data-testid="image-privacy-details" :text="report.privacy.evidence" />
          </div>
        </div>
        <div class="row">
          <div class="lab">Timed sleep<span class="hint">The camera turns its picture off inside the window.</span></div>
          <div class="ctl-col">
            <template v-if="report.sleep.supported">
              <label class="switchrow"><input id="image-sleep" v-model="form.sleepOn" data-testid="image-sleep" type="checkbox" role="switch"><span>Sleep on a schedule</span></label>
              <div class="ctl-line">
                <label class="field"><span>From</span><input id="image-sleep-start" v-model="form.sleepStart" type="time"></label>
                <label class="field"><span>Until</span><input id="image-sleep-end" v-model="form.sleepEnd" type="time"></label>
              </div>
              <label class="switchrow"><input id="image-sleep-repeat" v-model="form.sleepRepeat" type="checkbox" role="switch"><span>Every day</span></label>
              <div class="ctl-line">
                <button v-if="report.sleep.value?.sleeping" class="btn" type="button" :aria-busy="busy === 'wake'" @click="wake">Wake now</button>
                <button class="btn btn-fill" type="button" data-testid="image-save-sleep" :aria-busy="busy === 'sleep'" @click="saveSleep">Save sleep</button>
              </div>
            </template>
            <template v-else>
              <p class="hint" data-testid="image-sleep-reason">Not offered by this camera.</p>
              <TechDetails data-testid="image-sleep-details" :text="report.sleep.evidence" />
            </template>
          </div>
        </div>
      </div>

      <div class="set-card">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Indicators and sound</h3></div>
        </div>
        <div class="row">
          <div class="lab">Indicators</div>
          <div class="ctl-col">
            <label class="switchrow">
              <input
                id="image-led"
                data-testid="image-led"
                type="checkbox"
                role="switch"
                :checked="Boolean(report.status_led.value)"
                :disabled="!report.status_led.supported || busy !== ''"
                @change="saveLed(checked($event))"
              ><span>Status light</span>
            </label>
            <p v-if="!report.status_led.supported" class="hint" data-testid="image-led-reason">Not offered by this camera.</p>
            <TechDetails v-if="!report.status_led.supported" data-testid="image-led-details" :text="report.status_led.evidence" />
            <label class="switchrow">
              <input
                id="image-voice-tip"
                data-testid="image-voice-tip"
                type="checkbox"
                role="switch"
                :checked="Boolean(report.voice_tip.value)"
                :disabled="!report.voice_tip.supported || busy !== ''"
                @change="saveTip(checked($event))"
              ><span>Spoken prompts from the camera</span>
            </label>
            <p v-if="!report.voice_tip.supported" class="hint" data-testid="image-voice-tip-reason">Not offered by this camera.</p>
            <TechDetails v-if="!report.voice_tip.supported" data-testid="image-voice-tip-details" :text="report.voice_tip.evidence" />
          </div>
        </div>
        <div class="row">
          <label class="lab" for="image-speaker">Speaker volume</label>
          <div class="ctl-col">
            <div v-if="report.speaker_volume.supported" class="ctl-line">
              <input id="image-speaker" v-model.number="form.speaker" data-testid="image-speaker" type="range" min="0" max="100" step="1">
              <span class="level">{{ form.speaker }}</span>
              <button class="btn btn-fill" type="button" data-testid="image-save-speaker" :aria-busy="busy === 'speaker'" @click="saveSpeaker">Save</button>
            </div>
            <template v-else>
              <p class="hint" data-testid="image-speaker-reason">Not offered by this camera.</p>
              <TechDetails data-testid="image-speaker-details" :text="report.speaker_volume.evidence" />
            </template>
          </div>
        </div>
        <div class="row">
          <label class="lab" for="image-mic">Microphone volume</label>
          <div class="ctl-col">
            <div v-if="report.mic_volume.supported" class="ctl-line">
              <input id="image-mic" v-model.number="form.mic" data-testid="image-mic" type="range" min="0" max="100" step="1">
              <span class="level">{{ form.mic }}</span>
              <button class="btn btn-fill" type="button" data-testid="image-save-mic" :aria-busy="busy === 'mic'" @click="saveMic">Save</button>
            </div>
            <template v-else>
              <p class="hint" data-testid="image-mic-reason">Not offered by this camera.</p>
              <TechDetails data-testid="image-mic-details" :text="report.mic_volume.evidence" />
            </template>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.tech-pad {
  padding: 0 var(--s4) var(--s3);
}
.ctl-line {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  align-items: flex-end;
  gap: var(--s2);
  max-width: 100%;
}
.ctl-line input[type='range'] {
  width: 160px;
  max-width: 100%;
  min-height: var(--hit);
  accent-color: var(--accent);
}
.level {
  align-self: center;
  min-width: 3ch;
  font-variant-numeric: tabular-nums;
  color: var(--ink-2);
  text-align: right;
}
</style>
