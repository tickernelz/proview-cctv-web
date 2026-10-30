<script setup lang="ts">
import { SwitchRoot, SwitchThumb, useForwardPropsEmits, type SwitchRootEmits, type SwitchRootProps } from 'reka-ui'

const props = defineProps<SwitchRootProps>()
const emits = defineEmits<SwitchRootEmits>()
const forwarded = useForwardPropsEmits(props, emits)
</script>

<template>
  <SwitchRoot
    v-bind="forwarded"
    class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border border-line bg-bg-3 p-0.5 transition-colors duration-(--dur-1) ease-standard before:absolute before:top-1/2 before:left-1/2 before:h-(--hit) before:w-[max(100%,var(--hit))] before:-translate-1/2 before:content-[''] focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:border-accent data-[state=checked]:bg-accent"
  >
    <SwitchThumb
      class="pointer-events-none block size-3.5 rounded-full bg-ink-2 transition-[translate,background-color] duration-(--dur-1) ease-standard data-[state=checked]:translate-x-4 data-[state=checked]:bg-accent-ink"
    />
  </SwitchRoot>
</template>
