<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from 'vue'
import { TabsContent, TabsList, TabsRoot, TabsTrigger } from 'reka-ui'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { hm, hms } from '../app/format'
import { openSettings, signedIn } from '../app/shell'
import { live } from './store'

type Probe = { host: string; session: number | string; device_type?: string; video_codec?: string; audio_codec?: string; framerate?: number }
type Remembered = { email?: string; restored?: boolean }

const tab = ref<'cloud' | 'local'>('cloud')
const emailInput = ref<HTMLInputElement>()
const passwordInput = ref<HTMLInputElement>()
const localPassInput = ref<HTMLInputElement>()
const cloud = reactive({ email: '', password: '', busy: false, error: '' })
const local = reactive({ user: '', password: '', busy: false, error: '' })
const probe = reactive({ busy: false, report: null as Probe | null, error: '' })
const remember = reactive({ on: false, note: '' })

const s = live.signin
const expired = computed(() => s.expired)
const mode = computed(() => (s.mode ? s.mode.mode : ''))
const localReady = computed(() => mode.value === 'local')
const lede = computed(() =>
  expired.value
    ? 'The server ended this session at ' + hm(s.expiredAt) + '. Signing in again picks the camera back up.'
    : 'The same PROVIEW account your phone uses. The camera password stays on this machine.',
)
const stage = computed(() =>
  expired.value && s.lastFrameAt
    ? 'Last frame ' + hms(s.lastFrameAt) + '. Signing in picks the stream up again.'
    : 'The picture appears here as soon as a camera answers.',
)
const localNote = computed(() => {
  if (!s.mode) return 'The server did not report which provider it uses.'
  if (localReady.value) return 'The server is set to this network, so the camera login below is the one to use.'
  return 'The server is set to the ' + (mode.value === 'cloud' ? 'cloud account' : mode.value || 'other provider') + '. Use Account, or choose this network under Settings, Connection.'
})

function sentence(error: unknown) {
  const text = error instanceof Error ? error.message : String(error)
  return text.charAt(0).toUpperCase() + text.slice(1)
}

async function rememberNow() {
  if (!remember.on) return
  try {
    await api('/api/account/remember', { method: 'POST' })
    remember.note = 'This account will come back signed in next time.'
  } catch (error) {
    remember.note = 'Could not remember the account: ' + sentence(error)
  }
}

