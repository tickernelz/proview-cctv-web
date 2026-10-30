<script setup lang="ts">
import { SelectItem, SelectItemIndicator, SelectItemText, useForwardPropsEmits, type SelectItemEmits, type SelectItemProps } from 'reka-ui'
import Icon from '../../Icon.vue'

const props = defineProps<SelectItemProps>()
const emits = defineEmits<SelectItemEmits>()
const forwarded = useForwardPropsEmits(props, emits)
</script>

<template>
  <SelectItem
    v-bind="forwarded"
    class="relative flex min-h-(--hit) cursor-pointer items-center gap-2 rounded-md py-1.5 pr-3 pl-8 text-14 text-ink outline-none select-none transition-colors duration-(--dur-1) ease-standard data-highlighted:bg-bg-3 focus-visible:shadow-[inset_0_0_0_2px_var(--focus)] data-disabled:cursor-not-allowed data-disabled:opacity-50"
  >
    <span class="absolute left-2 grid size-4 place-items-center text-accent">
      <SelectItemIndicator><Icon name="check" small /></SelectItemIndicator>
    </span>
    <SelectItemText><slot /></SelectItemText>
  </SelectItem>
</template>
