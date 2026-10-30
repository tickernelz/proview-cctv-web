<script setup lang="ts">
import { nextTick, reactive, ref } from 'vue'
import Icon from '../components/Icon.vue'
import { api, type ApiError } from '../lib/api'
import { sessionLost } from '../app/shell'
import { loadDevices } from '../app/view'

type Bind = { serial: string; status?: string; vendor_code?: string | number }

const serial = reactive({ sn: '', username: 'admin', password: '', missing: false })
const qr = reactive({ payload: '', missing: false })
const result = reactive({ busy: '' as '' | 'serial' | 'qr', text: '', ok: false })
const snInput = ref<HTMLInputElement>()
const qrInput = ref<HTMLInputElement>()

async function report(reply: Bind) {
  result.ok = true
  result.text = reply.status === 'already_bound' ? reply.serial + ' was already on this account (' + reply.vendor_code + ').' : reply.serial + ' is now on this account.'
  await loadDevices()
}

function refused(error: ApiError, form: 'serial' | 'qr') {
  if (sessionLost(error)) return
  result.ok = false
  result.text = 'Not added: ' + (form === 'qr' && error.code === 'bad_qr' ? 'that code is not a PROVIEW device code. ' : '') + error.message
}

async function bindSerial() {
  if (result.busy) return
  serial.missing = !serial.sn.trim()
  if (serial.missing) {
    result.text = ''
    await nextTick()
    snInput.value?.focus()
    return
  }
  result.busy = 'serial'
  result.text = ''
  try {
    const reply = await api<Bind>('/api/devices/bind', {
      method: 'POST',
      body: JSON.stringify({ sn: serial.sn.trim(), username: serial.username.trim() || 'admin', password: serial.password }),
    })
    serial.password = ''
    await report(reply)
  } catch (error) {
    refused(error as ApiError, 'serial')
  } finally {
    result.busy = ''
  }
}

async function bindQr() {
  if (result.busy) return
  qr.missing = !qr.payload.trim()
  if (qr.missing) {
    result.text = ''
    await nextTick()
    qrInput.value?.focus()
    return
  }
  result.busy = 'qr'
  result.text = ''
  try {
    const reply = await api<Bind>('/api/devices/bind/qr', { method: 'POST', body: JSON.stringify({ payload: qr.payload.trim() }) })
    qr.payload = ''
    await report(reply)
  } catch (error) {
    refused(error as ApiError, 'qr')
  } finally {
    result.busy = ''
  }
}
</script>

<template>
  <section id="add-device" class="add-device" data-testid="add-camera" aria-labelledby="add-device-title">
    <h3 id="add-device-title">Add a camera to this account</h3>
    <form id="add-device-form" class="add-form" novalidate @submit.prevent="bindSerial">
      <h4>By serial number</h4>
      <div :class="['field', { bad: serial.missing }]">
        <label for="add-sn">Serial number</label>
        <input id="add-sn" ref="snInput" v-model="serial.sn" type="text" spellcheck="false" autocomplete="off" placeholder="AASN000000000001" :aria-invalid="serial.missing" :aria-describedby="serial.missing ? 'add-sn-missing' : undefined">
        <p v-if="serial.missing" id="add-sn-missing" class="err"><Icon name="warning" small /><span>Type the serial printed on the camera or in the phone app.</span></p>
      </div>
      <div class="add-pair">
        <div class="field">
          <label for="add-user">Camera username</label>
          <input id="add-user" v-model="serial.username" type="text" spellcheck="false" autocomplete="off">
        </div>
        <div class="field">
          <label for="add-pass">Camera password</label>
          <input id="add-pass" v-model="serial.password" type="password" autocomplete="off">
        </div>
      </div>
      <button id="add-device-submit" class="btn btn-line" type="submit" :aria-busy="result.busy === 'serial'">
        <Icon name="plus" small />{{ result.busy === 'serial' ? 'Adding…' : 'Add this camera' }}
      </button>
    </form>
    <form id="add-qr-form" class="add-form" novalidate @submit.prevent="bindQr">
      <h4>From a QR code or share link</h4>
      <div :class="['field', { bad: qr.missing }]">
        <label for="add-qr">QR code text or share link</label>
        <input id="add-qr" ref="qrInput" v-model="qr.payload" type="text" spellcheck="false" autocomplete="off" placeholder="https://d.xmeye.net/CSee?shareInfo=..." :aria-invalid="qr.missing" :aria-describedby="qr.missing ? 'add-qr-missing' : undefined">
        <p v-if="qr.missing" id="add-qr-missing" class="err"><Icon name="warning" small /><span>Paste the text of the code or the share link.</span></p>
      </div>
      <button id="add-qr-submit" class="btn btn-line" type="submit" :aria-busy="result.busy === 'qr'">
        <Icon name="plus" small />{{ result.busy === 'qr' ? 'Reading the code…' : 'Add the camera in that code' }}
      </button>
    </form>
    <p v-if="result.text && !result.ok" id="add-device-status" class="err" role="alert" data-testid="add-camera-result"><Icon name="warning" small /><span>{{ result.text }}</span></p>
    <p v-else-if="result.text" id="add-device-status" class="ok-line" role="status" data-testid="add-camera-result"><Icon name="check" small /><span>{{ result.text }}</span></p>
  </section>
</template>
