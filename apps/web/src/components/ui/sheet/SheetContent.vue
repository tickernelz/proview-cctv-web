<script setup lang="ts">
import { DialogClose, DialogContent, DialogOverlay, DialogPortal, useForwardPropsEmits, type DialogContentEmits, type DialogContentProps } from 'reka-ui'
import { computed } from 'vue'
import IconButton from '../icon-button/IconButton.vue'
import type { SheetSide } from './types'

defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<DialogContentProps & { side?: SheetSide; hideClose?: boolean }>(), { side: 'right' })
const emits = defineEmits<DialogContentEmits>()
const contentProps = computed(() => {
  const { side: _side, hideClose: _hideClose, ...rest } = props
  return rest
})
const forwarded = useForwardPropsEmits(contentProps, emits)

const SIDES: Record<SheetSide, string> = {
  right: 'inset-y-0 right-0 w-(--panel) max-w-full border-l starting:translate-x-2',
  left: 'inset-y-0 left-0 w-(--panel) max-w-full border-r starting:-translate-x-2',
  bottom: 'inset-x-0 bottom-0 max-h-[85vh] rounded-t-lg border-t pb-[max(16px,env(safe-area-inset-bottom))] starting:translate-y-2',
}
</script>

<template>
  <DialogPortal>
    <DialogOverlay class="fixed inset-0 z-40 bg-scrim transition-opacity duration-(--dur-2) ease-standard starting:opacity-0" />
    <DialogContent
      v-bind="{ ...forwarded, ...$attrs }"
      :data-side="side"
      :class="[
        'fixed z-40 flex flex-col gap-4 overflow-auto border-line-soft bg-bg-2 p-4 text-ink shadow-panel outline-none transition-[opacity,translate] duration-(--dur-2) ease-standard starting:opacity-0',
        SIDES[side],
      ]"
    >
      <slot />
      <DialogClose v-if="!hideClose" as-child>
        <IconButton label="Close" icon="x" size="sm" class="absolute top-3 right-3" />
      </DialogClose>
    </DialogContent>
  </DialogPortal>
</template>
