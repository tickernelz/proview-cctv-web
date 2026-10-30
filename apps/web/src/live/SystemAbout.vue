<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { sessionLost } from '../app/shell'
import { state } from '../app/state'

type Reply = { ok: boolean; status: number; body: any }
type Ping = { url: string; reachable: boolean; average_ms?: number | null; lost_percent?: number | null; sent: number }
type User = { name: string; group?: string | null; memo?: string | null; reserved?: boolean | null; authorities: number }

const props = defineProps<{ host: HTMLElement }>()

const BASE = '/api/device/system'
const RESET_WORD = 'RESET'
const UPGRADE_WORD = 'UPGRADE'

const status = reactive({ text: 'Not loaded yet.', bad: false })
const busy = ref('')
const about = reactive({ data: null as any, error: '' })
const clock = reactive({ data: null as any, error: '' })
const language = reactive({ data: null as any, error: '', pick: '' })
const firmware = reactive({ data: null as any, error: '', word: '', file: null as File | null, fileWord: '' })
const network = reactive({ data: null as any, error: '' })
const users = reactive({ data: null as any, error: '' })
const password = reactive({ user: 'admin', old: '', next: '' })
const rename = reactive({ user: 'admin', name: '' })
const recovery = reactive({ answers: '', user: 'admin', password: '', reply: null as any })
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
  if (!firmware.data) return 'Check for an update first.'
  if (!offers.value.length) return 'The upgrade server offers no newer firmware for this camera.'
  if (!(about.data && about.data.gates && about.data.gates.cloud_upgrade)) return 'The camera does not report cloud upgrade support (OtherFunction/SupportCfgCloudupgrade).'
  if (firmware.word !== UPGRADE_WORD) return 'Type ' + UPGRADE_WORD + ' to confirm.'
  return ''
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

async function load() {
  camera.value = currentCamera()
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
    const offset = String(-new Date().getTimezoneOffset())
    const result = await request(withDevice('/time', { time: localTime(), offset }), { method: 'POST' })
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
    const result = await request(withDevice('/firmware'))
    if (expired(result)) return
    apply(firmware, 'firmware', result)
    say(result.ok ? (result.body.server_check ? result.body.server_check.detail : result.body.server_check_error) : 'Firmware check failed: ' + failureText(result), !result.ok)
  })

const startUpgrade = () =>
  run('upgrade', async () => {
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
    const result = await request(withDevice('/network'))
    if (expired(result)) return
    apply(network, 'network', result)
    say(result.ok ? 'Network check finished.' : 'Network check failed: ' + failureText(result), !result.ok)
  })

async function postJson(path: string, body: unknown) {
  return request(withDevice(path), { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })
}

const changePassword = () =>
  run('password', async () => {
    const result = await postJson('/users/password', { user: password.user, old: password.old, new: password.next })
    if (expired(result)) return
    password.old = ''
    password.next = ''
    say(result.ok ? result.body.detail : 'Password change failed: ' + failureText(result), !result.ok)
  })

const renameUser = () =>
  run('rename', async () => {
    const result = await postJson('/users/rename', { user: rename.user, name: rename.name })
    if (expired(result)) return
    say(result.ok ? result.body.detail : 'Rename failed: ' + failureText(result), !result.ok)
    if (result.ok) await load()
  })

