<script setup lang="ts">
import { SelectContent, SelectPortal, SelectViewport, useForwardPropsEmits, type SelectContentEmits, type SelectContentProps } from 'reka-ui'

defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<SelectContentProps>(), { position: 'popper', sideOffset: 4, collisionPadding: 8 })
const emits = defineEmits<SelectContentEmits>()
const forwarded = useForwardPropsEmits(props, emits)
</script>

<template>
  <SelectPortal>
    <SelectContent
      v-bind="{ ...forwarded, ...$attrs }"
      :class="[
        'z-50 max-h-[min(360px,var(--reka-select-content-available-height))] overflow-hidden rounded-lg border border-line bg-bg-2 text-ink shadow-menu transition-opacity duration-(--dur-2) ease-standard starting:opacity-0',
        position === 'popper' ? 'min-w-(--reka-select-trigger-width)' : '',
      ]"
    >
      <SelectViewport class="p-1">
        <slot />
      </SelectViewport>
    </SelectContent>
  </SelectPortal>
</template>
