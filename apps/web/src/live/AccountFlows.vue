<script setup lang="ts">
import { computed, nextTick, onMounted, reactive } from 'vue'
import Icon from '../components/Icon.vue'
import { api } from '../lib/api'
import { looksLikeMail, sentence } from './account-flows'

type Done = { ok: boolean; message: string }
type NameCheck = { name: string; available: boolean; message: string }

const props = defineProps<{ kind: 'forgot' | 'register'; email: string }>()
const emit = defineEmits<{ close: [email: string, note: string] }>()

const form = reactive({ email: props.email, code: '', password: '', confirm: '', username: '' })
const step = reactive({ sent: false, checked: false, busy: '', note: '', bad: false })
const name = reactive({ checked: '', available: false, note: '' })

const forgot = computed(() => props.kind === 'forgot')
const title = computed(() => (forgot.value ? 'Reset the password' : 'Create a cloud account'))
const lede = computed(() =>
  forgot.value
    ? 'The cloud emails a six-digit code to the account address. Check it, then choose a new password.'
    : 'A PROVIEW cloud account, the same kind the phone app creates. The cloud emails a six-digit code to confirm the address.',
)
const sendBlock = computed(() => {
  if (step.busy) return ''
  if (!forgot.value && (!name.checked || name.checked !== form.username.trim())) return 'Check the username first.'
  if (!forgot.value && !name.available) return 'Pick a username that is free.'
  if (!looksLikeMail(form.email)) return 'Enter the account email address.'
  return ''
})
const checkBlock = computed(() => {
  if (step.busy) return ''
  if (!step.sent) return 'Send the code first.'
  if (!form.code.trim()) return 'Enter the code from the email.'
  return ''
})
const finishBlock = computed(() => {
  if (step.busy) return ''
  if (forgot.value && !step.checked) return 'Check the code first.'
  if (!forgot.value && !step.sent) return 'Send the code first.'
  if (!form.code.trim()) return 'Enter the code from the email.'
  if (!form.password) return 'Enter a password.'
  if (form.password !== form.confirm) return 'The two passwords differ.'
  return ''
})
const nameBlock = computed(() => (step.busy ? '' : form.username.trim() ? '' : 'Enter a username.'))

async function run(job: string, note: string, call: () => Promise<string>) {
  step.busy = job
  step.bad = false
  step.note = note
  try {
    step.note = sentence(await call())
    return true
  } catch (error) {
    step.bad = true
    step.note = sentence(error)
    return false
  } finally {
    step.busy = ''
  }
}

async function checkName() {
  if (nameBlock.value) return
  const wanted = form.username.trim()
  await run('name', 'Asking the cloud whether the name is free…', async () => {
    const reply = await api<NameCheck>('/api/account/register/name?name=' + encodeURIComponent(wanted))
    name.checked = wanted
    name.available = reply.available
    return reply.message
  })
}

async function sendCode() {
  if (sendBlock.value) return
  const path = forgot.value ? '/api/account/forgot/code' : '/api/account/register/code'
  const sent = await run('send', 'Asking the cloud to email a code…', async () => {
    const done = await api<Done>(path, { method: 'POST', body: JSON.stringify({ email: form.email.trim() }) })
    return done.message
  })
  if (sent) {
    step.sent = true
    step.checked = false
  }
}

async function checkCode() {
  if (checkBlock.value) return
  const checked = await run('check', 'Checking the code with the cloud…', async () => {
    const done = await api<Done>('/api/account/forgot/check', {
      method: 'POST',
      body: JSON.stringify({ email: form.email.trim(), code: form.code.trim() }),
    })
    return done.message
  })
  if (checked) step.checked = true
}

async function finish() {
  if (finishBlock.value) return
  const finished = await run('finish', forgot.value ? 'Setting the new password…' : 'Creating the account…', async () => {
    const done = forgot.value
      ? await api<Done>('/api/account/forgot/reset', {
          method: 'POST',
          body: JSON.stringify({ email: form.email.trim(), code: form.code.trim(), password: form.password }),
        })
      : await api<Done>('/api/account/register', {
          method: 'POST',
          body: JSON.stringify({ username: form.username.trim(), email: form.email.trim(), password: form.password, code: form.code.trim() }),
        })
    return done.message
  })
  if (finished) {
    form.password = ''
    form.confirm = ''
    emit('close', forgot.value ? form.email.trim() : form.username.trim() || form.email.trim(), step.note)
  }
}

