<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import Icon from '../../components/Icon.vue'
import { SegmentedControl } from '../../components/ui/segmented-control'
import { addPreset, calibrate, gotoPreset, holdsAction, movingLensLabel, openEditor, pickPtzSpeed, pressControl, releaseControl, sendControl, tapControl, togglePad } from '../../app/controls'
import PtzEditor from '../PtzEditor.vue'
import { live } from '../store'

const props = defineProps<{ mode: 'overlay' | 'sheet' }>()

const LIMIT = 32
const pad = live.pad
const root = ref<HTMLElement>()
const ARROWS = [
  { act: 'up', label: 'Tilt up', icon: 'chevron-up' },
  { act: 'left', label: 'Pan left', icon: 'chevron-left' },
  { act: 'right', label: 'Pan right', icon: 'chevron-right' },
  { act: 'down', label: 'Tilt down', icon: 'chevron-down' },
]
const LENS = [
  { act: 'zoom-out', label: 'Zoom out', icon: 'zoom-out', text: 'Zoom out', gate: 'zoom' },
  { act: 'zoom-in', label: 'Zoom in', icon: 'zoom-in', text: 'Zoom in', gate: 'zoom' },
  { act: 'focus-near', label: 'Focus near', icon: 'focus', text: 'Near', gate: 'focus' },
  { act: 'focus-far', label: 'Focus far', icon: 'focus', text: 'Far', gate: 'focus' },
]
const SPEEDS = [
  { value: 'slow', label: 'Slow', testid: 'ptz-speed-slow' },
  { value: 'normal', label: 'Normal', testid: 'ptz-speed-normal' },
  { value: 'fast', label: 'Fast', testid: 'ptz-speed-fast' },
]
const lensName = computed(() => (live.padOpen ? movingLensLabel() : ''))
const heading = computed(() => (lensName.value ? 'Move · ' + lensName.value.toLowerCase() : 'Move'))
const lensKeys = computed(() => LENS.filter((key) => live.gate[key.gate].ok))
const full = computed(() => pad.presets.length >= LIMIT)
const hint = computed(() => (props.mode === 'sheet' ? 'Hold an arrow or drag the knob.' : 'Hold an arrow or drag the knob. Arrow keys work too.'))

function pressKey(act: string, event: PointerEvent) {
  if (event.button !== 0 || !holdsAction(act)) return
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  pressControl(act)
}

function clickKey(act: string, event: MouseEvent) {
  if (!holdsAction(act)) sendControl(act)
  else if (event.detail === 0) tapControl(act)
}

const knob = ref({ x: 0, y: 0, active: false })
let aimed = ''
const REACH = 40
const DEAD = 14

function grab(event: PointerEvent) {
  if (event.button !== 0) return
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  knob.value = { x: 0, y: 0, active: true }
  aimed = ''
}

function drag(event: PointerEvent) {
  if (!knob.value.active) return
  const box = (event.currentTarget as HTMLElement).parentElement!.getBoundingClientRect()
  let x = event.clientX - (box.left + box.width / 2)
  let y = event.clientY - (box.top + box.height / 2)
  const length = Math.hypot(x, y)
  if (length > REACH) {
    x = (x / length) * REACH
    y = (y / length) * REACH
  }
  knob.value = { x, y, active: true }
  if (length < DEAD) return
  const act = Math.abs(x) > Math.abs(y) ? (x > 0 ? 'right' : 'left') : y > 0 ? 'down' : 'up'
  if (act === aimed || !live.gate[act].ok) return
  aimed = act
  if (holdsAction(act)) pressControl(act)
  else sendControl(act)
}

function drop() {
  if (!knob.value.active) return
  knob.value = { x: 0, y: 0, active: false }
  if (aimed) releaseControl()
  aimed = ''
}

const knobStyle = computed(() => ({ transform: 'translate(' + knob.value.x + 'px, ' + knob.value.y + 'px)' }))

watch(
  () => live.padOpen,
  async (open) => {
    if (props.mode !== 'sheet' || !open) return
    await nextTick()
    root.value?.scrollIntoView({ block: 'nearest' })
  },
)
</script>

