<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import Icon from '../components/Icon.vue'
import { Button } from '../components/ui/button'
import StageButton from './StageButton.vue'
import { detailText } from './time'
import type { DetailRows, StateSurface } from './types'

const props = withDefaults(
  defineProps<{ title: string; description?: string; details?: DetailRows | null; surface?: StateSurface; compact?: boolean; retry?: boolean; retryLabel?: string; busy?: boolean; open?: boolean }>(),
  { description: '', details: null, surface: 'stage', retry: true, retryLabel: 'Try again', open: false },
)
const emit = defineEmits<{ retry: [] }>()
defineSlots<{ actions?(): unknown }>()

const id = useId()
const expanded = ref(props.open)
const text = computed(() => (props.details ? detailText(props.details) : ''))
const stage = computed(() => props.surface === 'stage')
</script>

<template>
  <div
    :class="[
      'flex flex-col items-center justify-center text-center',
      stage ? 'absolute inset-0 overflow-auto text-stage-ink' : 'text-ink',
      compact ? 'gap-1.5 px-3 py-2' : 'gap-2.5 px-6 py-4',
    ]"
    data-state="error"
  >
    <div role="alert" :class="['flex flex-col items-center', compact ? 'gap-1' : 'gap-2']">
      <Icon name="warning" :class="[compact ? '' : 'size-8', stage ? 'text-stage-warn' : 'text-bad']" />
      <p :class="['font-semibold text-balance', compact ? 'text-15' : 'text-17']">{{ title }}</p>
      <p v-if="description" :class="['max-w-[48ch] text-pretty', compact ? 'text-13' : 'text-14', stage ? 'text-stage-ink-2' : 'text-ink-2']">{{ description }}</p>
    </div>
    <div v-if="retry || text || $slots.actions" class="mt-1 flex flex-wrap justify-center gap-2">
      <template v-if="stage">
        <StageButton v-if="retry" icon="refresh" :busy="busy" data-testid="state-retry" @click="emit('retry')">{{ retryLabel }}</StageButton>
        <StageButton v-if="text" :icon="expanded ? 'chevron-up' : 'chevron-down'" :aria-expanded="expanded" :aria-controls="id" data-testid="state-details-toggle" @click="expanded = !expanded">Details</StageButton>
      </template>
      <template v-else>
        <Button v-if="retry" size="sm" icon="refresh" :loading="busy" data-testid="state-retry" @click="emit('retry')">{{ retryLabel }}</Button>
        <Button v-if="text" size="sm" variant="ghost" :icon="expanded ? 'chevron-up' : 'chevron-down'" :aria-expanded="expanded" :aria-controls="id" data-testid="state-details-toggle" @click="expanded = !expanded">Details</Button>
      </template>
      <slot name="actions" />
    </div>
    <pre
      v-if="text"
      v-show="expanded"
      :id="id"
      data-testid="state-details"
      :class="[
        'relative z-2 max-h-40 max-w-full overflow-auto rounded-md border px-3 py-2 text-left font-mono text-12 whitespace-pre-wrap [overflow-wrap:anywhere] select-text',
        stage ? 'border-stage-line bg-stage-matte text-stage-ink-2' : 'border-line-soft bg-bg-2 text-ink-2',
      ]"
    >{{ text }}</pre>
  </div>
</template>
