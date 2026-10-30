<script setup lang="ts">
import { SliderRange, SliderRoot, SliderThumb, SliderTrack, useForwardPropsEmits, type SliderRootEmits, type SliderRootProps } from 'reka-ui'
import { computed } from 'vue'

const props = defineProps<Omit<SliderRootProps, 'orientation'> & { ariaLabel: string }>()
const emits = defineEmits<SliderRootEmits>()
const rootProps = computed(() => {
  const { ariaLabel: _label, ...rest } = props
  return rest
})
const forwarded = useForwardPropsEmits(rootProps, emits)
const thumbs = computed(() => (props.modelValue ?? props.defaultValue ?? [props.min ?? 0]).length)
</script>

<template>
  <SliderRoot
    v-bind="forwarded"
    class="relative flex h-(--hit) w-full touch-none items-center select-none data-disabled:cursor-not-allowed data-disabled:opacity-50"
  >
    <SliderTrack class="relative h-1 grow overflow-hidden rounded-full bg-line">
      <SliderRange class="absolute h-full bg-accent" />
    </SliderTrack>
    <SliderThumb
      v-for="index in thumbs"
      :key="index"
      :aria-label="thumbs > 1 ? `${ariaLabel} ${index}` : ariaLabel"
      class="block size-4 cursor-grab rounded-full border-2 border-accent bg-bg-2 shadow-menu transition-colors duration-(--dur-1) ease-standard hover:bg-bg-3 focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus active:cursor-grabbing data-disabled:cursor-not-allowed"
    />
  </SliderRoot>
</template>
