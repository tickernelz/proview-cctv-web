<script setup lang="ts">
import { computed } from 'vue'
import StageChip from './StageChip.vue'
import { clockHms } from './time'

const props = withDefaults(defineProps<{ lastFrameAt?: number | null; label?: string; chip?: boolean }>(), { lastFrameAt: null, label: 'Reconnecting.', chip: true })

const text = computed(() => (props.lastFrameAt ? props.label + ' Last frame' : props.label))
const time = computed(() => (props.lastFrameAt ? clockHms(props.lastFrameAt) : ''))
</script>

<template>
  <div class="pointer-events-none absolute inset-0" role="status" aria-live="polite">
    <span aria-hidden="true" class="absolute top-0 left-0 h-0.5 w-[38%] rounded-r-sm bg-stage-warn" />
    <div v-if="chip" class="absolute top-3 left-3 max-w-[calc(100%-24px)]">
      <StageChip tone="warn" :label="text" :time="time" />
    </div>
    <span v-else class="sr-only">{{ text }} {{ time }}</span>
  </div>
</template>
