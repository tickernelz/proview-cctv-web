<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs'
import { api, type ApiError } from '../lib/api'
import { openSettings, sessionLost } from '../app/shell'
import { loadDevices } from '../app/view'
import { live, menuOpen } from './store'
import { settingsUi } from '../settings/ui'

type Device = { uuid: string; nickname: string; group?: string; favourite?: boolean; administrator?: boolean; shared_accounts?: number }
type Attributes = { devices?: Device[]; groups?: string[] }
type Reply = { message?: string; resolved?: string; materialised?: boolean; verified?: boolean; also_missing?: { uuid: string; name: string }[]; data?: any } | null
type Said = { text: string; kind: 'ok' | 'bad' }
type Action = 'rename' | 'save' | 'share' | 'check' | 'cancel' | 'accept' | 'password' | 'permit' | 'shares' | 'perm' | 'revoke' | 'qr' | 'answer' | 'unbind'
type Permission = { id: string; label: string; default: boolean }
type ShareRow = { shareId: string; serial: string; account: string; userId: string; permissions: string[]; state: string; deviceName: string }
type QrReply = { link: string; svg: string; expiresAt: number; lifetimeSecs: number; permissions: string }

const BUSY: Record<Action, string> = {
  rename: 'Renaming…',
  save: 'Saving…',
  share: 'Sending…',
  check: 'Checking…',
  cancel: 'Cancelling…',
  accept: 'Accepting…',
  password: 'Changing…',
  permit: 'Sharing…',
  shares: 'Loading…',
  perm: 'Saving…',
  revoke: 'Revoking…',
  qr: 'Making…',
  answer: 'Answering…',
  unbind: 'Removing…',
}

const FALLBACK_PERMISSIONS: Permission[] = [
  { id: 'DP_PTZ', label: 'Pan and tilt', default: true },
  { id: 'DP_Intercom', label: 'Two-way talk', default: true },
  { id: 'DP_LocalStorage', label: 'Recordings on the card', default: true },
  { id: 'DP_ModifyConfig', label: 'Change settings', default: false },
  { id: 'DP_AlarmPush', label: 'Alarm messages', default: false },
  { id: 'DP_ViewCloudVideo', label: 'Cloud video', default: false },
]

const open = computed(() => live.sheet === 'devices')
const sheet = ref<HTMLElement>()
const closeButton = ref<HTMLButtonElement>()
const rows: (HTMLButtonElement | null)[] = []
let opener: HTMLElement | null = null
let openerId = ''
let origin = 'watch'
const list = reactive({ phase: 'idle' as 'idle' | 'loading' | 'ok' | 'error', devices: [] as Device[], groups: [] as string[], error: '' })
const chosen = ref('')
const tab = ref('name')
const form = reactive({ name: '', group: '', favourite: false, shareTo: '', transferTo: '', key: '', sharedDevice: '', accountPassword: '', oldPassword: '', newPassword: '', unbindConfirm: '' })
const transfer = reactive({ confirming: false })
const action = reactive({ busy: '' as Action | '', message: '', kind: '' as '' | 'ok' | 'bad' })
const permissions = ref<Permission[]>(FALLBACK_PERMISSIONS)
const granted = reactive(new Set<string>(FALLBACK_PERMISSIONS.filter((item) => item.default).map((item) => item.id)))
const shares = reactive({ mine: [] as ShareRow[], received: [] as ShareRow[], loaded: false, qr: null as QrReply | null })
const labelOf = (id: string) => permissions.value.find((item) => item.id === id)?.label || id
const shareStates: Record<string, string> = { '0': 'Not accepted yet', '1': 'Accepted', '2': 'Refused' }
const stateOf = (state: string) => shareStates[state] || state
const permitReason = computed(() => (!form.shareTo.trim() ? 'Type the other account email to share' : ''))
const qrImage = computed(() => (shares.qr ? 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(shares.qr.svg) : ''))

const device = computed(() => list.devices.find((entry) => entry.uuid === chosen.value) || null)
const shownTab = computed(() => (device.value ? tab.value : 'share'))

watch(
  () => open.value && list.phase === 'ok' && shownTab.value === 'share',
  (sharing) => {
    if (!sharing) return
    if (permissions.value === FALLBACK_PERMISSIONS) loadPermissions()
    loadReceived()
  },
)

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

