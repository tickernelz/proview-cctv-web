<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { api } from '../lib/api'
import { sessionLost, signOut } from '../app/shell'
import { live } from './store'
import { settingsUi } from '../settings/ui'
import Icon from '../components/Icon.vue'
import { sentence, stamp } from './account-flows'

type Profile = {
  id?: string | null
  username?: string | null
  mail?: string | null
  phone?: string | null
  nickname?: string | null
  registerTime?: string | null
  passUpdateTime?: string | null
  source?: string
}
type Done = { ok: boolean; message: string; latencyMs?: number }

const props = defineProps<{ shown: boolean }>()
const s = live.settings

const profile = reactive({ data: null as Profile | null, error: '', status: 0, loading: false })
const me = reactive({ account: '' })
const password = reactive({ current: '', next: '', confirm: '', busy: false, note: '', bad: false })
const removal = reactive({ sent: false, sending: false, code: '', account: '', understood: false, busy: false, note: '', bad: false })

const notCloud = computed(() => {
  if (!s.signedIn) return 'Sign in with the cloud account first.'
  if (profile.status === 409) return 'This session has no cloud sign-in. Sign in with the cloud account to manage it.'
  return ''
})
const passwordBlock = computed(() => {
  if (notCloud.value) return notCloud.value
  if (password.busy) return ''
  if (!password.current) return 'Enter the current password.'
  if (!password.next) return 'Enter a new password.'
  if (password.next === password.current) return 'The new password is the same as the current one.'
  if (password.next !== password.confirm) return 'The two new passwords differ.'
  return ''
})
const codeBlock = computed(() => notCloud.value)
const deleteBlock = computed(() => {
  if (notCloud.value) return notCloud.value
  if (removal.busy) return ''
  if (!removal.sent) return 'Send the deletion code first.'
  if (!removal.code.trim()) return 'Enter the code from the email.'
  if (!removal.account.trim()) return 'Type the account email you sign in with.'
  if (me.account && removal.account.trim().toLowerCase() !== me.account.trim().toLowerCase()) return 'That is not the signed-in account.'
  if (!removal.understood) return 'Tick the box to confirm.'
  return ''
})

async function load() {
  if (!s.signedIn || profile.loading) return
  profile.loading = true
  profile.error = ''
  profile.status = 0
  try {
    const who = await api<{ account?: string }>('/api/me')
    me.account = who.account || ''
    profile.data = await api<Profile>('/api/cloud/profile')
  } catch (error) {
    if (sessionLost(error)) return
    profile.data = null
    profile.status = (error as { status?: number }).status || 0
    profile.error = sentence(error)
  } finally {
    profile.loading = false
  }
}

async function changePassword() {
  if (passwordBlock.value || password.busy) return
  password.busy = true
  password.note = 'Checking the current password with the cloud…'
  password.bad = false
  try {
    const done = await api<Done>('/api/account/password', {
      method: 'POST',
      body: JSON.stringify({ current: password.current, next: password.next }),
    })
    password.current = ''
    password.next = ''
    password.confirm = ''
    password.note = sentence(done.message)
    await load()
  } catch (error) {
    if (sessionLost(error)) return
    password.bad = true
    password.note = sentence(error)
  } finally {
    password.busy = false
  }
}

async function sendDeletionCode() {
  if (codeBlock.value || removal.sending) return
  removal.sending = true
  removal.bad = false
  removal.note = 'Asking the cloud to email a deletion code…'
  try {
    const done = await api<Done>('/api/account/delete/code', { method: 'POST' })
    removal.sent = true
    removal.note = sentence(done.message)
  } catch (error) {
    if (sessionLost(error)) return
    removal.bad = true
    removal.note = sentence(error)
  } finally {
    removal.sending = false
  }
}