onMounted(async () => {
  await nextTick()
  document.getElementById(forgot.value ? 'flow-email' : 'flow-username')?.focus()
})
</script>

<template>
  <div class="account-flow" :data-testid="'signin-' + kind">
    <h2 class="set-h">{{ title }}</h2>
    <p class="auth-sub">{{ lede }}</p>
    <form novalidate @submit.prevent>
      <template v-if="!forgot">
        <div class="field">
          <label for="flow-username">Username</label>
          <input id="flow-username" v-model="form.username" type="text" autocomplete="username" spellcheck="false" data-testid="account-flow-username">
        </div>
        <button class="btn btn-line wide" type="button" data-testid="account-flow-name-check" :disabled="Boolean(nameBlock)" :aria-busy="step.busy === 'name'" @click="checkName">
          {{ step.busy === 'name' ? 'Checking…' : 'Check the username' }}
        </button>
        <p v-if="nameBlock" class="hint">{{ nameBlock }}</p>
      </template>
      <div class="field">
        <label for="flow-email">Email</label>
        <input
          id="flow-email"
          v-model="form.email"
          type="email"
          autocomplete="email"
          spellcheck="false"
          placeholder="name@example.com"
          data-testid="account-flow-email"
          @input="step.sent = false; step.checked = false"
        >
      </div>
      <button class="btn btn-line wide" type="button" data-testid="account-flow-send" :disabled="Boolean(sendBlock)" :aria-busy="step.busy === 'send'" @click="sendCode">
        {{ step.busy === 'send' ? 'Sending…' : step.sent ? 'Send the code again' : 'Email me a code' }}
      </button>
      <p v-if="sendBlock" class="hint" data-testid="account-flow-send-reason">{{ sendBlock }}</p>
      <div class="field">
        <label for="flow-code">Code from the email</label>
        <input id="flow-code" v-model="form.code" type="text" inputmode="numeric" autocomplete="one-time-code" data-testid="account-flow-code" :disabled="!step.sent" @input="step.checked = false">
      </div>
      <template v-if="forgot">
        <button class="btn btn-line wide" type="button" data-testid="account-flow-check" :disabled="Boolean(checkBlock)" :aria-busy="step.busy === 'check'" @click="checkCode">
          {{ step.busy === 'check' ? 'Checking…' : 'Check the code' }}
        </button>
        <p v-if="checkBlock" class="hint">{{ checkBlock }}</p>
      </template>
      <div class="field">
        <label for="flow-password">{{ forgot ? 'New password' : 'Password' }}</label>
        <input id="flow-password" v-model="form.password" type="password" autocomplete="new-password" data-testid="account-flow-password">
      </div>
      <div class="field">
        <label for="flow-confirm">{{ forgot ? 'New password again' : 'Password again' }}</label>
        <input id="flow-confirm" v-model="form.confirm" type="password" autocomplete="new-password" data-testid="account-flow-confirm">
      </div>
      <button class="btn btn-fill" type="button" data-testid="account-flow-finish" :disabled="Boolean(finishBlock)" :aria-busy="step.busy === 'finish'" @click="finish">
        {{ step.busy === 'finish' ? 'Working…' : forgot ? 'Set the new password' : 'Create the account' }}
      </button>
      <p v-if="finishBlock" class="hint" data-testid="account-flow-finish-reason">{{ finishBlock }}</p>
      <p v-if="step.note" :class="step.bad ? 'err' : 'hint'" data-testid="account-flow-note" role="status">
        <Icon v-if="step.bad" name="warning" small /><span>{{ step.note }}</span>
      </p>
    </form>
    <p class="auth-note">
      <button class="btn-text" type="button" data-testid="account-flow-back" @click="emit('close', form.email.trim(), '')">Back to sign in</button>
    </p>
  </div>
</template>
