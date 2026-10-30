<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { sessionLost } from '../app/shell'
import { state } from '../app/state'
import Icon from '../components/Icon.vue'
import { settingsUi } from '../settings/ui'
import { TechDetails } from '../states'
import CardFormat from './CardFormat.vue'
import OsdSettings, { type OsdState, type TitleDot } from './OsdSettings.vue'
import RecordSettings, { type RecordState } from './RecordSettings.vue'

type Stream = 'main' | 'extra'
type Block = 'encode' | 'network' | 'storage' | 'record' | 'osd'
type Job = '' | Stream | 'smart' | 'network' | 'storage' | 'record' | 'osd' | 'format'
type Field = { field: string; before: unknown; requested: unknown; after: unknown; matches: boolean }
type Report = { block: string; backup: string; fields: Field[]; reverted: boolean; restored: Field[] }
type Format = {
  resolution?: string | null
  compression?: string | null
  fps?: number | null
  bitrate?: number | null
  gop?: number | null
  quality?: number | null
  bitrate_control?: string | null
}
type Network = {
  host_ip?: string | null
  submask?: string | null
  gateway?: string | null
  mac?: string | null
  dhcp?: boolean | null
  dns?: string | null
  spare_dns?: string | null
}
type Partition = {
  disk?: number
  index: number
  current?: boolean | null
  status?: number | string | null
  driver_type?: number | null
  total_space?: string | null
  remain_space?: string | null
  total_mb?: number
  remain_mb?: number
  start?: string | null
}
type Usage = {
  card: string
  total_mb: number
  remain_mb: number
  record_mb: number
  snapshot_mb: number
  remain_percent?: number | null
  record_percent?: number | null
  snapshot_percent?: number | null
}
type Storage = {
  partitions: Partition[]
  usage?: Usage
  percent_view?: boolean
  overwrite?: string | null
  record_mode?: string | null
  missing_alarm?: boolean | null
}
type Limits = { resolutions: string[]; max_fps: Record<string, number> }
type Capability = { max_bitrate?: number | null; fps_ceiling: number; main: Limits; extra: Limits }
type Smart = { mode: string; modes: string[]; source: string }
type Reply = { ok: boolean; status: number; body: any }
type EncodeForm = { fps: number | string; bitrate: number | string; gop: number | string; resolution: string }

const props = defineProps<{ shown: boolean }>()

const BASE = '/api/device/settings'
const STREAMS: Stream[] = ['main', 'extra']
const WORKING: Record<Exclude<Job, ''>, string> = {
  main: 'Saving main…',
  extra: 'Saving extra…',
  smart: 'Saving smart encoding…',
  network: 'Saving network…',
  storage: 'Saving storage…',
  record: 'Saving recording…',
  osd: 'Saving on-screen text…',
  format: 'Formatting…',
}
const SMART_LABEL: Record<string, string> = { off: 'Off', smart: 'Smart (H.264+ / H.265+)', smart_plus: 'Smart plus' }
const NO_SMART = 'The camera answers neither AVEnc.SmartH264V2 with Encode264ability nor AVEnc.SmartH264.'
const ONLY_OFF = 'Encode264ability reports neither H.264+ nor H.265+.'

const status = reactive({ text: 'Not loaded yet.', bad: false })
const reports = ref<Report[]>([])
const busy = ref<Job>('')
const encode = reactive({ data: null as Record<string, Format> | null, error: '', capability: null as Capability | null, smart: null as Smart | null })
const network = reactive({ data: null as Network | null, error: '' })
const storage = reactive({ data: null as Storage | null, error: '' })
const record = reactive({ data: null as RecordState | null, error: '' })
const osd = reactive({ data: null as OsdState | null, error: '' })
const smartForm = ref('off')
const encodeForm = reactive<Record<Stream, EncodeForm>>({
  main: { fps: '', bitrate: '', gop: '', resolution: '' },
  extra: { fps: '', bitrate: '', gop: '', resolution: '' },
})
const networkForm = reactive({ dhcp: false, dns: '', spare_dns: '', gateway: '', submask: '' })
const storageForm = reactive({ overwrite: 'overwrite', record: 'config', missing_alarm: false })

