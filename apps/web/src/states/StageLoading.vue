<script setup lang="ts">
import type { StateSurface } from './types'

withDefaults(defineProps<{ label: string; surface?: StateSurface; line?: boolean }>(), { surface: 'stage', line: true })
</script>

<template>
  <div class="pointer-events-none absolute inset-0 flex items-center justify-center p-4 text-center" role="status" aria-live="polite">
    <span v-if="line" aria-hidden="true" :class="['state-line absolute top-0 left-0 h-0.5 w-[38%] rounded-r-sm', surface === 'stage' ? 'bg-stage-ink-2' : 'bg-accent']" />
    <p :class="['max-w-[44ch] text-13', surface === 'stage' ? 'text-stage-ink-2' : 'text-ink-2']">{{ label }}</p>
  </div>
</template>

<style>
.state-line { animation: state-line 1.4s linear infinite; }
@keyframes state-line { from { transform: translateX(-100%); } to { transform: translateX(265%); } }
@media (prefers-reduced-motion: reduce) {
  .state-line { animation: none; }
}
</style>
