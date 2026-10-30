<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../../Icon.vue'
import type { BadgeTone } from './types'

const props = withDefaults(defineProps<{ tone?: BadgeTone; icon?: string }>(), { tone: 'neutral' })

const TONES: Record<BadgeTone, string> = {
  neutral: 'bg-bg-3 text-ink-2',
  accent: 'bg-accent-soft text-accent',
  live: 'bg-live-soft text-live',
  ok: 'bg-bg-3 text-ok',
  warn: 'bg-bg-3 text-warn',
  bad: 'bg-bg-3 text-bad',
}

const shownIcon = computed(() => props.icon ?? (props.tone === 'warn' ? 'warning' : undefined))
</script>

<template>
  <span :data-tone="tone" :class="['inline-flex h-(--chip) shrink-0 items-center gap-1 rounded-sm px-2 text-12 font-medium whitespace-nowrap tabular-nums', TONES[tone]]">
    <Icon v-if="shownIcon" :name="shownIcon" small />
    <slot />
  </span>
</template>
