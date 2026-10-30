<script setup lang="ts">
import type { DotStatus } from './types'

defineProps<{ status: DotStatus; label: string }>()

const STATUSES: Record<DotStatus, string> = {
  online: 'bg-ok',
  offline: 'border-[1.5px] border-ink-3 bg-transparent',
  unknown: 'bg-ink-3',
  live: 'bg-live',
  warn: 'bg-warn',
  recording: 'bg-live',
}
</script>

<template>
  <span role="img" :aria-label="label" :data-status="status" :class="['inline-block size-2 shrink-0 rounded-full', STATUSES[status]]" />
</template>
