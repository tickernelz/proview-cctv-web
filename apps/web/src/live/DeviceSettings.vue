<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { sessionLost } from '../app/shell'

type Stream = 'main' | 'extra'
type Block = 'encode' | 'network' | 'storage'
type Job = '' | Stream | 'network' | 'storage'
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
  index: number
  current?: boolean | null
  status?: number | string | null
  total_space?: string | null
  remain_space?: string | null
  start?: string | null
}
type Storage = { partitions: Partition[]; overwrite?: string | null; record_mode?: string | null }
type Reply = { ok: boolean; status: number; body: any }
type EncodeForm = { fps: number | string; bitrate: number | string; gop: number | string; resolution: string }

const props = defineProps<{ host: HTMLElement }>()

const BASE = '/api/device/settings'
const STREAMS: Stream[] = ['main', 'extra']
const WORKING: Record<Exclude<Job, ''>, string> = {
  main: 'Saving main…',
  extra: 'Saving extra…',
  network: 'Saving network…',
  storage: 'Saving storage…',
}

const status = reactive({ text: 'Not loaded yet.', bad: false })
const reports = ref<Report[]>([])
const busy = ref<Job>('')
const encode = reactive({ data: null as Record<string, Format> | null, error: '' })
const network = reactive({ data: null as Network | null, error: '' })
const storage = reactive({ data: null as Storage | null, error: '' })
const encodeForm = reactive<Record<Stream, EncodeForm>>({
  main: { fps: '', bitrate: '', gop: '', resolution: '' },
  extra: { fps: '', bitrate: '', gop: '', resolution: '' },
})
const networkForm = reactive({ dhcp: false, dns: '', spare_dns: '', gateway: '', submask: '' })
const storageForm = reactive({ overwrite: 'overwrite', record: 'config' })

const text = (value: unknown) => (value === null || value === undefined ? '' : String(value))
const known = (value: unknown) => (value === null || value === undefined ? null : Number(value))
const asNumber = (value: number | string) => (value === '' ? null : Number(value))

const currentOverwrite = computed(() => ((storage.data?.overwrite || 'OverWrite') === 'StopRecord' ? 'stop' : 'overwrite'))
const currentRecord = computed(() => {
  const wire = (storage.data?.record_mode || 'ConfigRecord').toLowerCase()
  return wire.startsWith('manual') ? 'manual' : wire.startsWith('off') ? 'off' : 'config'
})

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
}

function apply(name: Block, result: Reply) {
  const target = { encode, network, storage }[name]
  if (result.ok && result.body) {
    target.data = result.body[name]
    target.error = ''
    ;({ encode: fillEncode, network: fillNetwork, storage: fillStorage })[name]()
  } else {
    target.error = name + ': ' + (result.body && result.body.message ? result.body.message : failureText(result))
  }
}

let ticket = 0