const text = (value: unknown) => (value === null || value === undefined ? '' : String(value))
const known = (value: unknown) => (value === null || value === undefined ? null : Number(value))
const asNumber = (value: number | string) => (value === '' ? null : Number(value))

const currentOverwrite = computed(() => ((storage.data?.overwrite || 'OverWrite') === 'StopRecord' ? 'stop' : 'overwrite'))
const currentRecord = computed(() => {
  const wire = (storage.data?.record_mode || 'ConfigRecord').toLowerCase()
  return wire.startsWith('manual') ? 'manual' : wire.startsWith('off') ? 'off' : 'config'
})

function withDevice(path: string) {
  const id = state.selectedId || ''
  return id ? path + (path.includes('?') ? '&' : '?') + 'device=' + encodeURIComponent(id) : path
}

function limitsFor(stream: Stream): Limits | null {
  return encode.capability ? encode.capability[stream] : null
}

function fpsCeiling(stream: Stream) {
  const limits = limitsFor(stream)
  if (!limits) return null
  const token = encodeForm[stream].resolution
  const found = Object.entries(limits.max_fps).find(([name]) => name.toLowerCase() === token.toLowerCase())
  return found ? found[1] : encode.capability?.fps_ceiling ?? null
}

function megabytes(value: number) {
  return value >= 1024 ? (value / 1024).toFixed(2) + ' GB' : value + ' MB'
}

function usageLine(usage: Usage, percent: boolean) {
  if (percent) {
    return 'Free ' + (usage.remain_percent ?? 0) + '% · recordings ' + (usage.record_percent ?? 0) + '% · pictures ' + (usage.snapshot_percent ?? 0) + '%'
  }
  return 'Total ' + megabytes(usage.total_mb) + ' · free ' + megabytes(usage.remain_mb) + ' · recordings ' + megabytes(usage.record_mb) + ' · pictures ' + megabytes(usage.snapshot_mb)
}

