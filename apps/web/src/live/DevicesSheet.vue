<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs'
import { api } from '../lib/api'
import { sessionLost } from '../legacy/app.js'
import { addLauncher, live, menuOpen } from './store'

type Device = { uuid: string; nickname: string; group?: string; favourite?: boolean; administrator?: boolean; shared_accounts?: number }
type Attributes = { devices?: Device[]; groups?: string[] }
type Reply = { message?: string; resolved?: string; materialised?: boolean; data?: any } | null
type Action = 'rename' | 'save' | 'share' | 'check' | 'cancel' | 'accept' | 'password'

const BUSY: Record<Action, string> = {
  rename: 'Renaming…',
  save: 'Saving…',
  share: 'Sending…',
  check: 'Checking…',
  cancel: 'Cancelling…',
  accept: 'Accepting…',
  password: 'Changing…',
}

const open = computed(() => live.sheet === 'devices')
const sheet = ref<HTMLElement>()
const closeButton = ref<HTMLButtonElement>()
const rows: (HTMLButtonElement | null)[] = []
const list = reactive({ phase: 'idle' as 'idle' | 'loading' | 'ok' | 'error', devices: [] as Device[], groups: [] as string[], error: '' })
const chosen = ref('')
const tab = ref('name')
const form = reactive({ name: '', group: '', favourite: false, shareTo: '', key: '', sharedDevice: '', accountPassword: '', oldPassword: '', newPassword: '' })
const action = reactive({ busy: '' as Action | '', message: '', kind: '' as '' | 'ok' | 'bad' })

const launcher = addLauncher({
  id: 'device-admin-open',
  label: 'Devices',
  icon: 'cctv',
  slot: 'start',
  title: 'Device names, groups, sharing, passwords',
  pressed: false,
  controls: 'device-admin',
  onClick: () => (open.value ? close() : show()),
})

watch(open, (shown) => (launcher.pressed = shown))

const device = computed(() => list.devices.find((entry) => entry.uuid === chosen.value) || null)
const shownTab = computed(() => (device.value ? tab.value : 'share'))

function sentence(error: unknown) {
  const text = error instanceof Error ? error.message : String(error)
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function tags(entry: Device) {
  const found: string[] = []
  if (entry.favourite) found.push('favourite')
  if (entry.group) found.push(entry.group)
  if (entry.administrator) found.push('administrator')
  if (entry.shared_accounts) found.push(entry.shared_accounts + ' shared')
  return entry.uuid.slice(-6) + (found.length ? ' · ' + found.join(' · ') : '')
}

function fill() {
  const entry = device.value
  if (!entry) return
  form.name = entry.nickname
  form.group = entry.group || ''
  form.favourite = Boolean(entry.favourite)
}

function choose(uuid: string) {
  if (chosen.value === uuid) return
  chosen.value = uuid
  fill()
}

function onRowKey(event: KeyboardEvent, index: number) {
  const step = event.key === 'ArrowDown' ? 1 : event.key === 'ArrowUp' ? -1 : 0
  if (!step) return
  event.preventDefault()
  const next = (index + step + list.devices.length) % list.devices.length
  choose(list.devices[next].uuid)
  rows[next]?.focus()
}

async function refresh() {
  if (list.phase !== 'ok') list.phase = 'loading'
  try {
    const payload = await api<Attributes>('/api/cloud/device-attributes')
    list.devices = payload.devices || []
    list.groups = payload.groups || []
    if (!list.devices.some((entry) => entry.uuid === chosen.value)) chosen.value = list.devices.length ? list.devices[0].uuid : ''
    list.error = ''
    list.phase = 'ok'
    fill()
  } catch (error) {
    if (sessionLost(error)) return close(false)
    list.error = sentence(error)
    list.phase = 'error'
  }
}

async function show() {
  live.sheet = 'devices'
  const loading = refresh()
  await nextTick()
  closeButton.value?.focus()
  await loading
  if (!open.value || !sheet.value?.contains(document.activeElement)) return
  await nextTick()
  const index = list.devices.findIndex((entry) => entry.uuid === chosen.value)
  if (index >= 0 && document.activeElement === closeButton.value) rows[index]?.focus()
}

function close(returnFocus = true) {
  if (live.sheet === 'devices') live.sheet = ''
  if (returnFocus) document.getElementById('device-admin-open')?.focus()
}

function onEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented || !open.value || menuOpen()) return
  event.preventDefault()
  close()
}

