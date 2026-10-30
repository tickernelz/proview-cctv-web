<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import { KIND, ev, nameOf, rangePhrase, retry, rows, shown } from './events'

const props = defineProps<{ compact?: boolean; skeletons?: number }>()
const camera = computed(() => nameOf(ev.asked.serial))
const loading = computed(() => ev.phase === 'loading' && !rows.value.length)
const empty = computed(() => ev.phase === 'ok' && !rows.value.length && ev.kind === 'all')
const filteredEmpty = computed(() => ev.phase === 'ok' && !shown.value.length && ev.kind !== 'all')
const kindWord = computed(() => (ev.kind === 'all' ? '' : KIND[ev.kind].word))
const lines = computed(() => Array.from({ length: props.skeletons ?? 4 }, (_, index) => index))
</script>

<template>
  <div v-if="ev.problem" class="ev-problem" data-testid="alarm-error" role="alert">
    <Icon name="warning" small class="ev-problem-icon" />
    <div class="ev-problem-body">
      <p class="ev-problem-title" data-testid="alarm-error-title">{{ ev.problem.title }}</p>
      <p class="ev-problem-text">{{ ev.problem.text }}</p>
      <details class="ev-details">
        <summary data-testid="alarm-error-toggle">Details</summary>
        <pre data-testid="alarm-error-detail">{{ ev.problem.detail }}</pre>
      </details>
      <Button size="sm" icon="refresh" data-testid="alarm-retry" @click="retry">Try again</Button>
    </div>
  </div>
  <div v-if="loading" class="ev-loading" data-testid="alarm-loading-block" :data-compact="compact ? 'true' : undefined">
    <p class="ev-line" data-testid="alarm-loading" role="status">Loading events for {{ camera }}…</p>
    <div v-for="line in lines" :key="line" class="ev-sk-row" aria-hidden="true">
      <span class="ev-sk ev-sk-thumb" />
      <span class="ev-sk-text"><span class="ev-sk ev-sk-a" /><span class="ev-sk ev-sk-b" /></span>
    </div>
  </div>
  <div v-else-if="empty" class="ev-empty" data-testid="alarm-empty-block">
    <Icon name="bell" class="ev-empty-icon" />
    <p class="ev-empty-title" data-testid="alarm-empty">No events {{ rangePhrase() }}</p>
    <p class="ev-empty-text">{{ camera }} reported no motion or alarms {{ rangePhrase() }}.</p>
  </div>
  <div v-else-if="filteredEmpty" class="ev-empty" data-testid="alarm-filtered-empty">
    <p class="ev-empty-title">No {{ kindWord }} events {{ rangePhrase() }}</p>
    <p class="ev-empty-text">{{ camera }} reported no {{ kindWord }} events {{ rangePhrase() }}.</p>
    <Button size="sm" data-testid="alarm-show-all" @click="ev.kind = 'all'">Show all events</Button>
  </div>
</template>
