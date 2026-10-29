<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { applyLensView, deriveLensFromFrame, layoutLensCrop, renderLensPicker, sendControl } from '../legacy/app.js'
import { openSettings } from '../app/shell'
import { copyStageDetail, renderFsLine, requestStream } from '../app/stage'
import { state } from '../app/state'
import { loadDevices } from '../app/view'
import { live } from './store'

const well = ref<HTMLElement>()
const stage = live.stage
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
    <div id="well" ref="well" class="well" data-testid="live-well" :data-state="stage.name">
      <div id="lens-crop" class="lens-crop" data-testid="live-frame" :hidden="!showing">
        <video id="video" data-testid="live-video" playsinline muted @loadedmetadata="onMetadata" @timeupdate="onTimeUpdate" />
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
      <div id="pad" class="pad" data-testid="ptz-pad" role="group" aria-label="Pan and tilt" hidden>
        <button class="pk up" type="button" data-act="up" data-requires="device" aria-label="Tilt up" @click="sendControl('up')"><Icon name="chevron-up" /></button>
        <button class="pk left" type="button" data-act="left" data-requires="device" aria-label="Pan left" @click="sendControl('left')"><Icon name="chevron-left" /></button>
        <button class="pk home" type="button" data-act="home" data-requires="device" aria-label="Stop" @click="sendControl('home')"><Icon name="stop" small /></button>
        <button class="pk right" type="button" data-act="right" data-requires="device" aria-label="Pan right" @click="sendControl('right')"><Icon name="chevron-right" /></button>
        <button class="pk down" type="button" data-act="down" data-requires="device" aria-label="Tilt down" @click="sendControl('down')"><Icon name="chevron-down" /></button>
        <div class="pad-note">
          <span id="preset-label" data-testid="ptz-preset-label">Presets</span><span id="presets" data-testid="ptz-presets" class="presets" /><button id="tour" class="btn btn-text" data-testid="ptz-tour" type="button" data-act="cruise" data-requires="device" aria-pressed="false" @click="sendControl('cruise')">Tour</button><span id="control-status" data-testid="control-status" role="status" />
        </div>
      </div>
      <div id="fs-line" class="fs-line" :hidden="!stage.fs.shown"><span id="fs-name">{{ stage.fs.name }}</span><span id="fs-time">{{ stage.fs.time }}</span></div>
    </div>
    <div id="live-bar" class="live-bar" />
  </section>
  <section id="grid" data-testid="grid" class="gridview" aria-label="Every camera" :hidden="!live.panes.grid" />
</template>
