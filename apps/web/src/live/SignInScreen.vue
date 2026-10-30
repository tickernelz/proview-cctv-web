<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { Field } from '../components/ui/field'
import { IconButton } from '../components/ui/icon-button'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs'
import { api } from '../lib/api'
import { openSettings, signedIn } from '../app/shell'
import { clockHm, clockHms } from '../states'
import { live } from './store'
import AccountFlows from './AccountFlows.vue'
import { sentence } from './account-flows'

type Probe = { host: string; session: number | string; device_type?: string; video_codec?: string; audio_codec?: string; framerate?: number }
type Remembered = { email?: string; restored?: boolean }

const tab = ref<'cloud' | 'local'>('cloud')
const emailInput = ref<HTMLInputElement>()
const passwordInput = ref<HTMLInputElement>()
const localPassInput = ref<HTMLInputElement>()
const cloud = reactive({ email: '', password: '', busy: false, error: '', reveal: false })
const local = reactive({ user: '', password: '', busy: false, error: '' })
const probe = reactive({ busy: false, report: null as Probe | null, error: '' })
const remember = reactive({ on: false, note: '' })
const flow = ref<'' | 'forgot' | 'register'>('')
const flowNote = ref('')
const server = location.host

const s = live.signin
const expired = computed(() => s.expired)
const mode = computed(() => (s.mode ? s.mode.mode : ''))
const localReady = computed(() => mode.value === 'local')
const lede = computed(() =>
  expired.value ? 'Sign in again to pick the camera back up where you left it.' : 'Use your PROVIEW account, or reach a camera on this network.',
)
const ended = computed(() => (s.expiredAt ? 'Your session ended at ' + clockHm(s.expiredAt) + '.' : 'Your session ended.'))
const endedDetail = computed(() =>
  (s.lastFrameAt ? 'The server no longer knew this browser. Last frame ' + clockHms(s.lastFrameAt) + '. ' : 'The server no longer knew this browser. ') +
  'Nothing is being recorded while you are signed out.',
)
const privacy = computed(() =>
  tab.value === 'local' ? 'The camera password goes only to this server and the camera.' : 'Your password goes only to this server and the PROVIEW cloud.',
)
const localNote = computed(() => {
  if (!s.mode) return 'The server did not report which provider it uses.'
  if (localReady.value) return 'The server is set to this network, so the camera login below is the one to use.'
  return 'The server is set to the ' + (mode.value === 'cloud' ? 'cloud account' : mode.value || 'other provider') + '. Use Account, or choose this network under Settings, Connection.'
})

async function rememberNow() {
  if (!remember.on) return
  try {
    await api('/api/account/remember', { method: 'POST' })
    remember.note = 'This account will come back signed in next time.'
  } catch (error) {
    remember.note = 'Could not remember the account: ' + sentence(error)
  }
}

function setRemember(on: boolean) {
  remember.on = on
  remember.note = ''
}

async function submitCloud() {
  if (cloud.busy) return
  cloud.error = ''
  cloud.busy = true
  try {
    await api('/api/login', { method: 'POST', body: JSON.stringify({ email: cloud.email, password: cloud.password }) })
    cloud.password = ''
    cloud.reveal = false
    cloud.busy = false
    await rememberNow()
    await signedIn()
  } catch (error) {
    cloud.busy = false
    const status = (error as { status?: number }).status
    cloud.error = status === 401 ? 'That email and password were not accepted.' : 'Sign in failed: ' + sentence(error)
    await nextTick()
    passwordInput.value?.focus()
    passwordInput.value?.select()
  }
}

async function submitLocal() {
  if (local.busy || !localReady.value) return
  local.error = ''
  local.busy = true
  try {
    await api('/api/login', { method: 'POST', body: JSON.stringify({ email: local.user, password: local.password }) })
    local.password = ''
    local.busy = false
    await rememberNow()
    await signedIn()
  } catch (error) {
    local.busy = false
    const status = (error as { status?: number }).status
    local.error = status === 401 ? 'The camera did not accept that username and password.' : 'The camera login failed: ' + sentence(error)
    await nextTick()
    localPassInput.value?.focus()
    localPassInput.value?.select()
  }
}

