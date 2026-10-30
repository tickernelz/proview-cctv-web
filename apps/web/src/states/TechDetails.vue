<script setup lang="ts">
import { ref, useId } from 'vue'
import { Button } from '../components/ui/button'

withDefaults(defineProps<{ text: string; align?: 'start' | 'end' }>(), { align: 'end' })
const id = useId()
const expanded = ref(false)
</script>

<template>
  <div v-if="text" :class="['flex max-w-full min-w-0 flex-col gap-1', align === 'start' ? 'items-start' : 'items-end']" data-testid="tech-details">
    <Button size="sm" variant="ghost" :icon="expanded ? 'chevron-up' : 'chevron-down'" :aria-expanded="expanded" :aria-controls="id" data-testid="tech-details-toggle" @click="expanded = !expanded">Details</Button>
    <pre
      v-show="expanded"
      :id="id"
      data-testid="tech-details-text"
      class="m-0 max-h-40 max-w-full overflow-auto rounded-md border border-line-soft bg-bg px-3 py-2 text-left font-mono text-12 whitespace-pre-wrap text-ink-2 [overflow-wrap:anywhere] select-text"
    >{{ text }}</pre>
  </div>
</template>