async function show(from = '') {
  const active = document.activeElement
  openerId = from
  opener = !from && active instanceof HTMLElement && active !== document.body && !sheet.value?.contains(active) ? active : null
  origin = live.settings.shown ? 'settings' : 'watch'
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
  const back = openerId ? document.getElementById(openerId) : opener
  opener = null
  openerId = ''
  if (returnFocus && back?.isConnected && back.getClientRects().length) back.focus()
}

function addCamera() {
  close(false)
  openSettings('config', { focus: true })
}

watch(
  () => settingsUi.devices,
  () => show(settingsUi.devicesFrom),
)

function onEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented || !open.value || menuOpen()) return
  event.preventDefault()
  close()
}

function say(message: string, kind: 'ok' | 'bad') {
  action.message = message
  action.kind = kind
}

async function run(name: Action, work: (uuid: string) => Promise<string | Said>, needsCamera = true) {
  if (action.busy) return
  const uuid = chosen.value
  if (needsCamera && !uuid) return say('Pick a camera first', 'bad')
  action.busy = name
  action.message = ''
  action.kind = ''
  try {
    const result = await work(uuid)
    await refresh()
    if (typeof result === 'string') say(result, 'ok')
    else say(result.text, result.kind)
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

const unbind = () =>
  run('unbind', async (uuid) => {
    const confirm = form.unbindConfirm.trim()
    const name = device.value?.nickname || uuid
    let reply: Reply
    try {
      reply = await post('/api/devices/unbind', { sn: uuid, confirm })
    } catch (error) {
      const code = (error as ApiError).code
      if (code && error instanceof Error) error.message += ' (' + code + ')'
      throw error
    }
    form.unbindConfirm = ''
    await loadDevices()
    if (reply?.also_missing?.length || reply?.verified === false) return { text: sentence(reply.message || 'the removal could not be verified'), kind: 'bad' }
    return 'Removed ' + name + ' from this account'
  })

const transferReason = computed(() => (!form.transferTo.trim() ? 'Type the account that will own this camera' : ''))

function askTransfer() {
  if (transferReason.value) return
  transfer.confirming = true
}

const share = () =>
  run('share', async (uuid) => {
    const target = form.transferTo.trim()
    if (!target) throw new Error('the receiving account is empty')
    if (!transfer.confirming) throw new Error('confirm the transfer first')
    transfer.confirming = false
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

async function loadPermissions() {
  try {
    const list = await api<Permission[]>('/api/cloud/device/share/permissions')
    if (Array.isArray(list) && list.length) permissions.value = list
  } catch {
    return
  }
}

function toggleGrant(id: string) {
  if (granted.has(id)) granted.delete(id)
  else granted.add(id)
}

const permit = () =>
  run('permit', async (uuid) => {
    const target = form.shareTo.trim()
    if (!target) throw new Error('the receiving account is empty')
    const body = { uuid, permissions: [...granted], ...(target.includes('@') ? { account: target } : { acceptId: target }) }
    const reply = await post('/api/cloud/device/share/permitted', body)
    await loadShares(uuid)
    return 'Shared' + (reply?.resolved ? ' with ' + reply.resolved : '') + '; the other account accepts it in its app'
  })

async function loadReceived() {
  try {
    const received = await api<ShareRow[]>('/api/cloud/device/shares/received')
    if (Array.isArray(received)) shares.received = received
  } catch {
    return
  }
}

async function loadShares(uuid: string) {
  const [mine, received] = await Promise.all([
    api<ShareRow[]>('/api/cloud/device/shares?uuid=' + encodeURIComponent(uuid)),
    api<ShareRow[]>('/api/cloud/device/shares/received'),
  ])
  shares.mine = mine || []
  shares.received = received || []
  shares.loaded = true
}

const listShares = () =>
  run('shares', async (uuid) => {
    await loadShares(uuid)
    return shares.mine.length + (shares.mine.length === 1 ? ' account has' : ' accounts have') + ' this camera; ' + shares.received.length + ' shared with you'
  })

const savePermissions = (row: ShareRow) =>
  run('perm', async (uuid) => {
    await post('/api/cloud/device/shares/permissions', { shareId: row.shareId, permissions: row.permissions })
    await loadShares(uuid)
    return 'Permissions of ' + (row.account || 'that account') + ' saved'
  })

function toggleRowPermission(row: ShareRow, id: string) {
  row.permissions = row.permissions.includes(id) ? row.permissions.filter((item) => item !== id) : [...row.permissions, id]
}

const revoke = (row: ShareRow) =>
  run('revoke', async (uuid) => {
    await post('/api/cloud/device/shares/revoke', { shareId: row.shareId })
    await loadShares(uuid)
    return 'Stopped sharing with ' + (row.account || 'that account')
  })

const answer = (row: ShareRow, accept: boolean) =>
  run('answer', async (uuid) => {
    await post('/api/cloud/device/shares/answer', { shareId: row.shareId, accept })
    if (uuid) await loadShares(uuid)
    return accept ? 'Accepted ' + (row.deviceName || row.serial) : 'Refused ' + (row.deviceName || row.serial)
  }, false)

const makeQr = () =>
  run('qr', async (uuid) => {
    shares.qr = (await post('/api/cloud/device/share/qr', { uuid, permissions: [...granted] })) as unknown as QrReply
    return 'Share code ready; it works for ' + Math.round((shares.qr?.lifetimeSecs || 1800) / 60) + ' minutes'
  })

const label = (name: Action, idle: string) => (action.busy === name ? BUSY[name] : idle)

function forgetForm() {
  Object.assign(form, { name: '', group: '', favourite: false, shareTo: '', transferTo: '', key: '', sharedDevice: '', accountPassword: '', oldPassword: '', newPassword: '', unbindConfirm: '' })
  transfer.confirming = false
  Object.assign(action, { busy: '', message: '', kind: '' })
  Object.assign(list, { phase: 'idle', devices: [], groups: [], error: '' })
  Object.assign(shares, { mine: [], received: [], loaded: false, qr: null })
  chosen.value = ''
}

function syncScreen() {
  if (!open.value) return
  const screen = document.getElementById(origin)
  if (screen && !screen.hidden) return
  close(false)
}

onMounted(() => {
  const screens = new MutationObserver(() => {
    syncScreen()
    const signin = document.getElementById('signin')
    if (signin && !signin.hidden) forgetForm()
  })
  for (const id of ['watch', 'settings', 'signin']) {
    const screen = document.getElementById(id)
    if (screen) screens.observe(screen, { attributes: true, attributeFilter: ['hidden'] })
  }
  document.addEventListener('keydown', onEscape, true)
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
      <button v-if="list.phase === 'error'" class="btn" type="button" data-testid="device-admin-retry" @click="refresh"><Icon name="refresh" small />Try again</button>
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
          <Icon name="cctv" small />
          <span class="col">
            <span class="name">{{ entry.nickname }}</span>
            <span class="sub">{{ tags(entry) }}</span>
          </span>
          <Icon name="check" small />
        </button>
      </div>
      <div class="devices-tools">
        <button class="btn" type="button" data-testid="device-admin-add" @click="addCamera"><Icon name="plus" small />Add a camera</button>
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
              <button class="btn" type="button" :aria-busy="action.busy === 'save'" @click="save">{{ label('save', 'Save group and favourite') }}</button>
            </div>
          </section>
          <section v-if="device" class="devices-group danger" aria-labelledby="device-admin-danger-name">
            <h3 id="device-admin-danger-name"><Icon name="warning" small />Danger zone</h3>
            <div class="danger-item" data-testid="unbind">
              <h4>Remove from this account</h4>
              <label class="field">
                <span class="lab">Type {{ device.nickname || device.uuid }} or its serial</span>
                <input id="device-admin-unbind" v-model="form.unbindConfirm" type="text" autocomplete="off" data-testid="unbind-confirm" :placeholder="device.uuid">
                <span class="hint">The cloud account stops listing this camera; add it again by serial to get it back.</span>
              </label>
              <div class="sheet-actions">
                <button class="btn btn-fill danger" type="button" data-testid="unbind-submit" :disabled="!form.unbindConfirm.trim()" :aria-busy="action.busy === 'unbind'" @click="unbind">{{ label('unbind', 'Remove from this account') }}</button>
              </div>
            </div>
          </section>
        </TabsContent>
        <TabsContent value="share">
          <section v-if="device" class="devices-group" data-testid="share-permitted">
            <h3>Share with permissions</h3>
            <label class="field">
              <span class="lab">Share to account</span>
              <input id="device-admin-permit-to" v-model="form.shareTo" type="text" placeholder="the other account email or its account id">
            </label>
            <fieldset class="share-perms">
              <legend class="lab">What the other account may do</legend>
              <label v-for="item in permissions" :key="item.id" class="perm">
                <input type="checkbox" :data-perm="item.id" :checked="granted.has(item.id)" @change="toggleGrant(item.id)"><span>{{ item.label }}</span>
              </label>
            </fieldset>
            <div class="sheet-actions">
              <button class="btn btn-fill" type="button" data-testid="share-permit" :disabled="Boolean(permitReason)" :aria-busy="action.busy === 'permit'" @click="permit">{{ label('permit', 'Share with these permissions') }}</button>
              <button class="btn" type="button" data-testid="share-qr" :aria-busy="action.busy === 'qr'" @click="makeQr"><Icon name="qr" small />{{ label('qr', 'Share by QR code') }}</button>
            </div>
            <span v-if="permitReason" class="hint" data-testid="share-permit-reason">{{ permitReason }}</span>
            <figure v-if="shares.qr" class="share-qr" data-testid="share-qr-code">
              <img :src="qrImage" alt="QR code to add this camera with the chosen permissions" width="232" height="232">
              <figcaption class="hint">Scan it in the PROVIEW app, or paste the link into Add camera. It expires {{ new Date(shares.qr.expiresAt).toLocaleTimeString() }}.<br><code>{{ shares.qr.link }}</code></figcaption>
            </figure>
            <div class="sheet-actions">
              <button class="btn" type="button" data-testid="share-list" :aria-busy="action.busy === 'shares'" @click="listShares">{{ label('shares', 'Show who has it') }}</button>
            </div>
            <ul v-if="shares.loaded" class="share-rows" data-testid="share-rows">
              <li v-if="!shares.mine.length" class="hint">No account has this camera through a share.</li>
              <li v-for="row in shares.mine" :key="row.shareId" data-testid="share-row">
                <strong>{{ row.account || row.userId }}</strong>
                <span class="hint">{{ row.state ? stateOf(row.state) + ' · ' : '' }}{{ row.permissions.map(labelOf).join(', ') || 'no permissions' }}</span>
                <span class="share-row-perms">
                  <label v-for="item in permissions" :key="item.id" class="perm"><input type="checkbox" :checked="row.permissions.includes(item.id)" @change="toggleRowPermission(row, item.id)"><span>{{ item.label }}</span></label>
                </span>
                <span class="sheet-actions">
                  <button class="btn" type="button" :aria-busy="action.busy === 'perm'" @click="savePermissions(row)">{{ label('perm', 'Save permissions') }}</button>
                  <button class="btn danger" type="button" :aria-busy="action.busy === 'revoke'" @click="revoke(row)">{{ label('revoke', 'Stop sharing') }}</button>
                </span>
              </li>
            </ul>
          </section>
          <section v-if="shares.received.length" class="devices-group" data-testid="share-received">
            <h3>Shared with you</h3>
            <ul class="share-rows">
              <li v-for="row in shares.received" :key="row.shareId">
                <strong>{{ row.deviceName || row.serial }}</strong>
                <span class="hint">{{ row.account }}{{ row.state ? ' · ' + stateOf(row.state) : '' }}{{ row.permissions.length ? ' · ' + row.permissions.map(labelOf).join(', ') : '' }}</span>
                <span class="sheet-actions">
                  <button class="btn" type="button" :aria-busy="action.busy === 'answer'" @click="answer(row, true)">Accept</button>
                  <button class="btn" type="button" @click="answer(row, false)">Refuse</button>
                </span>
              </li>
            </ul>
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
              <button class="btn" type="button" :aria-busy="action.busy === 'accept'" @click="accept">{{ label('accept', 'Accept') }}</button>
            </div>
          </section>
          <section v-if="device" class="devices-group danger" aria-labelledby="device-admin-danger-share">
            <h3 id="device-admin-danger-share"><Icon name="warning" small />Danger zone</h3>
            <div class="danger-item">
              <h4>Transfer this camera</h4>
              <label class="field">
                <span class="lab">Transfer to account</span>
                <input id="device-admin-accept" v-model="form.transferTo" type="text" placeholder="the account that will own this camera" @input="transfer.confirming = false">
                <span class="hint">Ownership moves to that account once it accepts; this account loses the camera. An email resolves to its account id first.</span>
              </label>
              <div v-if="!transfer.confirming" class="sheet-actions">
                <button class="btn danger" type="button" data-testid="transfer-ask" :disabled="Boolean(transferReason)" @click="askTransfer">Transfer ownership…</button>
                <span v-if="transferReason" class="hint" data-testid="transfer-reason">{{ transferReason }}</span>
              </div>
              <div v-else class="sheet-actions confirm" role="alertdialog" aria-labelledby="transfer-ask-text" data-testid="transfer-confirm-panel">
                <span id="transfer-ask-text" class="hint">Give {{ device.nickname }} to {{ form.transferTo.trim() }}? This account stops owning it when they accept.</span>
                <button class="btn btn-fill danger" type="button" data-testid="transfer-confirm" :aria-busy="action.busy === 'share'" @click="share">{{ label('share', 'Yes, transfer ownership') }}</button>
                <button class="btn" type="button" data-testid="transfer-cancel" @click="transfer.confirming = false">Keep it</button>
              </div>
              <label class="field"><span class="lab">Invitation key</span><input id="device-admin-key" v-model="form.key" type="text" placeholder="transKey from the invitation"></label>
              <div class="sheet-actions">
                <button class="btn" type="button" :aria-busy="action.busy === 'check'" @click="check">{{ label('check', 'Check this camera') }}</button>
                <button class="btn" type="button" :aria-busy="action.busy === 'cancel'" @click="cancel">{{ label('cancel', 'Cancel invitation') }}</button>
              </div>
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

<style scoped>
#device-admin .devices-body > .btn { margin-bottom: var(--s3); }
#device-admin .devices-list { gap: var(--s1); margin: 0 0 var(--s2); }
#device-admin .devices-list .menu-row { gap: var(--s3); min-height: 52px; padding: var(--s2) var(--s3); border: 1px solid var(--line-soft); border-radius: var(--r-md); background: transparent; }
#device-admin .devices-list .menu-row:hover { background: var(--bg-3); }
#device-admin .devices-list .menu-row[aria-checked="true"] { border-color: var(--accent); background: var(--accent-soft); }
#device-admin .devices-list .menu-row > .i-16:first-child { visibility: visible; margin: 0; color: var(--ink-3); }
#device-admin .devices-list .menu-row[aria-checked="true"] > .i-16:first-child { color: var(--accent); }
#device-admin .devices-list .menu-row .col { flex: 1; min-width: 0; }
#device-admin .devices-list .menu-row .name { font-size: var(--t-14); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
#device-admin .devices-list .menu-row .sub { overflow-wrap: anywhere; }
#device-admin .devices-tools { display: flex; justify-content: flex-end; margin: 0 0 var(--s3); }
#device-admin .tabs-list { margin-bottom: var(--s3); }
#device-admin .devices-group { margin: 0 0 var(--s3); padding: var(--s3); border: 1px solid var(--line-soft); border-radius: var(--r-md); background: var(--bg); }
#device-admin .devices-group:last-child { margin-bottom: 0; }
#device-admin .devices-group h3 { display: flex; align-items: center; gap: var(--s2); margin: 0 0 var(--s3); font-size: var(--t-13); font-weight: 500; color: var(--ink-2); }
#device-admin .devices-group .sheet-actions { margin-bottom: var(--s3); }
#device-admin .devices-group .sheet-actions:last-child { margin-bottom: 0; }
#device-admin .devices-group.danger { border-color: var(--bad); }
#device-admin .devices-group.danger h3 { color: var(--bad); }
#device-admin .danger-item h4 { margin: 0 0 var(--s2); font-size: var(--t-14); font-weight: 500; color: var(--ink); }
#device-admin .sheet-actions.confirm { padding: var(--s2); border: 1px solid var(--bad); border-radius: var(--r-sm); }
#device-admin .sheet-actions.confirm .hint { flex: 1 1 100%; color: var(--ink); }
#device-admin .share-perms { margin-bottom: var(--s3); }
#device-admin .share-perms .lab { margin-bottom: var(--s1); font-size: var(--t-13); color: var(--ink-2); }
#device-admin .perm { display: flex; align-items: center; gap: var(--s2); min-height: 32px; font-size: var(--t-14); color: var(--ink); cursor: pointer; }
#device-admin .share-row-perms { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 0 var(--s3); }
#device-admin .share-rows { margin-top: var(--s3); }
#device-admin .share-rows li { padding: var(--s2) 0; }
@media (pointer: coarse) {
  #device-admin .perm { min-height: var(--hit); }
}
</style>