function say(line: string, bad = false) {
  status.text = line
  status.bad = bad
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

const expired = (...results: Reply[]) => results.some((result) => result.status === 401) && sessionLost({ status: 401 })

function formatLine(format: Format) {
  return (
    (format.compression || 'unknown codec') +
    ' · quality ' +
    (format.quality === null || format.quality === undefined ? '?' : format.quality) +
    ' · ' +
    (format.bitrate_control || '?')
  )
}

function partitionState(part: Partition) {
  if (part.current === true) return 'active'
  return part.status === null || part.status === undefined ? '?' : String(part.status)
}

function fillEncode() {
  for (const stream of STREAMS) {
    const format = encode.data?.[stream] || {}
    encodeForm[stream] = { fps: text(format.fps), bitrate: text(format.bitrate), gop: text(format.gop), resolution: text(format.resolution) }
  }
}

function fillNetwork() {
  const data = network.data || {}
  Object.assign(networkForm, {
    dhcp: Boolean(data.dhcp),
    dns: text(data.dns),
    spare_dns: text(data.spare_dns),
    gateway: text(data.gateway),
    submask: text(data.submask),
  })
}

function fillStorage() {
  storageForm.overwrite = currentOverwrite.value
  storageForm.record = currentRecord.value
  storageForm.missing_alarm = Boolean(storage.data?.missing_alarm)
}

function fillSmart() {
  smartForm.value = encode.smart ? encode.smart.mode : 'off'
}

const noop = () => {}

function apply(name: Block, result: Reply) {
  const target = { encode, network, storage, record, osd }[name]
  if (result.ok && result.body) {
    target.data = result.body[name]
    target.error = ''
    if (name === 'encode') {
      encode.capability = result.body.capability || null
      encode.smart = result.body.smart || null
      fillSmart()
    }
    ;({ encode: fillEncode, network: fillNetwork, storage: fillStorage, record: noop, osd: noop })[name]()
  } else {
    target.error = name + ': ' + (result.body && result.body.message ? result.body.message : failureText(result))
  }
}

let ticket = 0

async function load(afterWrite = false) {
  const mine = ++ticket
  if (!afterWrite) say('Reading the device settings over DVRIP…')
  const [encodeReply, networkReply, storageReply, recordReply, osdReply] = await Promise.all([
    request(withDevice(BASE + '/encode')),
    request(withDevice(BASE + '/network')),
    request(withDevice(BASE + '/storage')),
    request(withDevice(BASE + '/record')),
    request(withDevice(BASE + '/osd')),
  ])
  if (mine !== ticket) return
  const replies = [encodeReply, networkReply, storageReply, recordReply, osdReply]
  if (expired(...replies)) return
  if (replies.every((reply) => !reply.ok)) {
    say('The camera answered nothing: ' + failureText(encodeReply), true)
    return
  }
  apply('encode', encodeReply)
  apply('network', networkReply)
  apply('storage', storageReply)
  apply('record', recordReply)
  apply('osd', osdReply)
  if (!afterWrite) say('Read from the device just now.')
}

function showReports(body: any) {
  const found: Report[] = (body && body.reports) || []
  const errors: string[] = (body && body.errors) || []
  reports.value = found
  if (errors.length) say(errors.join(' / '), true)
  else if (body && body.reverted) say('The write did not hold and was reverted; the device keeps its previous values.', true)
  else if (found.length) say('Read back after the write and kept.')
}

async function write(
  job: Exclude<Job, ''>,
  path: string,
  patch: Record<string, string>,
  nothing: string,
  writing: string,
  after?: () => Promise<string>,
) {
  if (busy.value) {
    if (busy.value !== job) say('Wait for the ' + busy.value + ' write to finish.')
    return
  }
  if (!Object.keys(patch).length) {
    say(nothing)
    return
  }
  busy.value = job
  say(writing)
  try {
    const result = await request(withDevice(BASE + path + '?' + new URLSearchParams(patch)), { method: 'PUT' })
    if (expired(result)) return
    if (!result.ok && !(result.body && result.body.reports)) {
      say('Could not write: ' + failureText(result), true)
      return
    }
    showReports(result.body)
    if (after && result.ok) {
      const problem = await after()
      if (problem) say(problem, true)
    }
    await load(true)
  } finally {
    busy.value = ''
  }
}

function saveEncode(stream: Stream) {
  const format = encode.data?.[stream] || {}
  const form = encodeForm[stream]
  const patch: Record<string, string> = {}
  for (const key of ['fps', 'bitrate', 'gop'] as const) {
    if (asNumber(form[key]) !== known(format[key])) patch[key] = String(asNumber(form[key]))
  }
  if (form.resolution !== (format.resolution || '')) patch.resolution = form.resolution
  const query = Object.keys(patch).length ? { stream, ...patch } : {}
  return write(stream, '/encode', query, 'Nothing changed on the ' + stream + ' stream.', 'Writing the ' + stream + ' encoder…')
}

function saveNetwork() {
  const data = network.data || {}
  const patch: Record<string, string> = {}
  if (networkForm.dhcp !== Boolean(data.dhcp)) patch.dhcp = String(networkForm.dhcp)
  for (const key of ['dns', 'spare_dns', 'gateway', 'submask'] as const) {
    if (networkForm[key] !== (data[key] || '')) patch[key] = networkForm[key]
  }
  return write('network', '/network', patch, 'Nothing changed on the network.', 'Writing the network settings…')
}

function saveSmart() {
  const current = encode.smart ? encode.smart.mode : 'off'
  const patch: Record<string, string> = smartForm.value !== current ? { smart: smartForm.value } : {}
  return write('smart', '/encode', patch, 'Smart encoding is unchanged.', 'Writing smart encoding…')
}

function saveStorage() {
  const patch: Record<string, string> = {}
  if (storageForm.overwrite !== currentOverwrite.value) patch.overwrite = storageForm.overwrite
  if (storageForm.record !== currentRecord.value) patch.record = storageForm.record
  const alarm = storage.data?.missing_alarm
  if (alarm !== null && alarm !== undefined && storageForm.missing_alarm !== alarm) patch.missing_alarm = String(storageForm.missing_alarm)
  return write('storage', '/storage', patch, 'Nothing changed on storage.', 'Writing the storage settings…')
}

function saveRecord(patch: Record<string, string>) {
  return write('record', '/record', patch, 'Nothing changed on recording.', 'Writing the recording settings…')
}

function saveOsd(patch: Record<string, string>, dot: TitleDot | null) {
  const after = dot
    ? async () => {
        const reply = await request(withDevice(BASE + '/osd/dot'), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(dot),
        })
        return reply.ok ? '' : 'The title was saved but the title bitmap was not: ' + failureText(reply)
      }
    : undefined
  return write('osd', '/osd', patch, 'Nothing changed on the on-screen text.', 'Writing the on-screen text…', after)
}

