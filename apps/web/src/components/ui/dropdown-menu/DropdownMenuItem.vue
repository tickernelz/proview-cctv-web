<script setup lang="ts">
import { DropdownMenuItem, useForwardPropsEmits, type DropdownMenuItemEmits, type DropdownMenuItemProps } from 'reka-ui'
import { computed } from 'vue'
import Icon from '../../Icon.vue'
import Kbd from '../kbd/Kbd.vue'

const props = defineProps<DropdownMenuItemProps & { icon?: string; shortcut?: string }>()
const emits = defineEmits<DropdownMenuItemEmits>()
const itemProps = computed(() => {
  const { icon: _icon, shortcut: _shortcut, ...rest } = props
  return rest
})
const forwarded = useForwardPropsEmits(itemProps, emits)
</script>

<template>
  <DropdownMenuItem v-bind="forwarded" :aria-keyshortcuts="shortcut" class="menu-row data-disabled:cursor-not-allowed data-disabled:opacity-50">
    <Icon v-if="icon" :name="icon" small />
    <span class="name min-w-0 flex-1 truncate"><slot /></span>
    <Kbd v-if="shortcut" class="ml-auto">{{ shortcut }}</Kbd>
  </DropdownMenuItem>
</template>