async function load(afterWrite = false) {
  const mine = ++ticket
  if (!afterWrite) say('Reading the device settings over DVRIP…')
  const [encodeReply, networkReply, storageReply] = await Promise.all([
    request(BASE + '/encode'),
    request(BASE + '/network'),
    request(BASE + '/storage'),
  ])
  if (mine !== ticket) return
  if (expired(encodeReply, networkReply, storageReply)) return
  if (!encodeReply.ok && !networkReply.ok && !storageReply.ok) {
    say('The camera answered nothing: ' + failureText(encodeReply), true)
    return
  }
  apply('encode', encodeReply)
  apply('network', networkReply)
  apply('storage', storageReply)
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

async function write(job: Exclude<Job, ''>, path: string, patch: Record<string, string>, nothing: string, writing: string) {
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
    const result = await request(BASE + path + '?' + new URLSearchParams(patch), { method: 'PUT' })
    if (expired(result)) return
    if (!result.ok && !(result.body && result.body.reports)) {
      say('Could not write: ' + failureText(result), true)
      return
    }
    showReports(result.body)
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

function saveStorage() {
  const patch: Record<string, string> = {}
  if (storageForm.overwrite !== currentOverwrite.value) patch.overwrite = storageForm.overwrite
  if (storageForm.record !== currentRecord.value) patch.record = storageForm.record
  return write('storage', '/storage', patch, 'Nothing changed on storage.', 'Writing the storage settings…')
}

function label(job: Exclude<Job, ''>, idle: string) {
  return busy.value === job ? WORKING[job] : idle
}

function open() {
  const page = window as unknown as { openSettings?: (section: string) => void; showSection?: (section: string, focus: boolean) => void }
  if (typeof page.openSettings === 'function') page.openSettings('device')
  else if (typeof page.showSection === 'function') page.showSection('device', true)
  else location.hash = '#settings/device'
}

function openFromNav() {
  const shown = !props.host.hidden
  open()
  if (shown) load()
}

const watcher = new MutationObserver(() => {
  if (!props.host.hidden) load()
})

onMounted(() => {
  watcher.observe(props.host, { attributes: true, attributeFilter: ['hidden'] })
})

onBeforeUnmount(() => watcher.disconnect())
</script>

<template>
  <Teleport to=".set-nav">
    <a href="#settings/device" data-section="device" data-testid="device-settings-link" @click.prevent="openFromNav">Device settings</a>
  </Teleport>
  <h2 id="h-device" class="set-h" tabindex="-1">Device settings</h2>
  <p class="set-sub">
    Encoder, network and storage as the camera holds them now, read over DVRIP. Every write saves a copy of the block first, reads it back, and
    puts the copy back when the device returns a different value.
  </p>
  <p id="device-status" :class="['hint', { 'warn-text': status.bad }]" data-testid="device-status" role="status">{{ status.text }}</p>
  <div id="device-report" data-testid="device-report">
    <div v-for="report in reports" :key="report.block + report.backup" class="row">
      <div class="lab">{{ report.block }}</div>
      <div class="ctl-col">
        <p class="hint">
          {{ (report.reverted ? 'The device returned a different value, so the saved copy was put back. ' : 'Read back and kept. ') + 'Copy: ' + report.backup }}
        </p>
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
  <div>
    <div id="device-encode" data-testid="device-encode">
      <p v-if="encode.error" class="hint warn-text">{{ encode.error }}</p>
      <div v-else-if="encode.data">
        <h3 class="set-h">Encoder</h3>
        <div v-for="stream in STREAMS" :key="stream" class="row">
          <div class="lab">{{ stream }} stream<div class="hint">{{ formatLine(encode.data[stream] || {}) }}</div></div>
          <div class="ctl-col">
            <label class="field">
              <span>Frames per second</span>
              <input :id="'device-' + stream + '-fps'" v-model="encodeForm[stream].fps" type="number" step="1" min="0">
            </label>
            <label class="field">
              <span>Bitrate (kbps)</span>
              <input :id="'device-' + stream + '-bitrate'" v-model="encodeForm[stream].bitrate" type="number" step="1" min="0">
            </label>
            <label class="field">
              <span>Keyframe interval</span>
              <input :id="'device-' + stream + '-gop'" v-model="encodeForm[stream].gop" type="number" step="1" min="0">
            </label>
            <label class="field">
              <span>Resolution</span>
              <input :id="'device-' + stream + '-resolution'" v-model="encodeForm[stream].resolution" type="text" placeholder="3072*2048">
              <span class="hint">The device token, for example 3072*2048.</span>
            </label>
            <button
              class="btn btn-fill"
              type="button"
              :data-testid="'device-save-' + stream"
              :aria-busy="busy === stream"
              @click="saveEncode(stream)"
            >{{ label(stream, 'Save ' + stream) }}</button>
          </div>
        </div>
      </div>
    </div>
    <div id="device-network" data-testid="device-network">
      <p v-if="network.error" class="hint warn-text">{{ network.error }}</p>
      <div v-else-if="network.data">
        <h3 class="set-h">Network</h3>
        <div class="row">
          <div class="lab">Address<div class="hint">Read only here: a new address moves the camera away from the one this app reaches, so the route refuses it.</div></div>
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
        <button class="btn btn-fill" type="button" data-testid="device-save-network" :aria-busy="busy === 'network'" @click="saveNetwork">
          {{ label('network', 'Save network') }}
        </button>
      </div>
    </div>
    <div id="device-storage" data-testid="device-storage">
      <p v-if="storage.error" class="hint warn-text">{{ storage.error }}</p>
      <div v-else-if="storage.data">
        <h3 class="set-h">Storage</h3>
        <div class="row">
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
        <button class="btn btn-fill" type="button" data-testid="device-save-storage" :aria-busy="busy === 'storage'" @click="saveStorage">
          {{ label('storage', 'Save storage') }}
        </button>
      </div>
    </div>
  </div>
</template>
