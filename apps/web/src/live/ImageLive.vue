<script setup lang="ts">
import { onBeforeUnmount, onMounted, watch } from 'vue'
import { live } from './store'
import { imageLive, readImageLive, resetImageLive, uncoverLens, wakeCamera } from './stage/image-live'

let timer: number | undefined
onMounted(() => {
  readImageLive()
  timer = window.setInterval(readImageLive, 60000)
})
onBeforeUnmount(() => window.clearInterval(timer))
watch(() => [live.camera, live.settings.shown], () => {
  resetImageLive()
  readImageLive()
})
</script>

<template>
  <template v-if="imageLive.view">
    <p v-if="imageLive.view.privacy.supported && imageLive.view.privacy.value" class="dock-note warn" data-testid="image-live-privacy" role="status">
      Privacy shading is on, so the camera sends no picture.
      <button class="note-btn" type="button" :aria-busy="imageLive.busy" @click="uncoverLens">Uncover the lens</button>
    </p>
    <p v-if="imageLive.view.sleep.supported && imageLive.view.sleep.value?.sleeping" class="dock-note warn" data-testid="image-live-sleep" role="status">
      The camera is inside its timed sleep window, so it sends no picture.
      <button class="note-btn" type="button" :aria-busy="imageLive.busy" @click="wakeCamera">Wake it</button>
    </p>
  </template>
  <p v-if="imageLive.note" class="dock-note" data-testid="image-live-note" role="status">{{ imageLive.note }}</p>
</template>