function say(message: string, kind: 'ok' | 'bad') {
  action.message = message
  action.kind = kind
}

async function run(name: Action, work: (uuid: string) => Promise<string>, needsCamera = true) {
  if (action.busy) return
  const uuid = chosen.value
  if (needsCamera && !uuid) return say('Pick a camera first', 'bad')
  action.busy = name
  action.message = ''
  action.kind = ''
  try {
    const result = await work(uuid)
    await refresh()
    say(result, 'ok')
  } catch (error) {
    if (sessionLost(error)) return close(false)
    say(sentence(error), 'bad')
  } finally {
    action.busy = ''
  }
}

const post = (path: string, body: unknown) => api<Reply>(path, { method: 'POST', body: JSON.stringify(body) })

const rename = () =>
  run('rename', async (uuid) => {
    const nickname = form.name.trim()
    if (!nickname) throw new Error('the name is empty')
    await post('/api/cloud/device/rename', { uuid, nickname })
    return 'Renamed to ' + nickname
  })

const save = () =>
  run('save', async (uuid) => {
    const group = await post('/api/cloud/device/group', { uuid, group: form.group.trim() })
    const favourite = await post('/api/cloud/device/favourite', { uuid, favourite: form.favourite })
    return sentence(favourite?.message || group?.message || 'saved on this server')
  })

const share = () =>
  run('share', async (uuid) => {
    const target = form.shareTo.trim()
    if (!target) throw new Error('the receiving account is empty')
    const body = target.includes('@') ? { uuids: [uuid], account: target } : { uuids: [uuid], acceptId: target }
    const reply = await post('/api/cloud/device/share', body)
    const data = reply?.data
    const key = data && (data.transKey || data.key || data.transInfos?.[0]?.transKey)
    if (key) form.key = key
    const invited = reply?.resolved ? ' to ' + reply.resolved : ''
    return key ? 'Invitation ' + key + ' sent' + invited : 'The cloud accepted the invitation' + invited
  })

const check = () =>
  run('check', async (uuid) => {
    const reply = await api<Reply>('/api/cloud/device/share/pending?uuids=' + encodeURIComponent(uuid))
    const pending = ((reply?.data as any[]) || []).filter((entry) => entry && entry.trans)
    if (!pending.length) return 'No pending invitation for this camera'
    form.key = pending[0].transKey || form.key
    return pending.length + (pending.length === 1 ? ' pending invitation' : ' pending invitations')
  })

const cancel = () =>
  run('cancel', async (uuid) => {
    const key = form.key.trim()
    if (!key) throw new Error('an invitation key is required')
    await post('/api/cloud/device/share/cancel', { key, uuids: [uuid] })
    return 'Cancelled ' + key
  })

const accept = () =>
  run('accept', async () => {
    const shared = form.sharedDevice.trim()
    const key = form.key.trim()
    if (shared) {
      if (!form.accountPassword) throw new Error('the receiver needs its own account password')
      const reply = await post('/api/cloud/device/share/accept', { devId: shared, password: form.accountPassword })
      if (!reply || reply.materialised !== true) throw new Error('the camera did not land in this account')
      form.accountPassword = ''
      return 'Accepted ' + shared + '; it is in this account\'s device list'
    }
    if (!key) throw new Error('paste an invitation key or a shared camera serial first')
    await post('/api/cloud/device/share/accept', { key })
    return 'Transfer ' + key + ' acknowledged; the camera lands after Accept with its serial and your password'
  }, false)

const password = () =>
  run('password', async (uuid) => {
    const reply = await post('/api/cloud/device/password', { uuid, password: form.oldPassword, newPassword: form.newPassword })
    form.oldPassword = ''
    form.newPassword = ''
    return sentence(reply?.message || 'the cloud changed the device password')
  })

