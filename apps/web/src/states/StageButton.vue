<script setup lang="ts">
import Icon from '../components/Icon.vue'

withDefaults(defineProps<{ icon?: string; iconEnd?: string; disabled?: boolean; busy?: boolean; type?: 'button' | 'submit' }>(), { type: 'button' })
</script>

<template>
  <button
    :type="type"
    :disabled="disabled || busy"
    :aria-busy="busy ? 'true' : undefined"
    class="relative z-2 inline-flex h-8 shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-md border border-stage-line bg-stage-matte px-3 text-13 font-medium whitespace-nowrap text-stage-ink transition-[border-color] duration-(--dur-1) ease-standard not-disabled:hover:border-stage-ink-2 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus focus-visible:outline-solid disabled:cursor-not-allowed disabled:text-stage-ink-2 pointer-coarse:h-(--hit)"
  >
    <Icon v-if="icon" :name="icon" small :class="busy ? 'motion-safe:animate-spin' : ''" />
    <slot />
    <Icon v-if="iconEnd" :name="iconEnd" small />
  </button>
</template>
