<script setup lang="ts">
import { SelectLabel, type SelectLabelProps } from 'reka-ui'

const props = defineProps<SelectLabelProps>()
</script>

<template>
  <SelectLabel v-bind="props" class="px-3 pt-2 pb-1 font-mono text-12 text-ink-3">
    <slot />
  </SelectLabel>
</template>