async function deleteAccount() {
  if (deleteBlock.value || removal.busy) return
  removal.busy = true
  removal.bad = false
  removal.note = 'Deleting the cloud account…'
  try {
    const done = await api<Done>('/api/account/delete', {
      method: 'POST',
      body: JSON.stringify({ account: removal.account.trim(), code: removal.code.trim(), confirm: removal.understood }),
    })
    removal.note = sentence(done.message)
    await signOut()
  } catch (error) {
    if (sessionLost(error)) return
    removal.bad = true
    removal.note = sentence(error)
  } finally {
    removal.busy = false
  }
}

watch(
  () => props.shown,
  (shown) => {
    if (shown) load()
  },
)

watch(
  () => s.signedIn,
  (signed) => {
    profile.data = null
    profile.error = ''
    profile.status = 0
    me.account = ''
    Object.assign(password, { current: '', next: '', confirm: '', note: '', bad: false })
    Object.assign(removal, { sent: false, code: '', account: '', understood: false, note: '', bad: false })
    if (signed && props.shown) load()
  },
)

watch(
  () => settingsUi.reload.account,
  () => {
    if (props.shown) load()
  },
)
</script>

<template>
  <header class="set-head">
    <div>
      <h2 id="h-account" class="set-h" tabindex="-1">Account</h2>
      <p class="set-sub">The PROVIEW cloud account this app is signed in with, read from the vendor cloud now. Changes here apply to the phone app too.</p>
    </div>
  </header>
  <p v-if="notCloud" class="set-status acct-state" data-testid="account-not-cloud" role="status"><Icon name="info" small /><span>{{ notCloud }}</span></p>
  <p v-else-if="profile.loading" class="set-status acct-state" role="status"><span>Reading the profile from the cloud…</span></p>
  <p v-else-if="profile.error" class="set-status acct-state warn-text" data-testid="account-error" role="status"><Icon name="warning" small /><span>{{ profile.error }}</span></p>
  <div class="set-cols">
    <div class="set-col">
      <div v-if="profile.data" id="account-profile" class="set-card" data-testid="account-profile">
        <div class="set-card-h">
          <div><h3 class="set-card-t"><Icon name="user" small />Signed in account</h3></div>
        </div>
        <div class="row"><div class="lab">Username</div><div class="ctl-col"><p class="val">{{ profile.data.username || 'not set' }}</p></div></div>
        <div class="row">
          <div class="lab">Email<span class="hint">Masked by the cloud.</span></div>
          <div class="ctl-col"><p class="val">{{ profile.data.mail || 'not set' }}</p></div>
        </div>
        <div class="row"><div class="lab">Phone</div><div class="ctl-col"><p class="val">{{ profile.data.phone || 'not set' }}</p></div></div>
        <div class="row"><div class="lab">Nickname</div><div class="ctl-col"><p class="val">{{ profile.data.nickname || 'not set' }}</p></div></div>
        <div class="row"><div class="lab">Account id</div><div class="ctl-col"><p class="codes"><span class="code">{{ profile.data.id || 'not reported' }}</span></p></div></div>
        <div class="row"><div class="lab">Registered</div><div class="ctl-col"><p class="val">{{ stamp(profile.data.registerTime) }}</p></div></div>
        <div class="row"><div class="lab">Password changed</div><div class="ctl-col"><p class="val">{{ stamp(profile.data.passUpdateTime) }}</p></div></div>
      </div>
    </div>
    <div class="set-col">
      <form id="account-password" class="set-card" novalidate data-testid="account-password" @submit.prevent="changePassword">
        <div class="set-card-h">
          <div>
            <h3 class="set-card-t"><Icon name="key" small />Change password</h3>
            <p class="set-card-d">The current password is checked with a fresh cloud sign-in first. This session switches to the new password.</p>
          </div>
        </div>
        <div class="row">
          <label class="lab" for="account-current">Current password</label>
          <div class="ctl-col"><input id="account-current" v-model="password.current" class="text-input" type="password" autocomplete="current-password" data-testid="account-current"></div>
        </div>
        <div class="row">
          <label class="lab" for="account-next">New password</label>
          <div class="ctl-col"><input id="account-next" v-model="password.next" class="text-input" type="password" autocomplete="new-password" data-testid="account-next"></div>
        </div>
        <div class="row">
          <label class="lab" for="account-confirm">New password again</label>
          <div class="ctl-col"><input id="account-confirm" v-model="password.confirm" class="text-input" type="password" autocomplete="new-password" data-testid="account-confirm"></div>
        </div>
        <div class="set-card-f">
          <div class="acct-foot">
            <p v-if="passwordBlock" class="hint" data-testid="account-password-reason">{{ passwordBlock }}</p>
            <p v-if="password.note" :class="['set-status', { 'warn-text': password.bad }]" data-testid="account-password-note" role="status">
              <Icon :name="password.bad ? 'warning' : 'check'" small /><span>{{ password.note }}</span>
            </p>
          </div>
          <button class="btn btn-fill" type="submit" data-testid="account-password-save" :disabled="Boolean(passwordBlock)" :aria-busy="password.busy">
            {{ password.busy ? 'Changing…' : 'Change password' }}
          </button>
        </div>
      </form>
    </div>
  </div>
  <div class="set-stack">
    <div class="set-card danger">
      <div class="set-card-h">
        <div><h3 class="set-card-t"><Icon name="warning" small />Danger zone</h3></div>
      </div>
      <div class="row stack" data-testid="account-delete">
        <div class="lab">
          Delete the cloud account
          <span class="hint">Irreversible. The account, its cameras, shares and cloud history are removed from the vendor cloud, and the phone app loses them too.</span>
        </div>
        <div class="ctl-col">
          <div class="acct-step">
            <button class="btn" type="button" data-testid="account-delete-code" :disabled="Boolean(codeBlock)" :aria-busy="removal.sending" @click="sendDeletionCode">
              {{ removal.sending ? 'Sending…' : removal.sent ? 'Send the code again' : 'Email me a deletion code' }}
            </button>
          </div>
          <div class="acct-fields">
            <label class="field">
              <span>Code from the email</span>
              <input v-model="removal.code" class="text-input" type="text" inputmode="numeric" autocomplete="one-time-code" data-testid="account-delete-codeinput" :disabled="!removal.sent">
            </label>
            <label class="field">
              <span>Type the account email you sign in with</span>
              <input v-model="removal.account" class="text-input" type="email" autocomplete="off" spellcheck="false" data-testid="account-delete-account" :disabled="!removal.sent">
            </label>
          </div>
          <label class="acct-check">
            <input v-model="removal.understood" type="checkbox" data-testid="account-delete-understood" :disabled="!removal.sent">
            <span>I understand this deletes the account for good</span>
          </label>
          <div class="acct-submit">
            <div class="acct-foot">
              <p v-if="deleteBlock" class="hint" data-testid="account-delete-reason">{{ deleteBlock }}</p>
              <p v-if="removal.note" :class="['set-status', { 'warn-text': removal.bad }]" data-testid="account-delete-note" role="status">
                <Icon v-if="removal.bad" name="warning" small /><span>{{ removal.note }}</span>
              </p>
            </div>
            <button class="btn btn-fill danger" type="button" data-testid="account-delete-submit" :disabled="Boolean(deleteBlock)" :aria-busy="removal.busy" @click="deleteAccount">
              {{ removal.busy ? 'Deleting…' : 'Delete the account' }}
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.acct-state { margin: 0 0 var(--s4); }
.acct-foot { display: grid; gap: var(--s1); margin-right: auto; min-width: 0; flex: 1 1 200px; }
.acct-foot .hint, .acct-foot .set-status { margin: 0; text-align: left; }
.acct-step { display: flex; }
.acct-fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--s3); }
.acct-fields .text-input { width: 100%; }
.acct-check { display: flex; align-items: center; gap: var(--s2); min-height: var(--hit); font-size: var(--t-14); color: var(--ink); cursor: pointer; }
.acct-check:has(input:disabled) { color: var(--ink-3); cursor: not-allowed; }
.acct-submit { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2) var(--s3); padding-top: var(--s3); border-top: 1px solid var(--line-soft); }
</style>
