<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { sessionLost } from '../app/shell'
import { baseOffsetMinutes, dstRule } from '../app/dst'
import { state } from '../app/state'
import { settingsUi } from '../settings/ui'
import Icon from '../components/Icon.vue'
import { TechDetails } from '../states'

type Reply = { ok: boolean; status: number; body: any }
type Ping = { url: string; reachable: boolean; average_ms?: number | null; lost_percent?: number | null; sent: number }
type User = { name: string; group?: string | null; memo?: string | null; reserved?: boolean | null; authorities: number }

const props = defineProps<{ shown: boolean }>()

const BASE = '/api/device/system'
const RESET_WORD = 'RESET'
const UPGRADE_WORD = 'UPGRADE'

const status = reactive({ text: 'Not loaded yet.', bad: false })
const busy = ref('')
const about = reactive({ data: null as any, error: '' })
const clock = reactive({ data: null as any, error: '' })
const language = reactive({ data: null as any, error: '', pick: '' })
const firmware = reactive({ data: null as any, error: '', camera: '', word: '', file: null as File | null, fileWord: '' })
const network = reactive({ data: null as any, error: '' })
const users = reactive({ data: null as any, error: '' })
const danger = reactive({ restart: false, reset: '' })

const camera = ref('')
const currentCamera = () => state.selectedId || (state.devices.length === 1 ? state.devices[0].id : '')
const withDevice = (path: string, extra: Record<string, string> = {}) => {
  const query = new URLSearchParams({ ...(camera.value ? { device: camera.value } : {}), ...extra })
  const text = query.toString()
  return BASE + path + (text ? '?' + text : '')
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

function say(line: string, bad = false) {
  status.text = line
  status.bad = bad
}

function zoneLabel(minutes: number | null | undefined) {
  if (minutes === null || minutes === undefined) return 'unknown'
  const sign = minutes < 0 ? '-' : '+'
  const total = Math.abs(minutes)
  return 'UTC' + sign + String(Math.floor(total / 60)).padStart(2, '0') + ':' + String(total % 60).padStart(2, '0')
}

function runTime(minutes: number | null | undefined) {
  if (minutes === null || minutes === undefined) return 'unknown'
  const days = Math.floor(minutes / 1440)
  const hours = Math.floor((minutes % 1440) / 60)
  return days + ' d ' + hours + ' h (' + minutes + ' min reported by the camera)'
}

function localTime() {
  const now = new Date()
  const pad = (value: number) => String(value).padStart(2, '0')
  return (
    now.getFullYear() + '-' + pad(now.getMonth() + 1) + '-' + pad(now.getDate()) + ' ' + pad(now.getHours()) + ':' + pad(now.getMinutes()) + ':' + pad(now.getSeconds())
  )
}

const noCamera = computed(() => (camera.value ? '' : 'Pick a camera first; every action here talks to one camera.'))
const offers = computed<any[]>(() => (firmware.data && firmware.data.server_check && firmware.data.server_check.offers) || [])
const startReason = computed(() => {
  if (noCamera.value) return noCamera.value
  if (!firmware.data || firmware.camera !== camera.value) return 'Check for an update first.'
  if (!offers.value.length) return 'The upgrade server offers no newer firmware for this camera.'
  if (!(about.data && about.data.gates && about.data.gates.cloud_upgrade)) return 'This camera does not take updates from the upgrade server.'
  if (firmware.word !== UPGRADE_WORD) return 'Type ' + UPGRADE_WORD + ' to confirm.'
  return ''
})
const deviceCheckLine = computed(() => {
  if (!firmware.data) return ''
  if (firmware.data.device_check) return 'The camera answered the check.'
  if (!(about.data && about.data.gates && about.data.gates.cloud_upgrade)) return 'Not offered by this camera.'
  return 'The camera did not answer the check.'
})
const fileReason = computed(() => {
  if (noCamera.value) return noCamera.value
  if (!firmware.file) return 'Choose a firmware file first.'
  if (firmware.fileWord !== UPGRADE_WORD) return 'Type ' + UPGRADE_WORD + ' to confirm.'
  return ''
})
const resetReason = computed(() => noCamera.value || (danger.reset === RESET_WORD ? '' : 'Type ' + RESET_WORD + ' to confirm.'))
const restartReason = computed(() => noCamera.value || (danger.restart ? '' : 'Tick the box to confirm the restart.'))
const languageReason = computed(() => {
  if (noCamera.value) return noCamera.value
  if (!language.data) return 'The language list has not loaded.'
  if (language.pick === language.data.current) return 'This is already the camera language.'
  return ''
})

function apply(target: { data: any; error: string }, name: string, result: Reply) {
  if (result.ok) {
    target.data = result.body
    target.error = ''
  } else {
    target.error = name + ': ' + failureText(result)
  }
}

let ticket = 0

function forgetCamera() {
  Object.assign(firmware, { data: null, error: '', camera: '', word: '', file: null, fileWord: '' })
  Object.assign(network, { data: null, error: '' })
  danger.restart = false
  danger.reset = ''
}

async function load() {
  const next = currentCamera()
  if (next !== camera.value) forgetCamera()
  camera.value = next
  if (noCamera.value) {
    say(noCamera.value, true)
    return
  }
  const mine = ++ticket
  say('Reading the camera over DVRIP…')
  const [aboutReply, timeReply, languageReply, usersReply] = await Promise.all([
    request(withDevice('')),
    request(withDevice('/time')),
    request(withDevice('/language')),
    request(withDevice('/users')),
  ])
  if (mine !== ticket) return
  if (expired(aboutReply, timeReply, languageReply, usersReply)) return
  apply(about, 'device info', aboutReply)
  apply(clock, 'time', timeReply)
  apply(language, 'language', languageReply)
  apply(users, 'users', usersReply)
  if (language.data) language.pick = language.data.current || ''
  if (!aboutReply.ok && !timeReply.ok && !languageReply.ok && !usersReply.ok) say('The camera answered nothing: ' + failureText(aboutReply), true)
  else say('Read from the camera just now.')
}

async function run(job: string, work: () => Promise<void>) {
  if (busy.value) {
    say('Wait for ' + busy.value + ' to finish.')
    return
  }
  busy.value = job
  try {
    await work()
  } finally {
    busy.value = ''
  }
}

const syncTime = () =>
  run('time sync', async () => {
    say('Writing this computer’s time to the camera…')
    const rule = dstRule()
    const query: Record<string, string> = { time: localTime(), offset: String(baseOffsetMinutes()), dst: rule.on ? 'on' : 'off' }
    if (rule.on) Object.assign(query, { dst_start: rule.start, dst_end: rule.end })
    const result = await request(withDevice('/time', query), { method: 'POST' })
    if (expired(result)) return
    if (!result.ok) return say('Time sync failed: ' + failureText(result), true)
    clock.data = result.body
    say('Camera time now reads ' + result.body.device_time + ' (' + zoneLabel(result.body.minutes_east) + ').')
  })

const saveLanguage = () =>
  run('language', async () => {
    say('Writing the camera language…')
    const result = await request(withDevice('/language', { value: language.pick }), { method: 'PUT' })
    if (expired(result)) return
    if (!result.ok) return say('Language write failed: ' + failureText(result), true)
    language.data = result.body
    say('The camera language reads back as ' + result.body.current + '.')
  })

const checkFirmware = () =>
  run('firmware check', async () => {
    say('Asking the camera and the upgrade server…')
    const asked = camera.value
    const result = await request(withDevice('/firmware'))
    if (expired(result) || asked !== camera.value) return
    apply(firmware, 'firmware', result)
    firmware.camera = asked
    say(result.ok ? (result.body.server_check ? result.body.server_check.detail : result.body.server_check_error) : 'Firmware check failed: ' + failureText(result), !result.ok)
  })

const startUpgrade = () =>
  run('upgrade', async () => {
    if (startReason.value) return say(startReason.value, true)
    const offer = offers.value[0] || {}
    const result = await request(withDevice('/firmware/start'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ confirm: firmware.word, file_name: offer.file_name || '', version_date: offer.date || '' }),
    })
    if (expired(result)) return
    say(result.ok ? result.body.detail : 'Upgrade did not start: ' + failureText(result), !result.ok)
  })