async function formatCard(confirm: string) {
  if (busy.value) {
    say('Wait for the ' + busy.value + ' write to finish.')
    return
  }
  busy.value = 'format'
  say('Formatting the memory card…')
  try {
    const result = await request(withDevice(BASE + '/storage/format?confirm=' + encodeURIComponent(confirm)), { method: 'POST' })
    if (expired(result)) return
    if (!result.ok) {
      say('Could not format: ' + failureText(result), true)
      return
    }
    say('The card was formatted (' + ((result.body && result.body.steps) || []).length + ' partitions cleared).')
    await load(true)
  } finally {
    busy.value = ''
  }
}

function label(job: Exclude<Job, ''>, idle: string) {
  return busy.value === job ? WORKING[job] : idle
}

watch(
  () => props.shown,
  (shown) => {
    if (shown) load()
  },
)

watch(
  () => settingsUi.reload.device,
  () => {
    if (props.shown) load()
  },
)
</script>

<template>
  <header class="set-head">
    <div>
      <h2 id="h-device" class="set-h" tabindex="-1">Video &amp; storage</h2>
      <p class="set-sub">Encoder, recording, on-screen text, network and storage as the camera holds them. Each card saves on its own and is checked with the camera.</p>
    </div>
  </header>
  <Teleport defer to="#savebar-note">
    <p v-show="shown" id="device-status" data-testid="device-status" :class="['set-status', { 'warn-text': status.bad }]" role="status">
      <Icon v-if="status.bad" name="warning" small /><span>{{ status.text }}</span>
    </p>
  </Teleport>
  <div class="set-cols">
    <div class="set-col">
      <div id="device-encode" class="dev-group" data-testid="device-encode">
        <div v-if="encode.error" class="set-card">
          <div class="set-card-h"><div><h3 class="set-card-t">Encoder</h3></div></div>
          <p class="set-status warn-text"><Icon name="warning" small /><span>{{ encode.error }}</span></p>
        </div>
        <template v-else-if="encode.data">
          <div v-for="stream in STREAMS" :key="stream" class="set-card">
            <div class="set-card-h">
              <div>
                <h3 class="set-card-t">{{ stream === 'main' ? 'Main stream' : 'Extra stream' }}</h3>
                <p class="set-card-d">{{ formatLine(encode.data[stream] || {}) }}</p>
              </div>
            </div>
            <div class="row">
              <div class="lab">
                <label :for="'device-' + stream + '-fps'">Frames per second</label>
                <span v-if="fpsCeiling(stream) !== null" class="hint">Up to {{ fpsCeiling(stream) }} at {{ encodeForm[stream].resolution || 'this resolution' }}, from EncodeCapability.</span>
              </div>
              <div class="ctl-col">
                <input :id="'device-' + stream + '-fps'" v-model="encodeForm[stream].fps" type="number" step="1" min="0" :max="fpsCeiling(stream) ?? undefined">
              </div>
            </div>
            <div class="row">
              <div class="lab"><label :for="'device-' + stream + '-bitrate'">Bitrate (kbps)</label></div>
              <div class="ctl-col">
                <input :id="'device-' + stream + '-bitrate'" v-model="encodeForm[stream].bitrate" type="number" step="1" min="0" :max="encode.capability?.max_bitrate ?? undefined">
              </div>
            </div>
            <div class="row">
              <div class="lab"><label :for="'device-' + stream + '-gop'">Keyframe interval</label></div>
              <div class="ctl-col">
                <input :id="'device-' + stream + '-gop'" v-model="encodeForm[stream].gop" type="number" step="1" min="0">
              </div>
            </div>
            <div class="row">
              <div class="lab">
                <label :for="'device-' + stream + '-resolution'">Resolution</label>
                <span class="hint">{{ limitsFor(stream)?.resolutions.length ? 'The sizes EncodeCapability allows for this stream.' : 'The device token, for example 3072*2048.' }}</span>
              </div>
              <div class="ctl-col">
                <select v-if="limitsFor(stream)?.resolutions.length" :id="'device-' + stream + '-resolution'" v-model="encodeForm[stream].resolution">
                  <option v-for="name in limitsFor(stream)?.resolutions" :key="name" :value="name">{{ name }}</option>
                </select>
                <input v-else :id="'device-' + stream + '-resolution'" v-model="encodeForm[stream].resolution" type="text" placeholder="3072*2048">
              </div>
            </div>
            <div class="set-card-f">
              <button
                class="btn btn-fill"
                type="button"
                :data-testid="'device-save-' + stream"
                :aria-busy="busy === stream"
                @click="saveEncode(stream)"
              >{{ label(stream, 'Save ' + stream) }}</button>
            </div>
          </div>
        </template>
      </div>
      <div v-if="encode.data" id="device-smart" class="set-card" data-testid="device-smart">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">Smart encoding</h3>
            <p class="set-card-d">{{ encode.smart ? 'Now: ' + (SMART_LABEL[encode.smart.mode] || encode.smart.mode) : 'Off' }}</p>
          </div>
        </div>
        <div class="row">
          <div class="lab">
            <label for="device-smart-mode">Mode</label>
            <span v-if="!encode.smart" class="hint">Not offered by this camera.</span>
            <span v-else-if="encode.smart.modes.length < 2" class="hint">This camera offers only Off.</span>
          </div>
          <div class="ctl-col">
            <select id="device-smart-mode" v-model="smartForm" :disabled="!encode.smart || encode.smart.modes.length < 2">
              <option v-for="mode in encode.smart ? encode.smart.modes : ['off']" :key="mode" :value="mode">{{ SMART_LABEL[mode] || mode }}</option>
            </select>
            <TechDetails v-if="!encode.smart || encode.smart.modes.length < 2" data-testid="device-smart-details" :text="encode.smart ? ONLY_OFF : NO_SMART" />
          </div>
        </div>
        <div class="set-card-f">
          <button class="btn btn-fill" type="button" data-testid="device-save-smart" :aria-busy="busy === 'smart'" :disabled="!encode.smart || encode.smart.modes.length < 2" @click="saveSmart">
            {{ label('smart', 'Save smart encoding') }}
          </button>
        </div>
      </div>
      <div id="device-record" class="dev-group" data-testid="device-record">
        <div v-if="record.error" class="set-card">
          <div class="set-card-h"><div><h3 class="set-card-t">Recording</h3></div></div>
          <p class="set-status warn-text"><Icon name="warning" small /><span>{{ record.error }}</span></p>
        </div>
        <RecordSettings v-else-if="record.data" :record="record.data" :busy="busy === 'record'" :label="label('record', 'Save recording')" @save="saveRecord" />
      </div>
      <div id="device-osd-block" class="dev-group">
        <div v-if="osd.error" class="set-card">
          <div class="set-card-h"><div><h3 class="set-card-t">On-screen text</h3></div></div>
          <p class="set-status warn-text"><Icon name="warning" small /><span>{{ osd.error }}</span></p>
        </div>
        <OsdSettings v-else-if="osd.data" :osd="osd.data" :busy="busy === 'osd'" :label="label('osd', 'Save on-screen text')" @save="saveOsd" />
      </div>
    </div>
    <div class="set-col">
      <div class="set-card">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">Last write report</h3>
            <p class="set-card-d">Before, asked for and after for every field the last save touched.</p>
          </div>
        </div>
        <p v-if="!reports.length" class="set-empty">No write in this visit yet.</p>
        <div id="device-report" class="dev-report" data-testid="device-report">
          <div v-for="report in reports" :key="report.block + report.backup" class="row stack">
            <div class="lab">
              {{ report.block }}
              <span class="hint">
                {{ (report.reverted ? 'The device returned a different value, so the saved copy was put back. ' : 'Read back and kept. ') + 'Copy: ' + report.backup }}
              </span>
            </div>
            <div class="ctl-col">
              <table class="tbl">
                <thead>
                  <tr>
                    <th scope="col">Field</th>
                    <th scope="col">Before</th>
                    <th scope="col">Asked for</th>
                    <th scope="col">After</th>
                    <th scope="col">State</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="field in report.fields" :key="'f' + field.field">
                    <th scope="row">{{ field.field }}</th>
                    <td class="code">{{ JSON.stringify(field.before) }}</td>
                    <td class="code">{{ JSON.stringify(field.requested) }}</td>
                    <td class="code">{{ JSON.stringify(field.after) }}</td>
                    <td>{{ field.matches ? 'matches' : 'differs' }}</td>
                  </tr>
                  <template v-if="report.restored.length">
                    <tr><th colspan="5" scope="row">put back from the saved copy</th></tr>
                    <tr v-for="field in report.restored" :key="'r' + field.field">
                      <th scope="row">{{ field.field }}</th>
                      <td class="code">{{ JSON.stringify(field.before) }}</td>
                      <td class="code">{{ JSON.stringify(field.requested) }}</td>
                      <td class="code">{{ JSON.stringify(field.after) }}</td>
                      <td>{{ field.matches ? 'restored' : 'still differs' }}</td>
                    </tr>
                  </template>
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </div>
      <div id="device-network" class="dev-group" data-testid="device-network">
        <div v-if="network.error" class="set-card">
          <div class="set-card-h"><div><h3 class="set-card-t">Network</h3></div></div>
          <p class="set-status warn-text"><Icon name="warning" small /><span>{{ network.error }}</span></p>
        </div>
        <div v-else-if="network.data" class="set-card">
          <div class="set-card-h">
            <div>
              <h3 class="set-card-t">Network</h3>
              <p class="set-card-d">DHCP, name servers, gateway and mask. The address itself is read only.</p>
            </div>
          </div>
          <div class="row">
            <div class="lab">Address<span class="hint">Read only here: a new address moves the camera away from the one this app reaches, so the route refuses it.</span></div>
            <div class="ctl-col"><p class="val"><span class="code">{{ network.data.host_ip || 'unknown' }}</span></p></div>
          </div>
          <div class="row">
            <div class="lab">MAC</div>
            <div class="ctl-col"><p class="val"><span class="code">{{ network.data.mac || 'unknown' }}</span></p></div>
          </div>
          <div class="row">
            <div class="lab">DHCP</div>
            <div class="ctl-col">
              <label class="switchrow"><input id="device-dhcp" v-model="networkForm.dhcp" type="checkbox" role="switch"><span>Take the address from DHCP</span></label>
            </div>
          </div>
          <div class="row">
            <label class="lab" for="device-dns">DNS</label>
            <div class="ctl-col"><input id="device-dns" v-model="networkForm.dns" type="text" placeholder="1.1.1.1"></div>
          </div>
          <div class="row">
            <label class="lab" for="device-spare-dns">Spare DNS</label>
            <div class="ctl-col"><input id="device-spare-dns" v-model="networkForm.spare_dns" type="text" placeholder="8.8.8.8"></div>
          </div>
          <div class="row">
            <label class="lab" for="device-gateway">Gateway</label>
            <div class="ctl-col"><input id="device-gateway" v-model="networkForm.gateway" type="text"></div>
          </div>
          <div class="row">
            <label class="lab" for="device-submask">Subnet mask</label>
            <div class="ctl-col"><input id="device-submask" v-model="networkForm.submask" type="text"></div>
          </div>
          <div class="set-card-f">
            <button class="btn btn-fill" type="button" data-testid="device-save-network" :aria-busy="busy === 'network'" @click="saveNetwork">
              {{ label('network', 'Save network') }}
            </button>
          </div>
        </div>
      </div>
      <div id="device-storage" class="dev-group" data-testid="device-storage">
        <div v-if="storage.error" class="set-card">
          <div class="set-card-h"><div><h3 class="set-card-t">Storage</h3></div></div>
          <p class="set-status warn-text"><Icon name="warning" small /><span>{{ storage.error }}</span></p>
        </div>
        <div v-else-if="storage.data" class="set-card">
          <div class="set-card-h">
            <div>
              <h3 class="set-card-t">Storage</h3>
              <p class="set-card-d">The memory card and what the camera does when it fills up.</p>
            </div>
          </div>
          <div class="row stack">
            <div class="lab">Memory card</div>
            <div class="ctl-col">
              <table v-if="storage.data.partitions.length" class="tbl">
                <thead>
                  <tr>
                    <th scope="col">Card</th>
                    <th scope="col">Status</th>
                    <th scope="col">Total</th>
                    <th scope="col">Free</th>
                    <th scope="col">Oldest recording</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="part in storage.data.partitions" :key="part.index">
                    <th scope="row">{{ part.index }}</th>
                    <td>{{ partitionState(part) }}</td>
                    <td class="code">{{ part.total_space || '?' }}</td>
                    <td class="code">{{ part.remain_space || '?' }}</td>
                    <td class="code">{{ part.start || 'no recording' }}</td>
                  </tr>
                </tbody>
              </table>
              <p v-else class="val">The device reports no card in this camera.</p>
              <p v-if="storage.data.usage" class="val" data-testid="device-usage">{{ usageLine(storage.data.usage, Boolean(storage.data.percent_view)) }}</p>
            </div>
          </div>
          <div class="row">
            <div class="lab">
              Card missing alarm
              <span v-if="storage.data.missing_alarm === null || storage.data.missing_alarm === undefined" class="hint">The camera does not answer Storage.StorageNotExist.</span>
            </div>
            <div class="ctl-col">
              <label class="switchrow">
                <input id="device-missing-alarm" v-model="storageForm.missing_alarm" type="checkbox" role="switch" :disabled="storage.data.missing_alarm === null || storage.data.missing_alarm === undefined">
                <span>Raise an alarm when the card is missing or fails</span>
              </label>
            </div>
          </div>
          <div class="row">
            <label class="lab" for="device-overwrite">When the card is full</label>
            <div class="ctl-col">
              <select id="device-overwrite" v-model="storageForm.overwrite">
                <option value="overwrite">Overwrite the oldest recording</option>
                <option value="stop">Stop recording when full</option>
              </select>
            </div>
          </div>
          <div class="row">
            <label class="lab" for="device-record">Record mode</label>
            <div class="ctl-col">
              <select id="device-record" v-model="storageForm.record">
                <option value="config">Follow the schedule</option>
                <option value="manual">Always record</option>
                <option value="off">Stop recording</option>
              </select>
            </div>
          </div>
          <div class="set-card-f">
            <button class="btn btn-fill" type="button" data-testid="device-save-storage" :aria-busy="busy === 'storage'" @click="saveStorage">
              {{ label('storage', 'Save storage') }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
  <div v-if="storage.data" class="set-stack">
    <div class="set-card danger">
      <div class="set-card-h">
        <div>
          <h3 class="set-card-t"><Icon name="warning" small />Danger zone</h3>
          <p class="set-card-d">Cannot be undone.</p>
        </div>
      </div>
      <CardFormat :card="storage.data.usage ? storage.data.usage.card : storage.data.partitions.length ? 'ok' : 'absent'" :busy="busy === 'format'" :label="label('format', 'Format card')" @format="formatCard" />
    </div>
  </div>
</template>

<style scoped>
.dev-group { display: grid; gap: var(--s4); min-width: 0; }
.dev-group:empty { display: none; }
.dev-report .hint, .dev-report .lab { overflow-wrap: anywhere; }
.dev-report > .row:first-child { border-top: 1px solid var(--line-soft); }
</style>
