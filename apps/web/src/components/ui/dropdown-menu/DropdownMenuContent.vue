<script setup lang="ts">
import { DropdownMenuContent, DropdownMenuPortal, injectDropdownMenuRootContext, useForwardProps, type DropdownMenuContentProps } from 'reka-ui'
import { useAttrs } from 'vue'

defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<DropdownMenuContentProps>(), { side: 'top', align: 'end', sideOffset: 8, collisionPadding: 8 })
const forwarded = useForwardProps(props)
const attrs = useAttrs()
const root = injectDropdownMenuRootContext()
if (typeof attrs.id === 'string') root.contentId = attrs.id
</script>

<template>
  <DropdownMenuPortal>
    <DropdownMenuContent v-bind="{ ...forwarded, ...$attrs }" class="menu-panel">
      <slot />
    </DropdownMenuContent>
  </DropdownMenuPortal>
</template>