const uploadFirmware = () =>
  run('upload', async () => {
    if (!firmware.file) return
    say('Sending ' + firmware.file.name + ' to the camera…')
    const result = await request(withDevice('/firmware/file', { confirm: firmware.fileWord }), {
      method: 'POST',
      headers: { 'content-type': 'application/octet-stream' },
      body: firmware.file,
    })
    if (expired(result)) return
    say(result.ok ? result.body.detail : 'Upload failed: ' + failureText(result), !result.ok)
  })

const checkNetwork = () =>
  run('network check', async () => {
    say('The camera is pinging from its own network…')
    const asked = camera.value
    const result = await request(withDevice('/network'))
    if (expired(result) || asked !== camera.value) return
    apply(network, 'network', result)
    say(result.ok ? 'Network check finished.' : 'Network check failed: ' + failureText(result), !result.ok)
  })

const restart = () =>
  run('restart', async () => {
    const result = await request(withDevice('/reboot', { confirm: 'reboot' }), { method: 'POST' })
    if (expired(result)) return
    danger.restart = false
    say(result.ok ? result.body.detail + '. It answers again after about a minute.' : 'Restart failed: ' + failureText(result), !result.ok)
  })

const reset = () =>
  run('reset', async () => {
    const result = await request(withDevice('/defaults', { confirm: danger.reset }), { method: 'POST' })
    if (expired(result)) return
    danger.reset = ''
    say(result.ok ? result.body.detail : 'Reset failed: ' + failureText(result), !result.ok)
  })

