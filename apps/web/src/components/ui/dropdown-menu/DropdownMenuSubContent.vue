<script setup lang="ts">
import { DropdownMenuPortal, DropdownMenuSubContent, useForwardPropsEmits, type DropdownMenuSubContentEmits, type DropdownMenuSubContentProps } from 'reka-ui'

defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<DropdownMenuSubContentProps>(), { sideOffset: 4, collisionPadding: 8 })
const emits = defineEmits<DropdownMenuSubContentEmits>()
const forwarded = useForwardPropsEmits(props, emits)
</script>

<template>
  <DropdownMenuPortal>
    <DropdownMenuSubContent v-bind="{ ...forwarded, ...$attrs }" class="menu-panel">
      <slot />
    </DropdownMenuSubContent>
  </DropdownMenuPortal>
</template>
