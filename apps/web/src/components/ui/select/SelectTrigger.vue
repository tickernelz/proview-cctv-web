<script setup lang="ts">
import { SelectIcon, SelectTrigger, type SelectTriggerProps } from 'reka-ui'
import Icon from '../../Icon.vue'

const props = defineProps<SelectTriggerProps>()
</script>

<template>
  <SelectTrigger
    v-bind="props"
    class="inline-flex h-(--hit) w-full min-w-0 cursor-pointer items-center justify-between gap-2 rounded-md border border-line bg-bg-2 px-3 text-left text-14 text-ink transition-colors duration-(--dur-1) ease-standard not-disabled:hover:bg-bg-3 focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-bad data-placeholder:text-ink-3"
  >
    <span class="min-w-0 flex-1 truncate"><slot /></span>
    <SelectIcon as-child><Icon name="chevron-down" small class="text-ink-3" /></SelectIcon>
  </SelectTrigger>
</template>