<template>
  <div id="pad" ref="root" class="pad" data-testid="ptz-pad" role="group" :aria-label="heading" :data-mode="mode" :hidden="!live.padOpen">
    <div class="pad-head">
      <Icon name="move" small />
      <span class="pad-title">{{ heading }}</span>
      <button class="pad-x" type="button" aria-label="Close Move" data-testid="ptz-close" @click="togglePad()"><Icon name="x" small /></button>
    </div>
    <div class="joy" role="group" aria-label="Pan and tilt">
      <button
        v-for="key in ARROWS"
        :key="key.act"
        :class="['pk', key.act]"
        type="button"
        :aria-label="key.label"
        :disabled="!live.gate[key.act].ok"
        @pointerdown="pressKey(key.act, $event)"
        @pointerup="releaseControl()"
        @pointercancel="releaseControl()"
        @lostpointercapture="releaseControl()"
        @click="clickKey(key.act, $event)"
      >
        <Icon :name="key.icon" />
      </button>
      <span
        class="knob"
        :data-active="knob.active ? 'true' : undefined"
        :style="knobStyle"
        aria-hidden="true"
        @pointerdown="grab"
        @pointermove="drag"
        @pointerup="drop"
        @pointercancel="drop"
        @lostpointercapture="drop"
      />
    </div>
    <div class="pad-panel">
      <div class="pad-row">
        <span class="pad-lab">Speed</span>
        <SegmentedControl data-testid="ptz-speed" label="Speed" size="sm" :options="SPEEDS" :model-value="pad.speed" @update:model-value="pickPtzSpeed" />
      </div>
      <div v-if="pad.presetsShown || live.gate.presetEdit.ok" class="pad-row top">
        <span id="preset-label" class="pad-lab" data-testid="ptz-preset-label">Presets</span>
        <div id="presets" class="chips" data-testid="ptz-presets">
          <button
            v-for="(preset, index) in pad.presets"
            :key="preset.token"
            class="pchip"
            type="button"
            data-testid="ptz-preset"
            :aria-label="'Go to preset ' + (index + 1) + ', ' + preset.name"
            @click="gotoPreset(preset.token, preset.name)"
          >
            <b aria-hidden="true">{{ index + 1 }}</b>{{ preset.name }}
          </button>
          <span v-if="!pad.presets.length" class="pad-why">No presets on this camera yet.</span>
          <button v-if="live.gate.presetEdit.ok" class="pchip" type="button" data-testid="ptz-preset-save" :disabled="pad.busy || full" @click="addPreset('')"><Icon name="plus" small />Save</button>
          <button v-if="live.gate.presetEdit.ok" id="ptz-edit" class="pchip" data-testid="ptz-edit" type="button" :aria-pressed="pad.editing ? 'true' : 'false'" aria-controls="ptz-editor" @click="openEditor()"><Icon name="pencil" small />Edit presets</button>
        </div>
      </div>
      <p v-if="full && live.gate.presetEdit.ok" class="pad-why">Save is off: the camera holds 32 presets.</p>
      <div v-if="lensKeys.length" class="pad-row" data-testid="ptz-lens">
        <span class="pad-lab">Lens</span>
        <div class="chips">
          <button
            v-for="key in lensKeys"
            :key="key.act"
            class="pchip"
            type="button"
            :data-testid="'ptz-' + key.act"
            :aria-label="key.label"
            @pointerdown="pressKey(key.act, $event)"
            @pointerup="releaseControl()"
            @pointercancel="releaseControl()"
            @lostpointercapture="releaseControl()"
            @click="clickKey(key.act, $event)"
          >
            <Icon :name="key.icon" small />{{ key.text }}
          </button>
        </div>
      </div>
      <div v-if="live.gate.cruise.ok || live.gate.home.ok || live.gate.calibrate.ok" class="pad-row">
        <span class="pad-lab">Motor</span>
        <div class="chips">
          <button v-if="live.gate.cruise.ok" id="tour" class="pchip" data-testid="ptz-tour" type="button" :aria-pressed="pad.tour ? 'true' : 'false'" @click="sendControl('cruise')"><Icon name="route" small />{{ pad.tour ? 'Stop tour' : 'Tour' }}</button>
          <button v-if="live.gate.home.ok" class="pchip" data-testid="ptz-stop" type="button" @click="sendControl('home')"><Icon name="stop" small />Stop</button>
          <button v-if="live.gate.calibrate.ok" class="pchip" data-testid="ptz-calibrate" type="button" :disabled="pad.busy" @click="calibrate()"><Icon name="crosshair" small />Calibrate the motor</button>
        </div>
      </div>
      <p class="pad-hint">{{ hint }}<span v-if="live.gate.aim.ok" data-testid="ptz-aim-hint"> Click the top lens to aim there.</span></p>
      <PtzEditor v-if="pad.editing" />
    </div>
  </div>
</template>
