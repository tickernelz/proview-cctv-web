<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import Icon from '../components/Icon.vue'
import AddCamera from './AddCamera.vue'
import FisheyeOverlay from './FisheyeOverlay.vue'
import GridView from './GridView.vue'
import LiveBar from './LiveBar.vue'
import DockStatus from './stage/DockStatus.vue'
import PipInset from './stage/PipInset.vue'
import PtzPad from './stage/PtzPad.vue'
import StatusPlate from './stage/StatusPlate.vue'
import { frameFit } from './stage/pip'
import { padSheet } from './stage/sheet'
import { aimAt } from '../app/controls'
import { applyLensView, deriveLensFromFrame, layoutLensCrop, renderLensPicker } from '../app/lens'
import { openSettings } from '../app/shell'
import { copyStageDetail, renderFsLine, requestStream } from '../app/stage'
import { state } from '../app/state'
import { loadDevices } from '../app/view'
import { live } from './store'

const well = ref<HTMLElement>()
const stage = live.stage
const showing = computed(() => stage.name === 'playing' || stage.name === 'stalled' || stage.name === 'playback')
const recovers = computed(() => stage.name === 'error' && live.mode !== 'playback')
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
      <AddCamera v-if="live.panes.empty" :key="live.signin.generation" />
    </div>
  </div>
  <section id="live" class="live" aria-label="Live" :hidden="!live.panes.live">
    <div id="well" ref="well" class="well" data-testid="live-well" :data-state="stage.name" :data-lens="stage.lens" :style="{ height: showing ? frameFit.height : '' }">
      <div class="stage-frame" :hidden="!showing" :style="{ width: stage.crop.width, height: stage.crop.height }">
        <div id="lens-crop" class="lens-crop" data-testid="live-frame" :data-flipped="stage.crop.flipped" :data-aim="live.padOpen && live.gate.aim.ok ? 'on' : undefined" @click="aimAt">
          <video id="video" data-testid="live-video" playsinline muted :style="stage.view" @loadedmetadata="onMetadata" @timeupdate="onTimeUpdate" />
        </div>
        <StatusPlate />
        <PipInset />
        <PtzPad v-if="!padSheet.on" mode="overlay" />
      </div>
      <div id="prog" class="prog" :hidden="stage.name !== 'loading'" />
      <div id="well-msg" class="well-msg" :hidden="!stage.title" :role="stage.name === 'error' ? 'alert' : undefined">
        <h2 id="well-title" data-testid="live-title"><Icon v-if="stage.name === 'error'" name="warning" /><Icon v-else-if="stage.name === 'offline'" name="disconnect" />{{ stage.title }}</h2>
        <p id="well-text">{{ stage.text }}</p>
        <p id="well-code" class="mono" :hidden="!stage.code">{{ stage.code }}</p>
        <p id="well-stage" class="mono" :hidden="!stage.trailShown">{{ stage.trail }}</p>
        <div id="well-actions" class="actions" :hidden="!recovers">
          <button id="retry" class="btn btn-line inv" type="button" @click="requestStream()"><Icon name="refresh" small />Try again</button>
          <button id="check-password" class="btn btn-text inv" type="button" :hidden="!stage.passwordHint" @click="checkPassword">Check the camera password</button>
          <button id="well-stage-copy" class="btn btn-text inv" type="button" :hidden="!stage.canCopy" @click="copyStageDetail">{{ stage.copied ? 'Copied' : 'Copy the stage detail' }}</button>
        </div>
      </div>
    </div>
    <LiveBar />
    <DockStatus />
    <PtzPad v-if="padSheet.on" mode="sheet" />
  </section>
  <section id="grid" data-testid="grid" class="gridview" aria-label="Every camera" :hidden="!live.panes.grid"><GridView /></section>
  <FisheyeOverlay />
</template>
