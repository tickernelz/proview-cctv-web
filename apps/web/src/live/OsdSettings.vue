<script setup lang="ts">
import { reactive, watch } from 'vue'

export type OsdState = {
  channel: number
  name?: string | null
  titles?: string[]
  title_shown?: boolean | null
  time_shown?: boolean | null
  dot_required?: boolean
}

export type TitleDot = { width: number; height: number; bits: string }

const props = defineProps<{ osd: OsdState; busy: boolean; label: string }>()
const emit = defineEmits<{ save: [patch: Record<string, string>, dot: TitleDot | null] }>()

const DOT_HEIGHT = 24
const DOT_FONT = '18px sans-serif'

const form = reactive({ name: '', title: false, time: false, dot: true })

function fill() {
  form.name = props.osd.name || ''
  form.title = Boolean(props.osd.title_shown)
  form.time = Boolean(props.osd.time_shown)
  form.dot = false
}

watch(() => props.osd, fill, { immediate: true })

function titleDot(text: string): TitleDot | null {
  const canvas = document.createElement('canvas')
  const context = canvas.getContext('2d', { willReadFrequently: true })
  if (!context) return null
  context.font = DOT_FONT
  const measured = Math.ceil(context.measureText(text).width)
  const width = Math.max(8, measured % 8 === 0 ? measured : measured + 8 - (measured % 8))
  canvas.width = width
  canvas.height = DOT_HEIGHT
  context.fillStyle = '#ffffff'
  context.fillRect(0, 0, width, DOT_HEIGHT)
  context.fillStyle = '#000000'
  context.font = DOT_FONT
  context.textBaseline = 'middle'
  context.fillText(text, 0, DOT_HEIGHT / 2)
  const pixels = context.getImageData(0, 0, width, DOT_HEIGHT).data
  const bytes = new Uint8Array((width * DOT_HEIGHT) / 8)
  for (let index = 0; index < width * DOT_HEIGHT; index += 1) {
    const offset = index * 4
    const white = pixels[offset] === 255 && pixels[offset + 1] === 255 && pixels[offset + 2] === 255
    if (!white) bytes[index >> 3] |= 1 << (7 - (index & 7))
  }
  return { width, height: DOT_HEIGHT, bits: Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('') }
}

function save() {
  const osd = props.osd
  const patch: Record<string, string> = {}
  const name = form.name.trim()
  if (name !== (osd.name || '')) patch.name = name
  if (osd.title_shown !== null && osd.title_shown !== undefined && form.title !== osd.title_shown) patch.title = String(form.title)
  if (osd.time_shown !== null && osd.time_shown !== undefined && form.time !== osd.time_shown) patch.time = String(form.time)
  if (Object.keys(patch).length) patch.channel = String(osd.channel)
  emit('save', patch, patch.name && osd.dot_required && form.dot ? titleDot(name) : null)
}
</script>

<template>
  <div id="device-osd" data-testid="device-osd">
    <h3 class="set-h">On-screen text</h3>
    <div class="row">
      <label class="lab" for="device-osd-name">Channel title<div class="hint">Burned into the video; the cloud device name in the list is separate.</div></label>
      <div class="ctl-col">
        <input id="device-osd-name" v-model="form.name" type="text" maxlength="32">
        <span class="hint">Up to 32 Latin or 21 other characters, the mobile app limit.</span>
      </div>
    </div>
    <div class="row">
      <div class="lab">Show title</div>
      <div class="ctl-col">
        <label class="switchrow"><input id="device-osd-title" v-model="form.title" type="checkbox" role="switch" :disabled="osd.title_shown === null || osd.title_shown === undefined"><span>Overlay the channel title</span></label>
        <p v-if="osd.title_shown === null || osd.title_shown === undefined" class="hint">AVEnc.VideoWidget carries no ChannelTitleAttribute.</p>
      </div>
    </div>
    <div class="row">
      <div class="lab">Show time</div>
      <div class="ctl-col">
        <label class="switchrow"><input id="device-osd-time" v-model="form.time" type="checkbox" role="switch" :disabled="osd.time_shown === null || osd.time_shown === undefined"><span>Overlay the date and time</span></label>
        <p v-if="osd.time_shown === null || osd.time_shown === undefined" class="hint">AVEnc.VideoWidget carries no TimeTitleAttribute.</p>
      </div>
    </div>
    <div class="row">
      <div class="lab">Title bitmap</div>
      <div class="ctl-col">
        <label class="switchrow"><input id="device-osd-dot" v-model="form.dot" type="checkbox" role="switch" :disabled="!osd.dot_required"><span>Redraw the title bitmap after a rename</span></label>
        <p v-if="!osd.dot_required" class="hint">This camera reports more than one channel title, so it draws the text itself and takes no bitmap.</p>
        <p v-else class="hint">Older single-channel firmware shows a 1-bit picture of the title (TitleDot) instead of drawing the text; the mobile app sends one on every rename. Leave it off when the video already shows the new title.</p>
      </div>
    </div>
    <button class="btn btn-fill" type="button" data-testid="device-save-osd" :aria-busy="busy" @click="save">{{ label }}</button>
  </div>
</template>
