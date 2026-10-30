<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../../components/Icon.vue'
import { hms } from '../../app/format'
import { state } from '../../app/state'
import { live } from '../store'

const playback = computed(() => live.mode === 'playback')
const stalled = computed(() => live.stage.name === 'stalled' && !playback.value)
const lastFrame = computed(() => {
  const now = live.clock
  return state.stage.lastFrameAt ? hms(state.stage.lastFrameAt) : now
})
const time = computed(() => (playback.value ? (live.playback.at === null ? '' : hms(live.playback.at)) : live.clock))
const quality = computed(() => (playback.value ? live.playback.quality : live.quality === 'sub' ? 'SD' : 'HD'))
const name = computed(() => (live.stage.fs.shown ? live.camera : ''))
</script>

<template>
  <div class="plate" data-testid="live-plate" :data-tone="stalled ? 'warn' : playback ? 'rec' : 'live'">
    <template v-if="stalled">
      <Icon name="warning" small />
      <span>Reconnecting. Last frame {{ lastFrame }}.</span>
    </template>
    <template v-else>
      <span v-if="name" class="plate-name">{{ name }}</span>
      <span class="plate-dot" aria-hidden="true" />
      <span class="plate-word">{{ playback ? 'REC' : 'LIVE' }}</span>
      <span data-testid="live-clock" class="plate-time">{{ time }}</span>
      <span class="plate-quality">{{ quality }}</span>
    </template>
  </div>
</template>
