<script setup lang="ts">
import { watch } from 'vue'
import { state as page } from '../app/state'
import { setView } from '../app/view'
import { liveHash, replaceHash } from '../shell/router'
import { addLauncher, live } from './store'
import { openRecordings } from './timeline/engine'

const launcher = addLauncher({ id: 'pb-open', label: 'Playback', icon: 'history', slot: 'end', pressed: false, onClick: () => enter('pb-open') })

function enter(from = '') {
  if (page.view !== 'live') setView('live')
  openRecordings('camera', from)
}

watch(
  () => live.screen,
  (screen) => {
    if (screen !== 'playback') return
    live.screen = ''
    replaceHash(liveHash('single'))
    enter()
  },
  { immediate: true },
)

watch(
  () => live.mode === 'playback',
  (on) => (launcher.pressed = on),
)
</script>

<template>
  <span hidden />
</template>
