<script setup lang="ts">
import Icon from '../components/Icon.vue'
import { chips, ev } from './events'
</script>

<template>
  <div class="ev-chips" role="group" aria-label="Event kind" data-testid="alarm-kinds">
    <button
      v-for="chip in chips"
      :key="chip.id"
      type="button"
      class="ev-chip"
      data-testid="alarm-kind"
      :data-kind="chip.id"
      :aria-pressed="ev.kind === chip.id ? 'true' : 'false'"
      @click="ev.kind = chip.id"
    >
      <Icon v-if="chip.icon" :name="chip.icon" small :class="'ev-ink-' + chip.id" />
      <span>{{ chip.label }}</span>
      <span v-if="chip.count !== undefined" class="ev-chip-n">{{ chip.count }}</span>
    </button>
  </div>
</template>
