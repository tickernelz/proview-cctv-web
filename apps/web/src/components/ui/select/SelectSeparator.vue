<script setup lang="ts">
import { SelectSeparator, type SelectSeparatorProps } from 'reka-ui'

const props = defineProps<SelectSeparatorProps>()
</script>

<template>
  <SelectSeparator v-bind="props" class="-mx-1 my-1 h-px bg-line-soft" />
</template>
