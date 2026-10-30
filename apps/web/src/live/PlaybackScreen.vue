<script setup lang="ts">
import { watch } from 'vue'
import { liveHash, replaceHash } from '../shell/router'
import { shell } from '../shell/layout'
import { live } from './store'
import { openRecordings } from './timeline/engine'

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
</script>

<template>
  <span hidden />
</template>
