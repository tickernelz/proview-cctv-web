<script setup lang="ts">
import { watch } from 'vue'
import { liveHash, replaceHash } from '../shell/router'
import { shell } from '../shell/layout'
import { addLauncher, live } from './store'
import { openRecordings } from './timeline/engine'

const launcher = addLauncher({ id: 'pb-open', label: 'Playback', icon: 'history', slot: 'end', pressed: false, onClick: () => openRecordings('camera', 'pb-open') })

watch(
  () => live.screen,
  (screen) => {
    if (screen !== 'playback') return
    live.screen = ''
    replaceHash(liveHash(shell.mode))
    openRecordings('camera')
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
