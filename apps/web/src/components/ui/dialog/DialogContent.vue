<script setup lang="ts">
import { DialogClose, DialogContent, DialogOverlay, DialogPortal, useForwardPropsEmits, type DialogContentEmits, type DialogContentProps } from 'reka-ui'
import { computed } from 'vue'
import IconButton from '../icon-button/IconButton.vue'

defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<DialogContentProps & { size?: 'sm' | 'md' | 'lg'; hideClose?: boolean }>(), { size: 'md' })
const emits = defineEmits<DialogContentEmits>()
const contentProps = computed(() => {
  const { size: _size, hideClose: _hideClose, ...rest } = props
  return rest
})
const forwarded = useForwardPropsEmits(contentProps, emits)

const SIZES = { sm: 'max-w-[400px]', md: 'max-w-[480px]', lg: 'max-w-[640px]' } as const
</script>

<template>
  <DialogPortal>
    <DialogOverlay class="fixed inset-0 z-40 bg-scrim transition-opacity duration-(--dur-2) ease-standard starting:opacity-0" />
    <DialogContent
      v-bind="{ ...forwarded, ...$attrs }"
      :class="[
        'fixed inset-0 z-40 m-auto flex h-fit max-h-[calc(100dvh-32px)] w-[calc(100vw-32px)] flex-col gap-4 overflow-auto rounded-lg border border-line-soft bg-bg-2 p-6 text-ink shadow-panel outline-none transition-[opacity,translate] duration-(--dur-2) ease-standard starting:translate-y-2 starting:opacity-0',
        SIZES[size],
      ]"
    >
      <slot />
      <DialogClose v-if="!hideClose" as-child>
        <IconButton label="Close" icon="x" size="sm" class="absolute top-3 right-3" />
      </DialogClose>
    </DialogContent>
  </DialogPortal>
</template>