async function runProbe() {
  if (probe.busy) return
  probe.busy = true
  probe.error = ''
  probe.report = null
  try {
    probe.report = await api<Probe>('/api/device/probe', { method: 'POST' })
  } catch (error) {
    const code = (error as { code?: string }).code
    probe.error =
      code === 'device_host_missing'
        ? 'This server has no camera address yet. Add it under Settings, Configuration, restart the app, then probe again.'
        : 'The camera on this network did not answer: ' + sentence(error)
  } finally {
    probe.busy = false
  }
}

function openFlow(kind: 'forgot' | 'register') {
  flowNote.value = ''
  cloud.error = ''
  flow.value = kind
}

async function closeFlow(account: string, note: string) {
  if (account) cloud.email = account
  flowNote.value = note
  flow.value = ''
  await focusFirst()
}

async function focusFirst() {
  await nextTick()
  if (tab.value !== 'cloud') return
  const target = expired.value && cloud.email ? passwordInput.value : emailInput.value
  target?.focus()
}

watch(() => s.focus, focusFirst)
watch(() => s.generation, () => {
  cloud.error = ''
  local.error = ''
  cloud.busy = false
  local.busy = false
})

onMounted(async () => {
  try {
    const state = await api<Remembered>('/api/account/remembered')
    if (state && state.email) {
      if (!cloud.email) cloud.email = state.email
      if (!state.restored) remember.note = 'The remembered session for ' + state.email + ' expired. Sign in again to continue.'
    }
  } catch {
    return
  }
})
</script>

