<script setup lang="ts">
import Icon from '../components/Icon.vue'
import type { ChipTone } from './types'

withDefaults(defineProps<{ tone?: ChipTone; label: string; time?: string }>(), { tone: 'idle' })
</script>

<template>
  <span class="inline-flex h-(--chip) max-w-full min-w-0 items-center gap-1.5 rounded-sm bg-stage-matte px-2 font-mono text-12 whitespace-nowrap text-stage-ink tabular-nums" :data-tone="tone">
    <span v-if="tone === 'live' || tone === 'rec'" aria-hidden="true" class="size-2 shrink-0 rounded-full bg-stage-live" />
    <Icon v-else-if="tone === 'warn'" name="warning" small class="shrink-0 text-stage-warn" />
    <Icon v-else-if="tone === 'offline'" name="disconnect" small class="shrink-0 text-stage-ink-2" />
    <span v-else aria-hidden="true" class="size-2 shrink-0 rounded-full border-[1.5px] border-stage-ink-2" />
    <span class="min-w-0 truncate">{{ label }}</span>
    <span v-if="time" class="shrink-0 text-stage-ink-2">{{ time }}</span>
  </span>
</template>