async function submitCloud() {
  if (cloud.busy) return
  cloud.error = ''
  cloud.busy = true
  try {
    await api('/api/login', { method: 'POST', body: JSON.stringify({ email: cloud.email, password: cloud.password }) })
    cloud.password = ''
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
  <div class="auth-form">
    <h1 id="signin-title" tabindex="-1">Watch the camera from this desk</h1>
    <p id="signin-lede" class="lede">{{ lede }}</p>
    <div v-if="expired" id="signin-notice" class="notice" data-testid="signin-notice">
      <Icon name="clock" small /><span>Nothing is being recorded while you are signed out.</span>
    </div>
    <TabsRoot v-model="tab">
      <TabsList class="tabs" aria-label="Sign-in method">
        <TabsTrigger value="cloud" data-testid="signin-tab-account">Account</TabsTrigger>
        <TabsTrigger value="local" data-testid="signin-tab-local">This network</TabsTrigger>
      </TabsList>
      <TabsContent value="cloud">
        <form id="login-form" autocomplete="on" novalidate @submit.prevent="submitCloud">
          <div class="field">
            <label for="email">Email</label>
            <input id="email" ref="emailInput" v-model="cloud.email" data-testid="signin-email" name="email" type="email" autocomplete="username" placeholder="name@example.com" spellcheck="false" required>
          </div>
          <div id="password-field" class="field" :class="{ bad: cloud.error }">
            <label for="password">Password</label>
            <input
              id="password"
              ref="passwordInput"
              v-model="cloud.password"
              data-testid="signin-password"
              name="password"
              type="password"
              autocomplete="current-password"
              required
              :aria-invalid="cloud.error ? 'true' : null"
              :aria-describedby="cloud.error ? 'login-error' : null"
            >
            <p v-if="cloud.error" id="login-error" class="err" data-testid="signin-error" role="alert"><Icon name="warning" small /><span>{{ cloud.error }}</span></p>
          </div>
          <label class="switchrow signin-remember">
            <input id="remember-account" v-model="remember.on" type="checkbox" role="switch" data-testid="signin-remember" @change="remember.note = ''">
            Remember this account in this browser
          </label>
          <p v-if="remember.note" id="remember-note" class="hint" data-testid="signin-remember-note" role="status">{{ remember.note }}</p>
          <button id="login-submit" data-testid="signin-submit" class="btn btn-fill" type="submit" :aria-busy="cloud.busy">
            {{ cloud.busy ? 'Signing in…' : expired ? 'Sign in to resume' : 'Sign in' }}
          </button>
        </form>
      </TabsContent>
      <TabsContent value="local">
        <p id="local-mode" class="auth-sub" role="status">{{ localNote }}</p>
        <form v-if="localReady" id="local-form" novalidate @submit.prevent="submitLocal">
          <div class="field">
            <label for="local-user">Camera username</label>
            <input id="local-user" v-model="local.user" type="text" autocomplete="username" spellcheck="false">
          </div>
          <div id="local-pass-field" class="field" :class="{ bad: local.error }">
            <label for="local-pass">Camera password</label>
            <input
              id="local-pass"
              ref="localPassInput"
              v-model="local.password"
              type="password"
              autocomplete="current-password"
              :aria-invalid="local.error ? 'true' : null"
              :aria-describedby="local.error ? 'local-error' : null"
            >
            <p v-if="local.error" id="local-error" class="err" data-testid="signin-local-error" role="alert"><Icon name="warning" small /><span>{{ local.error }}</span></p>
          </div>
          <button id="local-submit" class="btn btn-fill" type="submit" :aria-busy="local.busy">{{ local.busy ? 'Signing in…' : 'Sign in to the camera' }}</button>
        </form>
        <button id="local-connect" class="btn btn-line wide" type="button" data-testid="signin-probe" :aria-busy="probe.busy" @click="runProbe">
          <Icon name="refresh" small />{{ probe.busy ? 'Probing…' : 'Probe the camera on this network' }}
        </button>
        <p v-if="probe.error" class="err signin-probe-err" data-testid="signin-probe-error" role="alert"><Icon name="warning" small /><span>{{ probe.error }}</span></p>
        <template v-if="probe.report">
          <dl id="local-report" class="report" data-testid="signin-probe-report">
            <div><dt>Host</dt><dd>{{ probe.report.host }}</dd></div>
            <div><dt>Session</dt><dd>{{ String(probe.report.session) }}</dd></div>
            <div><dt>Device type</dt><dd>{{ probe.report.device_type || 'not reported' }}</dd></div>
            <div><dt>Video</dt><dd>{{ probe.report.video_codec || 'not reported' }}</dd></div>
            <div><dt>Audio</dt><dd>{{ probe.report.audio_codec || 'none' }}</dd></div>
            <div><dt>Frame rate</dt><dd>{{ probe.report.framerate ? probe.report.framerate + ' fps' : 'not reported' }}</dd></div>
          </dl>
          <p id="local-next" class="auth-note">This server is set to the camera on this network, so the account sign-in shows that camera too.</p>
        </template>
      </TabsContent>
    </TabsRoot>
    <p class="auth-note">
      Settings opens before you sign in: cloud address, camera address, camera password.
      <button id="login-settings" class="btn btn-line signin-settings" type="button" data-testid="signin-settings" @click="openSettings('config', { focus: true })"><Icon name="sliders" small />Open Settings</button>
    </p>
  </div>
  <div class="auth-stage" aria-hidden="true">
    <div class="stage" />
    <p id="signin-stage" class="stage-cap">{{ stage }}</p>
  </div>
</template>