<template>
  <div class="auth-col">
    <p class="auth-brand"><span class="brand"><span class="mark" aria-hidden="true" /><span>PROVIEW</span></span></p>
    <div class="auth-head">
      <h1 id="signin-title" tabindex="-1">Sign in</h1>
      <p id="signin-lede" class="auth-lede">{{ lede }}</p>
    </div>
    <section v-if="expired" id="signin-notice" class="auth-expired" data-testid="signin-notice" aria-labelledby="signin-ended">
      <Icon name="clock" small />
      <div>
        <p id="signin-ended" class="auth-expired-title" data-testid="signin-ended">{{ ended }}</p>
        <p class="auth-expired-text">{{ endedDetail }}</p>
      </div>
    </section>
    <AccountFlows v-if="flow" :kind="flow" :email="cloud.email" @close="closeFlow" />
    <Tabs v-else v-model="tab" class="auth-tabs">
      <TabsList aria-label="Sign-in method">
        <TabsTrigger value="cloud" data-testid="signin-tab-account">Account</TabsTrigger>
        <TabsTrigger value="local" data-testid="signin-tab-local">This network</TabsTrigger>
      </TabsList>
      <TabsContent value="cloud">
        <form id="login-form" class="auth-form" autocomplete="on" novalidate @submit.prevent="submitCloud">
          <Field id="email" label="Email">
            <template #default="{ id }">
              <input :id="id" ref="emailInput" v-model="cloud.email" class="auth-input" data-testid="signin-email" name="email" type="email" autocomplete="username" placeholder="you@example.com" spellcheck="false" required>
            </template>
          </Field>
          <div id="password-field" class="flex flex-col gap-1.5">
            <label for="password" class="text-13 font-medium text-ink-2">Password</label>
            <div class="auth-secret">
              <input
                id="password"
                ref="passwordInput"
                v-model="cloud.password"
                class="auth-input"
                data-testid="signin-password"
                name="password"
                :type="cloud.reveal ? 'text' : 'password'"
                autocomplete="current-password"
                required
                :aria-invalid="cloud.error ? 'true' : null"
                :aria-describedby="cloud.error ? 'login-error' : null"
              >
              <IconButton
                size="sm"
                class="auth-reveal"
                data-testid="signin-reveal"
                :icon="cloud.reveal ? 'eye-off' : 'eye'"
                :label="cloud.reveal ? 'Hide the password' : 'Show the password'"
                :pressed="cloud.reveal"
                @click="cloud.reveal = !cloud.reveal"
              />
            </div>
            <p v-if="cloud.error" id="login-error" class="err" data-testid="signin-error" role="alert"><Icon name="warning" small /><span>{{ cloud.error }}</span></p>
          </div>
          <label class="auth-switch">
            <input id="remember-account" :checked="remember.on" type="checkbox" role="switch" class="peer" data-testid="signin-remember" @change="setRemember(($event.target as HTMLInputElement).checked)">
            <span class="auth-track" aria-hidden="true"><span /></span>
            <span>Remember this account in this browser</span>
          </label>
          <p v-if="remember.note" id="remember-note" class="hint" data-testid="signin-remember-note" role="status">{{ remember.note }}</p>
          <Button id="login-submit" data-testid="signin-submit" variant="primary" size="lg" block type="submit" :aria-busy="cloud.busy ? 'true' : 'false'">
            {{ cloud.busy ? 'Signing in…' : expired ? 'Sign in to resume' : 'Sign in' }}
          </Button>
          <p v-if="flowNote" class="hint" data-testid="signin-flow-note" role="status">{{ flowNote }}</p>
          <div class="auth-links">
            <Button variant="ghost" size="sm" data-testid="signin-forgot" @click="openFlow('forgot')">Forgot password</Button>
            <Button variant="ghost" size="sm" data-testid="signin-register" @click="openFlow('register')">Create account</Button>
          </div>
        </form>
      </TabsContent>
      <TabsContent value="local">
        <div class="auth-form">
          <p id="local-mode" class="auth-sub" role="status">{{ localNote }}</p>
          <form v-if="localReady" id="local-form" class="auth-form" novalidate @submit.prevent="submitLocal">
            <Field id="local-user" label="Camera username">
              <template #default="{ id }">
                <input :id="id" v-model="local.user" class="auth-input" type="text" autocomplete="username" spellcheck="false">
              </template>
            </Field>
            <div id="local-pass-field" class="flex flex-col gap-1.5">
              <label for="local-pass" class="text-13 font-medium text-ink-2">Camera password</label>
              <input
                id="local-pass"
                ref="localPassInput"
                v-model="local.password"
                class="auth-input"
                type="password"
                autocomplete="current-password"
                :aria-invalid="local.error ? 'true' : null"
                :aria-describedby="local.error ? 'local-error' : null"
              >
              <p v-if="local.error" id="local-error" class="err" data-testid="signin-local-error" role="alert"><Icon name="warning" small /><span>{{ local.error }}</span></p>
            </div>
            <Button id="local-submit" variant="primary" size="lg" block type="submit" :aria-busy="local.busy ? 'true' : 'false'">{{ local.busy ? 'Signing in…' : 'Sign in to the camera' }}</Button>
          </form>
          <Button id="local-connect" block icon="refresh" data-testid="signin-probe" :aria-busy="probe.busy ? 'true' : 'false'" @click="runProbe">
            {{ probe.busy ? 'Probing…' : 'Probe the camera on this network' }}
          </Button>
          <p v-if="probe.error" class="err" data-testid="signin-probe-error" role="alert"><Icon name="warning" small /><span>{{ probe.error }}</span></p>
          <template v-if="probe.report">
            <dl id="local-report" class="report" data-testid="signin-probe-report">
              <div><dt>Host</dt><dd>{{ probe.report.host }}</dd></div>
              <div><dt>Session</dt><dd>{{ String(probe.report.session) }}</dd></div>
              <div><dt>Device type</dt><dd>{{ probe.report.device_type || 'not reported' }}</dd></div>
              <div><dt>Video</dt><dd>{{ probe.report.video_codec || 'not reported' }}</dd></div>
              <div><dt>Audio</dt><dd>{{ probe.report.audio_codec || 'none' }}</dd></div>
              <div><dt>Frame rate</dt><dd>{{ probe.report.framerate ? probe.report.framerate + ' fps' : 'not reported' }}</dd></div>
            </dl>
            <p id="local-next" class="auth-sub">This server is set to the camera on this network, so the account sign-in shows that camera too.</p>
          </template>
        </div>
      </TabsContent>
    </Tabs>
    <div class="auth-row">
      <Button id="login-settings" variant="ghost" size="sm" icon="gear" class="auth-settings" data-testid="signin-settings" @click="openSettings('config', { focus: true })">Open settings</Button>
      <span class="auth-server" data-testid="signin-server">server {{ server }}</span>
    </div>
    <p class="auth-why">Settings opens before you sign in: cloud address, camera address, camera password.</p>
    <p class="auth-privacy"><Icon name="lock" small /><span>{{ privacy }}</span></p>
  </div>
  <p class="auth-keys">Keyboard: Tab moves between fields, Enter signs in</p>
</template>