function pickFile(event: Event) {
  const input = event.target as HTMLInputElement
  firmware.file = input.files && input.files[0] ? input.files[0] : null
}

watch(
  () => props.shown,
  (shown) => {
    if (shown) load()
  },
)

watch(
  () => settingsUi.reload.system,
  () => {
    if (props.shown) load()
  },
)
</script>

<template>
  <header class="set-head">
    <div>
      <h2 id="h-system" class="set-h" tabindex="-1">System</h2>
      <p class="set-sub">Identity, clock, language, firmware, network health and users as the camera reports them.</p>
    </div>
  </header>
  <Teleport defer to="#savebar-note">
    <p v-show="shown" :class="['set-status', { 'warn-text': status.bad }]" data-testid="system-status" role="status">
      <Icon v-if="status.bad" name="warning" small /><span>{{ status.text }}</span>
    </p>
  </Teleport>

  <div class="set-cols">
    <div class="set-col">
      <div class="set-card" data-testid="system-about">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Device</h3></div>
        </div>
        <p v-if="about.error" class="hint warn-text">{{ about.error }}</p>
        <template v-else-if="about.data">
          <div class="row"><div class="lab">Model</div><div class="ctl-col"><p class="val">{{ about.data.summary.model || 'unknown' }}</p></div></div>
          <div class="row"><div class="lab">Serial</div><div class="ctl-col"><p class="val"><span class="code">{{ about.data.summary.serial || 'unknown' }}</span></p></div></div>
          <div class="row"><div class="lab">Firmware</div><div class="ctl-col"><p class="val"><span class="code">{{ about.data.summary.firmware || 'unknown' }}</span></p></div></div>
          <div class="row"><div class="lab">Build time</div><div class="ctl-col"><p class="val">{{ about.data.summary.build_time || 'unknown' }}</p></div></div>
          <div class="row"><div class="lab">Hardware</div><div class="ctl-col"><p class="val"><span class="code">{{ about.data.summary.hardware || 'unknown' }}</span></p></div></div>
          <div class="row"><div class="lab">Run time</div><div class="ctl-col"><p class="val">{{ runTime(about.data.summary.uptime_minutes) }}</p></div></div>
          <div class="row">
            <div class="lab">Extended info</div>
            <div class="ctl-col">
              <p v-if="about.data.info_ex" class="val"><span class="code">{{ JSON.stringify(about.data.info_ex) }}</span></p>
              <p v-else class="hint">{{ about.data.info_ex_error }}</p>
            </div>
          </div>
          <div class="row">
            <div class="lab">Abilities<span class="hint">{{ about.data.abilities.length }} flags the camera reports as on</span></div>
            <div class="ctl-col">
              <details><summary>Show the list</summary><p class="hint code">{{ about.data.abilities.join(', ') }}</p></details>
            </div>
          </div>
        </template>
        <p v-else class="hint">Not read yet.</p>
      </div>

      <div class="set-card" data-testid="system-time">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Time</h3></div>
        </div>
        <p v-if="clock.error" class="hint warn-text">{{ clock.error }}</p>
        <template v-else-if="clock.data">
          <div class="row"><div class="lab">Camera time</div><div class="ctl-col"><p class="val">{{ clock.data.device_time || 'unknown' }}</p></div></div>
          <div class="row"><div class="lab">Time zone</div><div class="ctl-col"><p class="val">{{ zoneLabel(clock.data.minutes_east) }}</p></div></div>
          <div class="row">
            <div class="lab">Daylight saving</div>
            <div class="ctl-col"><p class="val">{{ (clock.data.location && clock.data.location.DSTRule) || 'unknown' }}</p></div>
          </div>
          <div class="row">
            <div class="lab">Date format</div>
            <div class="ctl-col"><p class="val">{{ (clock.data.location && clock.data.location.DateFormat) || 'unknown' }} · {{ (clock.data.location && clock.data.location.TimeFormat) || '' }}</p></div>
          </div>
          <div class="set-card-f">
            <p class="hint">
              Writes this computer’s clock, its standard time zone ({{ zoneLabel(baseOffsetMinutes()) }}) and its daylight-saving rule to the camera, as the phone app does.
              <template v-if="noCamera">{{ noCamera }}</template>
            </p>
            <button class="btn btn-fill" type="button" data-testid="system-sync-time" :disabled="!!noCamera" :aria-busy="busy === 'time sync'" @click="syncTime">Sync with this computer</button>
          </div>
        </template>
        <p v-else class="hint">Not read yet.</p>
      </div>

      <div class="set-card" data-testid="system-language">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Camera language</h3></div>
        </div>
        <p v-if="language.error" class="hint warn-text">{{ language.error }}</p>
        <div v-else-if="language.data" class="row">
          <label class="lab" for="system-language-pick">Language<span v-if="languageReason" class="hint">{{ languageReason }}</span></label>
          <div class="ctl-col">
            <select id="system-language-pick" v-model="language.pick">
              <option v-for="item in language.data.available" :key="item" :value="item">{{ item }}</option>
            </select>
            <button class="btn btn-fill" type="button" data-testid="system-save-language" :disabled="!!languageReason" :aria-busy="busy === 'language'" @click="saveLanguage">Save language</button>
          </div>
        </div>
        <p v-else class="hint">Not read yet.</p>
      </div>
    </div>

    <div class="set-col">
      <div class="set-card" data-testid="system-firmware">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">Firmware</h3>
            <p class="set-card-d">Asks the camera and the upgrade server for a newer build.</p>
          </div>
          <div class="set-card-a">
            <button class="btn" type="button" data-testid="system-check-firmware" :disabled="!!noCamera" :aria-busy="busy === 'firmware check'" @click="checkFirmware">Check for update</button>
          </div>
        </div>
        <p v-if="noCamera" class="hint">{{ noCamera }}</p>
        <p v-if="firmware.error" class="hint warn-text">{{ firmware.error }}</p>
        <template v-else-if="firmware.data">
          <div class="row">
            <div class="lab">Upgrade server</div>
            <div class="ctl-col">
              <p class="val" data-testid="system-firmware-server">{{ firmware.data.server_check ? firmware.data.server_check.detail : firmware.data.server_check_error }}</p>
              <p v-for="offer in offers" :key="offer.file_name" class="hint">{{ offer.file_name }} · {{ offer.date }}</p>
            </div>
          </div>
          <div class="row">
            <div class="lab">Camera check</div>
            <div class="ctl-col">
              <p class="val" data-testid="system-firmware-device">{{ deviceCheckLine }}</p>
              <TechDetails data-testid="system-firmware-device-details" :text="firmware.data.device_check ? JSON.stringify(firmware.data.device_check) : firmware.data.device_check_error || ''" />
            </div>
          </div>
        </template>
      </div>

      <div class="set-card" data-testid="system-network">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t">Network health</h3>
            <p class="set-card-d">The camera pings from its own network and reports cloud, NAT and Wi-Fi state.</p>
          </div>
          <div class="set-card-a">
            <button class="btn" type="button" data-testid="system-check-network" :disabled="!!noCamera" :aria-busy="busy === 'network check'" @click="checkNetwork">Run network check</button>
          </div>
        </div>
        <p v-if="noCamera" class="hint">{{ noCamera }}</p>
        <p v-if="network.error" class="hint warn-text">{{ network.error }}</p>
        <template v-else-if="network.data">
          <div class="row stack">
            <table class="tbl">
              <thead><tr><th scope="col">Host</th><th scope="col">Reachable</th><th scope="col">Average</th><th scope="col">Lost</th></tr></thead>
              <tbody>
                <tr v-for="ping in (network.data.pings as Ping[])" :key="ping.url">
                  <th scope="row" class="code">{{ ping.url }}</th>
                  <td>{{ ping.reachable ? 'yes' : 'no' }}</td>
                  <td>{{ ping.average_ms === null || ping.average_ms === undefined ? '?' : ping.average_ms + ' ms' }}</td>
                  <td>{{ ping.lost_percent === null || ping.lost_percent === undefined ? '?' : ping.lost_percent + ' % of ' + ping.sent }}</td>
                </tr>
              </tbody>
            </table>
            <p v-for="line in network.data.ping_errors" :key="line" class="hint warn-text">{{ line }}</p>
          </div>
          <div class="row"><div class="lab">Cloud status</div><div class="ctl-col"><p class="val code">{{ network.data.cloud_status ? JSON.stringify(network.data.cloud_status) : network.data.cloud_status_error }}</p></div></div>
          <div class="row"><div class="lab">NAT</div><div class="ctl-col"><p class="val code">{{ network.data.nat ? JSON.stringify(network.data.nat) : network.data.nat_error }}</p></div></div>
          <div class="row"><div class="lab">Wi-Fi signal</div><div class="ctl-col"><p class="val">{{ network.data.wifi ? JSON.stringify(network.data.wifi) : network.data.wifi_error }}</p></div></div>
        </template>
      </div>

      <div class="set-card" data-testid="system-users">
        <div class="set-card-h">
          <div><h3 class="set-card-t">Users</h3></div>
        </div>
        <p v-if="users.error" class="hint warn-text">{{ users.error }}</p>
        <template v-else-if="users.data">
          <div class="row stack">
            <table class="tbl">
              <thead><tr><th scope="col">Name</th><th scope="col">Group</th><th scope="col">Built in</th><th scope="col">Rights</th></tr></thead>
              <tbody>
                <tr v-for="user in (users.data.users as User[])" :key="user.name">
                  <th scope="row">{{ user.name }}</th>
                  <td>{{ user.group || '?' }}</td>
                  <td>{{ user.reserved ? 'yes' : 'no' }}</td>
                  <td>{{ user.authorities }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <div class="row">
            <div class="lab">Password recovery</div>
            <div class="ctl-col"><p class="val">{{ users.data.recovery ? JSON.stringify(users.data.recovery) : users.data.recovery_error }}</p></div>
          </div>
          <p class="hint" data-testid="system-users-note">
            Change the camera password from the camera list (Device names, groups, sharing, passwords, then Change through the cloud). This page reads device users only: a
            user or password write over DVRIP is not offered because it cannot be tried on the camera and put back.
          </p>
        </template>
        <p v-else class="hint">Not read yet.</p>
      </div>
    </div>
  </div>

  <div class="set-stack">
    <div class="set-card danger" data-testid="system-danger">
      <div class="set-card-h">
        <div><h3 class="set-card-t"><Icon name="warning" small />Danger zone</h3></div>
      </div>
      <div class="row">
        <label class="lab" for="system-upgrade-word">Install from the cloud<span class="hint">The camera downloads the offered firmware, flashes it and restarts. It is offline until the install finishes.</span></label>
        <div class="ctl-col">
          <input id="system-upgrade-word" v-model="firmware.word" type="text" autocomplete="off" :placeholder="UPGRADE_WORD">
          <button class="btn danger" type="button" data-testid="system-start-upgrade" :disabled="!!startReason" :aria-busy="busy === 'upgrade'" @click="startUpgrade">Install update</button>
          <p v-if="startReason" class="hint" data-testid="system-start-reason">{{ startReason }}</p>
        </div>
      </div>
      <div class="row">
        <label class="lab" for="system-upgrade-file">Install from a file<span class="hint">The camera flashes the file you choose and restarts. A wrong file can leave it unusable.</span></label>
        <div class="ctl-col">
          <input id="system-upgrade-file" type="file" @change="pickFile">
          <input v-model="firmware.fileWord" type="text" autocomplete="off" aria-label="Type UPGRADE to confirm" :placeholder="UPGRADE_WORD">
          <button class="btn danger" type="button" data-testid="system-upload-firmware" :disabled="!!fileReason" :aria-busy="busy === 'upload'" @click="uploadFirmware">Send file to camera</button>
          <p v-if="fileReason" class="hint">{{ fileReason }}</p>
        </div>
      </div>
      <div class="row">
        <div class="lab">Restart<span class="hint">Every stream drops; the camera answers again after about a minute.</span></div>
        <div class="ctl-col">
          <label class="switchrow"><input v-model="danger.restart" type="checkbox"><span>Yes, restart the camera now</span></label>
          <button class="btn danger" type="button" data-testid="system-restart" :disabled="!!restartReason" :aria-busy="busy === 'restart'" @click="restart">Restart camera</button>
          <p v-if="restartReason" class="hint">{{ restartReason }}</p>
        </div>
      </div>
      <div class="row">
        <label class="lab" for="system-reset-word">Factory defaults<span class="hint">Erases every setting on the camera. Recordings stay.</span></label>
        <div class="ctl-col">
          <input id="system-reset-word" v-model="danger.reset" type="text" autocomplete="off" :placeholder="RESET_WORD">
          <button class="btn danger" type="button" data-testid="system-reset" :disabled="!!resetReason" :aria-busy="busy === 'reset'" @click="reset">Restore factory defaults</button>
          <p v-if="resetReason" class="hint" data-testid="system-reset-reason">{{ resetReason }}</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
details summary {
  min-height: var(--hit);
  display: flex;
  align-items: center;
  justify-content: flex-end;
  cursor: pointer;
  font-size: var(--t-13);
  color: var(--accent);
}
details > p {
  margin: var(--s2) 0 0;
  overflow-wrap: anywhere;
}
</style>