const recover = (step: 'questions' | 'answer' | 'password') =>
  run('recovery', async () => {
    const answers = recovery.answers
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
    const result = await postJson('/users/recover', { step, answers, user: recovery.user, password: recovery.password })
    if (expired(result)) return
    recovery.reply = result.ok ? result.body.reply : null
    say(result.ok ? 'The camera answered the ' + step + ' step.' : 'Recovery step failed: ' + failureText(result), !result.ok)
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

function open() {
  const page = window as unknown as { openSettings?: (section: string) => void; showSection?: (section: string, focus: boolean) => void }
  if (typeof page.openSettings === 'function') page.openSettings('system')
  else if (typeof page.showSection === 'function') page.showSection('system', true)
  else location.hash = '#settings/system'
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
    <a href="#settings/system" data-section="system" data-testid="system-link" @click.prevent="openFromNav">About device</a>
  </Teleport>
  <h2 id="h-system" class="set-h" tabindex="-1">About device</h2>
  <p class="set-sub">Identity, clock, language, firmware, network health and users as the camera reports them over DVRIP.</p>
  <div class="sys-body">
  <p :class="['hint', { 'warn-text': status.bad }]" data-testid="system-status" role="status">{{ status.text }}</p>

  <div data-testid="system-about">
    <p v-if="about.error" class="hint warn-text">{{ about.error }}</p>
    <div v-else-if="about.data">
      <h3 class="set-h">Device</h3>
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
        <div class="lab">Abilities<div class="hint">{{ about.data.abilities.length }} flags the camera reports as on</div></div>
        <div class="ctl-col">
          <details><summary>Show the list</summary><p class="hint code">{{ about.data.abilities.join(', ') }}</p></details>
        </div>
      </div>
    </div>
  </div>

  <div data-testid="system-time">
    <p v-if="clock.error" class="hint warn-text">{{ clock.error }}</p>
    <div v-else-if="clock.data">
      <h3 class="set-h">Time</h3>
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
      <button class="btn btn-fill" type="button" data-testid="system-sync-time" :disabled="!!noCamera" :aria-busy="busy === 'time sync'" @click="syncTime">Sync with this computer</button>
      <p class="hint">Writes this computer’s clock and time zone ({{ zoneLabel(-new Date().getTimezoneOffset()) }}) to the camera, as the phone app does.</p>
    </div>
  </div>

  <div data-testid="system-language">
    <p v-if="language.error" class="hint warn-text">{{ language.error }}</p>
    <div v-else-if="language.data">
      <h3 class="set-h">Camera language</h3>
      <div class="row">
        <label class="lab" for="system-language-pick">Language</label>
        <div class="ctl-col">
          <select id="system-language-pick" v-model="language.pick">
            <option v-for="item in language.data.available" :key="item" :value="item">{{ item }}</option>
          </select>
          <button class="btn btn-fill" type="button" data-testid="system-save-language" :disabled="!!languageReason" :aria-busy="busy === 'language'" @click="saveLanguage">Save language</button>
          <p v-if="languageReason" class="hint">{{ languageReason }}</p>
        </div>
      </div>
    </div>
  </div>

  <div data-testid="system-firmware">
    <h3 class="set-h">Firmware</h3>
    <button class="btn btn-line" type="button" data-testid="system-check-firmware" :disabled="!!noCamera" :aria-busy="busy === 'firmware check'" @click="checkFirmware">Check for update</button>
    <p v-if="firmware.error" class="hint warn-text">{{ firmware.error }}</p>
    <div v-else-if="firmware.data">
      <div class="row">
        <div class="lab">Upgrade server</div>
        <div class="ctl-col">
          <p class="val" data-testid="system-firmware-server">{{ firmware.data.server_check ? firmware.data.server_check.detail : firmware.data.server_check_error }}</p>
          <p v-for="offer in offers" :key="offer.file_name" class="hint">{{ offer.file_name }} · {{ offer.date }}</p>
        </div>
      </div>
      <div class="row">
        <div class="lab">Camera check</div>
        <div class="ctl-col"><p class="val">{{ firmware.data.device_check ? JSON.stringify(firmware.data.device_check) : firmware.data.device_check_error }}</p></div>
      </div>
    </div>
    <div class="row">
      <label class="lab" for="system-upgrade-word">Install from the cloud</label>
      <div class="ctl-col">
        <input id="system-upgrade-word" v-model="firmware.word" type="text" autocomplete="off" :placeholder="UPGRADE_WORD">
        <button class="btn btn-line" type="button" data-testid="system-start-upgrade" :disabled="!!startReason" :aria-busy="busy === 'upgrade'" @click="startUpgrade">Install update</button>
        <p v-if="startReason" class="hint" data-testid="system-start-reason">{{ startReason }}</p>
      </div>
    </div>
    <div class="row">
      <label class="lab" for="system-upgrade-file">Install from a file</label>
      <div class="ctl-col">
        <input id="system-upgrade-file" type="file" @change="pickFile">
        <input v-model="firmware.fileWord" type="text" autocomplete="off" aria-label="Type UPGRADE to confirm" :placeholder="UPGRADE_WORD">
        <button class="btn btn-line" type="button" data-testid="system-upload-firmware" :disabled="!!fileReason" :aria-busy="busy === 'upload'" @click="uploadFirmware">Send file to camera</button>
        <p v-if="fileReason" class="hint">{{ fileReason }}</p>
      </div>
    </div>
  </div>

  <div data-testid="system-network">
    <h3 class="set-h">Network check</h3>
    <button class="btn btn-line" type="button" data-testid="system-check-network" :disabled="!!noCamera" :aria-busy="busy === 'network check'" @click="checkNetwork">Run network check</button>
    <p v-if="network.error" class="hint warn-text">{{ network.error }}</p>
    <div v-else-if="network.data">
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
      <div class="row"><div class="lab">Cloud status</div><div class="ctl-col"><p class="val code">{{ network.data.cloud_status ? JSON.stringify(network.data.cloud_status) : network.data.cloud_status_error }}</p></div></div>
      <div class="row"><div class="lab">NAT</div><div class="ctl-col"><p class="val code">{{ network.data.nat ? JSON.stringify(network.data.nat) : network.data.nat_error }}</p></div></div>
      <div class="row"><div class="lab">Wi-Fi signal</div><div class="ctl-col"><p class="val">{{ network.data.wifi ? JSON.stringify(network.data.wifi) : network.data.wifi_error }}</p></div></div>
    </div>
  </div>

  <div data-testid="system-users">
    <p v-if="users.error" class="hint warn-text">{{ users.error }}</p>
    <div v-else-if="users.data">
      <h3 class="set-h">Device users</h3>
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
      <div class="row">
        <div class="lab">Change password</div>
        <div class="ctl-col">
          <label class="field"><span>User</span><input v-model="password.user" type="text" autocomplete="username"></label>
          <label class="field"><span>Current password</span><input v-model="password.old" type="password" autocomplete="current-password"></label>
          <label class="field"><span>New password</span><input v-model="password.next" type="password" autocomplete="new-password"></label>
          <button class="btn btn-line" type="button" data-testid="system-change-password" :disabled="!!noCamera || !password.next" :aria-busy="busy === 'password'" @click="changePassword">Change password</button>
          <p v-if="!password.next" class="hint">Enter a new password first. The app's saved device login must change with it.</p>
        </div>
      </div>
      <div class="row">
        <div class="lab">Rename user</div>
        <div class="ctl-col">
          <label class="field"><span>User</span><input v-model="rename.user" type="text"></label>
          <label class="field"><span>New name</span><input v-model="rename.name" type="text"></label>
          <button class="btn btn-line" type="button" data-testid="system-rename-user" :disabled="!!noCamera || !rename.name.trim()" :aria-busy="busy === 'rename'" @click="renameUser">Rename</button>
          <p v-if="!rename.name.trim()" class="hint">Enter the new name first.</p>
        </div>
      </div>
      <div class="row">
        <div class="lab">Password recovery<div class="hint">{{ users.data.recovery ? 'The camera reports ' + JSON.stringify(users.data.recovery) : users.data.recovery_error }}</div></div>
        <div class="ctl-col">
          <button class="btn btn-text" type="button" :disabled="!!noCamera" @click="recover('questions')">Load questions</button>
          <label class="field"><span>Answers, one per line</span><textarea v-model="recovery.answers" rows="2"></textarea></label>
          <button class="btn btn-text" type="button" :disabled="!!noCamera || !recovery.answers.trim()" @click="recover('answer')">Check answers</button>
          <label class="field"><span>New password</span><input v-model="recovery.password" type="password" autocomplete="new-password"></label>
          <button class="btn btn-text" type="button" :disabled="!!noCamera || !recovery.password" @click="recover('password')">Set new password</button>
          <p v-if="recovery.reply" class="hint code">{{ JSON.stringify(recovery.reply) }}</p>
        </div>
      </div>
    </div>
  </div>

  <div data-testid="system-danger">
    <h3 class="set-h">Restart and reset</h3>
    <div class="row">
      <div class="lab">Restart</div>
      <div class="ctl-col">
        <label class="switchrow"><input v-model="danger.restart" type="checkbox"><span>Yes, restart the camera now</span></label>
        <button class="btn btn-line" type="button" data-testid="system-restart" :disabled="!!restartReason" :aria-busy="busy === 'restart'" @click="restart">Restart camera</button>
        <p v-if="restartReason" class="hint">{{ restartReason }}</p>
      </div>
    </div>
    <div class="row">
      <label class="lab" for="system-reset-word">Factory defaults<div class="hint">Erases every setting on the camera. Recordings stay.</div></label>
      <div class="ctl-col">
        <input id="system-reset-word" v-model="danger.reset" type="text" autocomplete="off" :placeholder="RESET_WORD">
        <button class="btn btn-line" type="button" data-testid="system-reset" :disabled="!!resetReason" :aria-busy="busy === 'reset'" @click="reset">Restore factory defaults</button>
        <p v-if="resetReason" class="hint" data-testid="system-reset-reason">{{ resetReason }}</p>
      </div>
    </div>
  </div>
  </div>
</template>

<style scoped>
.sys-body {
  min-width: 0;
  overflow-wrap: anywhere;
}
.sys-body .tbl {
  width: 100%;
  table-layout: fixed;
}
.sys-body input,
.sys-body select,
.sys-body textarea {
  max-width: 100%;
  min-width: 0;
}
</style>