const label = (name: Action, idle: string) => (action.busy === name ? BUSY[name] : idle)

function forgetForm() {
  Object.assign(form, { name: '', group: '', favourite: false, shareTo: '', key: '', sharedDevice: '', accountPassword: '', oldPassword: '', newPassword: '' })
  Object.assign(action, { busy: '', message: '', kind: '' })
  Object.assign(list, { phase: 'idle', devices: [], groups: [], error: '' })
  chosen.value = ''
}

function syncScreen() {
  if (!open.value) return
  const watchView = document.getElementById('watch')
  if (watchView && !watchView.hidden) return
  close(false)
}

onMounted(() => {
  const screens = new MutationObserver(() => {
    syncScreen()
    const signin = document.getElementById('signin')
    if (signin && !signin.hidden) forgetForm()
  })
  for (const id of ['watch', 'signin']) {
    const screen = document.getElementById(id)
    if (screen) screens.observe(screen, { attributes: true, attributeFilter: ['hidden'] })
  }
  document.addEventListener('keydown', onEscape)
  ;(window as unknown as { proviewDeviceAdmin: unknown }).proviewDeviceAdmin = { open: show, close, refresh }
})
</script>

<template>
  <section
    id="device-admin"
    ref="sheet"
    class="menu sheet"
    data-testid="device-admin"
    role="dialog"
    aria-labelledby="device-admin-title"
    :hidden="!open"
  >
    <div class="sheet-h">
      <h2 id="device-admin-title">Devices on this account</h2>
      <button ref="closeButton" class="iconbtn" type="button" aria-label="Close" data-testid="device-admin-close" @click="close()">
        <Icon name="x" />
      </button>
    </div>
    <div class="devices-body">
      <p v-if="list.phase === 'error'" class="err" role="alert" data-testid="device-admin-error">
        <Icon name="warning" small /><span>{{ list.error }}</span>
      </p>
      <button v-if="list.phase === 'error'" class="btn btn-line" type="button" data-testid="device-admin-retry" @click="refresh">Try again</button>
      <p v-else-if="list.phase === 'loading' && !list.devices.length" class="hint" data-testid="device-admin-loading">Loading the cameras on this account…</p>
      <p v-else-if="list.phase === 'ok' && !list.devices.length" class="hint" data-testid="device-admin-empty">No camera on this account yet.</p>
      <div v-if="list.devices.length" id="device-admin-list" class="devices-list" role="radiogroup" aria-label="Camera">
        <button
          v-for="(entry, index) in list.devices"
          :key="entry.uuid"
          :ref="(el) => (rows[index] = el as HTMLButtonElement | null)"
          type="button"
          class="menu-row"
          role="radio"
          data-testid="device-admin-row"
          :aria-checked="entry.uuid === chosen"
          :tabindex="entry.uuid === chosen ? 0 : -1"
          @click="choose(entry.uuid)"
          @keydown="onRowKey($event, index)"
        >
          <span class="col">
            <span class="name">{{ entry.nickname }}</span>
            <span class="sub">{{ tags(entry) }}</span>
          </span>
          <Icon name="check" small />
        </button>
      </div>
      <Tabs v-if="list.phase !== 'loading' || list.devices.length" :model-value="shownTab" @update:model-value="(value) => (tab = String(value))">
        <TabsList aria-label="What to change">
          <TabsTrigger value="name" data-testid="device-admin-tab-name" :disabled="!device">Name</TabsTrigger>
          <TabsTrigger value="share" data-testid="device-admin-tab-share">Sharing</TabsTrigger>
          <TabsTrigger value="password" data-testid="device-admin-tab-password" :disabled="!device">Password</TabsTrigger>
        </TabsList>
        <TabsContent value="name">
          <section class="devices-group">
            <h3>Name in the cloud</h3>
            <label class="field"><span class="lab">Name</span><input id="device-admin-name" v-model="form.name" type="text" placeholder="Front gate"></label>
            <div class="sheet-actions">
              <button class="btn btn-fill" type="button" :aria-busy="action.busy === 'rename'" @click="rename">{{ label('rename', 'Rename through the cloud') }}</button>
            </div>
          </section>
          <section class="devices-group">
            <h3>Group and favourite</h3>
            <label class="field">
              <span class="lab">Group</span>
              <input id="device-admin-group" v-model="form.group" type="text" placeholder="gate, garage, porch">
              <span id="device-admin-groups" class="hint">Kept on this server; the cloud account has no group field.{{ list.groups.length ? ' In use: ' + list.groups.join(', ') + '.' : '' }}</span>
            </label>
            <label class="switchrow"><input id="device-admin-favourite" v-model="form.favourite" type="checkbox" role="switch">Favourite</label>
            <div class="sheet-actions">
              <button class="btn btn-line" type="button" :aria-busy="action.busy === 'save'" @click="save">{{ label('save', 'Save group and favourite') }}</button>
            </div>
          </section>
        </TabsContent>
        <TabsContent value="share">
          <section v-if="device" class="devices-group">
            <h3>Share this camera</h3>
            <label class="field">
              <span class="lab">Share to account</span>
              <input id="device-admin-accept" v-model="form.shareTo" type="text" placeholder="the other account email or its account id">
              <span class="hint">An email resolves to its account id first.</span>
            </label>
            <div class="sheet-actions">
              <button class="btn btn-fill" type="button" :aria-busy="action.busy === 'share'" @click="share">{{ label('share', 'Send the invitation') }}</button>
            </div>
            <label class="field"><span class="lab">Invitation key</span><input id="device-admin-key" v-model="form.key" type="text" placeholder="transKey from the invitation"></label>
            <div class="sheet-actions">
              <button class="btn btn-line" type="button" :aria-busy="action.busy === 'check'" @click="check">{{ label('check', 'Check this camera') }}</button>
              <button class="btn btn-line" type="button" :aria-busy="action.busy === 'cancel'" @click="cancel">{{ label('cancel', 'Cancel invitation') }}</button>
            </div>
          </section>
          <section class="devices-group">
            <h3>Accept a shared camera</h3>
            <label v-if="!device" class="field"><span class="lab">Invitation key</span><input id="device-admin-key" v-model="form.key" type="text" placeholder="transKey from the invitation"></label>
            <label class="field">
              <span class="lab">Camera shared with you</span>
              <input id="device-admin-share-device" v-model="form.sharedDevice" type="text" placeholder="camera serial shared with you">
            </label>
            <label class="field">
              <span class="lab">Your account password</span>
              <input id="device-admin-share-pass" v-model="form.accountPassword" type="password" placeholder="your own account password" autocomplete="current-password">
              <span class="hint">Sent once inside the sealed body; the share service checks it.</span>
            </label>
            <div class="sheet-actions">
              <button class="btn btn-line" type="button" :aria-busy="action.busy === 'accept'" @click="accept">{{ label('accept', 'Accept') }}</button>
            </div>
          </section>
        </TabsContent>
        <TabsContent value="password">
          <section class="devices-group">
            <h3>Device password</h3>
            <p class="hint">Changed through the account: the camera picks it up on its next cloud call, and the server keeps the new one for its own camera routes.</p>
            <label class="field"><span class="lab">Current</span><input id="device-admin-old" v-model="form.oldPassword" type="password" placeholder="current device password" autocomplete="off"></label>
            <label class="field"><span class="lab">New</span><input id="device-admin-new" v-model="form.newPassword" type="password" placeholder="new device password" autocomplete="new-password"></label>
            <div class="sheet-actions">
              <button class="btn btn-fill" type="button" :aria-busy="action.busy === 'password'" @click="password">{{ label('password', 'Change through the cloud') }}</button>
            </div>
          </section>
        </TabsContent>
      </Tabs>
    </div>
    <p
      id="device-admin-status"
      :class="['devices-status', action.kind]"
      data-testid="device-admin-status"
      :role="action.kind === 'bad' ? 'alert' : 'status'"
    ><template v-if="action.message"><Icon :name="action.kind === 'bad' ? 'warning' : 'check'" small /><span>{{ action.message }}</span></template></p>
  </section>
</template>
