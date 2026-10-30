<script setup lang="ts">
import Icon from '../../Icon.vue'

defineProps<{ icon?: string; title: string; description?: string; compact?: boolean }>()
defineSlots<{ actions?(): unknown }>()
</script>

<template>
  <div :class="['flex flex-col items-center justify-center text-center', compact ? 'gap-1.5 px-4 py-6' : 'gap-2 px-6 py-12']">
    <Icon v-if="icon" :name="icon" class="mb-1 text-ink-3" />
    <p :class="['font-medium text-balance text-ink', compact ? 'text-14' : 'text-15']">{{ title }}</p>
    <p v-if="description" :class="['max-w-[44ch] text-pretty text-ink-2', compact ? 'text-13' : 'text-14']">{{ description }}</p>
    <div v-if="$slots.actions" :class="['flex flex-wrap justify-center gap-2', compact ? 'mt-1' : 'mt-2']">
      <slot name="actions" />
    </div>
  </div>
</template>
