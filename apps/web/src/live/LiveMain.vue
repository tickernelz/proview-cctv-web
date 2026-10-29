<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { gotoPreset, sendControl } from '../app/controls'
import { applyLensView, deriveLensFromFrame, layoutLensCrop, renderLensPicker } from '../app/lens'
import { openSettings } from '../app/shell'
import { copyStageDetail, renderFsLine, requestStream } from '../app/stage'
import { state } from '../app/state'
import { loadDevices } from '../app/view'
import { live } from './store'

const well = ref<HTMLElement>()
const stage = live.stage
const pad = live.pad
const PAD = [
  { act: 'up', label: 'Tilt up', icon: 'chevron-up' },
  { act: 'left', label: 'Pan left', icon: 'chevron-left' },
  { act: 'home', label: 'Stop', icon: 'stop' },
  { act: 'right', label: 'Pan right', icon: 'chevron-right' },
  { act: 'down', label: 'Tilt down', icon: 'chevron-down' },
]
const showing = computed(() => stage.name === 'playing' || stage.name === 'stalled')
let lastVideoTime = -1
let observer: ResizeObserver | null = null

function checkPassword() {
  live.settings.focusKey = 'PROVIEW_DEVICE_PASS'
  openSettings('config')
}

function onMetadata() {
  if (!state.lens && state.selectedId) {
    const derived = deriveLensFromFrame()
    if (derived) {
      state.lens = derived
      renderLensPicker()
      applyLensView(state.lensView)
    }
  }
  layoutLensCrop()
}

function onTimeUpdate(event: Event) {
  const video = event.target as HTMLVideoElement
  if (video.currentTime !== lastVideoTime) {
    lastVideoTime = video.currentTime
    state.stage.lastFrameAt = Date.now()
  }
}

function onFullscreen() {
  layoutLensCrop()
  renderFsLine()
}

watch(showing, () => layoutLensCrop(), { flush: 'post' })

onMounted(() => {
  document.addEventListener('fullscreenchange', onFullscreen)
  if (typeof ResizeObserver === 'function' && well.value) {
    observer = new ResizeObserver(() => layoutLensCrop())
    observer.observe(well.value)
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('fullscreenchange', onFullscreen)
  observer?.disconnect()
})
</script>

<template>
  <div id="empty" class="emptywrap" data-testid="devices-empty" :hidden="!live.panes.empty">
    <div class="empty">
      <h2 id="empty-title" tabindex="-1">No cameras on this account yet</h2>
      <p id="empty-text">Add the camera in the phone app with the same account, then refresh. If the camera is on your own network, set its address under Settings, Configuration instead.</p>
      <div class="actions">
        <button id="refresh" class="btn btn-line" data-testid="devices-refresh" type="button" @click="loadDevices()"><Icon name="refresh" small />Refresh</button>
        <button id="empty-settings" class="btn btn-text" type="button" @click="openSettings('config', { focus: true })">Open Settings</button>
      </div>
    </div>
  </div>
  <section id="live" class="live" aria-label="Live" :hidden="!live.panes.live">
    <div id="well" ref="well" class="well" data-testid="live-well" :data-state="stage.name" :data-lens="stage.lens">
      <div id="lens-crop" class="lens-crop" data-testid="live-frame" :hidden="!showing" :data-flipped="stage.crop.flipped" :style="{ width: stage.crop.width, height: stage.crop.height }">
        <video id="video" data-testid="live-video" playsinline muted :style="stage.view" @loadedmetadata="onMetadata" @timeupdate="onTimeUpdate" />
      </div>
      <div id="prog" class="prog" :hidden="stage.name !== 'loading'" />
      <div id="well-msg" class="well-msg" :hidden="!stage.title" :role="stage.name === 'error' ? 'alert' : undefined">
        <h2 id="well-title" data-testid="live-title"><Icon v-if="stage.name === 'error'" name="warning" /><Icon v-else-if="stage.name === 'offline'" name="disconnect" />{{ stage.title }}</h2>
        <p id="well-text">{{ stage.text }}</p>
        <p id="well-code" class="mono" :hidden="!stage.code">{{ stage.code }}</p>
        <p id="well-stage" class="mono" :hidden="!stage.trailShown">{{ stage.trail }}</p>
        <div id="well-actions" class="actions" :hidden="stage.name !== 'error'">
          <button id="retry" class="btn btn-line inv" type="button" @click="requestStream()"><Icon name="refresh" small />Try again</button>
          <button id="check-password" class="btn btn-text inv" type="button" :hidden="!stage.passwordHint" @click="checkPassword">Check the camera password</button>
          <button id="well-stage-copy" class="btn btn-text inv" type="button" :hidden="!stage.canCopy" @click="copyStageDetail">{{ stage.copied ? 'Copied' : 'Copy the stage detail' }}</button>
        </div>
      </div>
      <div id="pad" class="pad" data-testid="ptz-pad" role="group" aria-label="Pan and tilt" :hidden="!live.padOpen">
        <button v-for="key in PAD" :key="key.act" :class="['pk', key.act]" type="button" :data-act="key.act" :aria-label="key.label" :disabled="!live.gate[key.act].ok" :title="live.gate[key.act].why" @click="sendControl(key.act)">
          <Icon :name="key.icon" :small="key.act === 'home'" />
        </button>
        <div class="pad-note">
          <span id="preset-label" data-testid="ptz-preset-label" :hidden="!pad.presetsShown">Presets</span>
          <span id="presets" data-testid="ptz-presets" class="presets">
            <button v-for="preset in pad.presets" :key="preset.token" type="button" data-testid="ptz-preset" :title="'GotoPreset ' + preset.token" @click="gotoPreset(preset.token, preset.name)">{{ preset.name }}</button>
          </span>
          <button id="tour" class="btn btn-text" data-testid="ptz-tour" type="button" data-act="cruise" :aria-pressed="pad.tour ? 'true' : 'false'" :disabled="!live.gate.cruise.ok" :title="live.gate.cruise.why" @click="sendControl('cruise')">Tour</button>
          <span id="control-status" data-testid="control-status" role="status">{{ pad.status }}</span>
        </div>
      </div>
      <div id="fs-line" class="fs-line" :hidden="!stage.fs.shown"><span id="fs-name">{{ stage.fs.name }}</span><span id="fs-time">{{ stage.fs.time }}</span></div>
    </div>
    <div id="live-bar" class="live-bar" />
  </section>
  <section id="grid" data-testid="grid" class="gridview" aria-label="Every camera" :hidden="!live.panes.grid" />
</template>
