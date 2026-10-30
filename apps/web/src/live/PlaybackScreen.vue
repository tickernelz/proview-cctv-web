<script setup lang="ts">
import { watch } from 'vue'
import { state as page } from '../app/state'
import { setView } from '../app/view'
import { liveHash, replaceHash } from '../shell/router'
import { addLauncher, live } from './store'
import { openPlayback, openRecordings } from './timeline/engine'

const launcher = addLauncher({ id: 'pb-open', label: 'Playback', icon: 'history', slot: 'end', pressed: false, onClick: () => void enter() })

async function enter() {
  if (page.view !== 'live') setView('live')
  const started = await openPlayback()
  if (started === false) openRecordings('camera')
}

watch(
  () => live.screen,
  (screen) => {
    if (screen !== 'playback') return
    live.screen = ''
    replaceHash(liveHash('single'))
    void enter()
  },
  { immediate: true },
)

watch(
  () => live.mode,
  (mode) => (launcher.pressed = mode === 'playback'),
)
</script>

<template>
  <span hidden />
</template>
