<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import StageButton from './StageButton.vue'
import StageChip from './StageChip.vue'
import { clockHm, clockHms } from './time'

const props = withDefaults(
  defineProps<{ name?: string; since?: number | null; lastFrameAt?: number | null; snapshot?: string | null; detail?: string; compact?: boolean; chip?: boolean; retry?: boolean; busy?: boolean }>(),
  { since: null, lastFrameAt: null, snapshot: null, detail: '', chip: true, retry: true },
)
const emit = defineEmits<{ retry: [] }>()
defineSlots<{ actions?(): unknown }>()

const sinceText = computed(() => (props.since ? 'Offline since ' + clockHm(props.since) : 'Offline'))
const title = computed(() => (props.compact ? sinceText.value : (props.name ? props.name : 'The camera') + ' is offline'))
const sentence = computed(() => {
  const parts: string[] = []
  if (!props.compact && props.since) parts.push(sinceText.value + '.')
  if (props.lastFrameAt) parts.push('Last frame ' + clockHms(props.lastFrameAt) + '.')
  if (!parts.length && props.detail) parts.push(props.detail.replace(/\.?$/, '.'))
  return parts.join(' ')
})
</script>

<template>
  <div class="absolute inset-0 overflow-hidden" data-state="offline">
    <img v-if="snapshot" :src="snapshot" alt="" class="absolute inset-0 size-full object-cover opacity-35 grayscale">
    <div v-if="chip && !compact" class="absolute top-3 left-3"><StageChip tone="offline" :label="sinceText" /></div>
    <div :class="['relative flex size-full flex-col items-center justify-center text-center text-stage-ink', compact ? 'gap-1.5 px-3' : 'gap-2.5 px-6']" role="status">
      <Icon name="disconnect" :class="compact ? '' : 'size-8'" />
      <p :class="['font-semibold text-balance', compact ? 'text-15' : 'text-17']">{{ title }}</p>
      <p v-if="sentence" :class="['max-w-[44ch] text-pretty text-stage-ink-2', compact ? 'text-13' : 'text-14']">{{ sentence }}</p>
      <div v-if="retry || $slots.actions" class="mt-1 flex flex-wrap justify-center gap-2">
        <StageButton v-if="retry" icon="refresh" :busy="busy" data-testid="state-retry" @click="emit('retry')">Try again</StageButton>
        <slot name="actions" />
      </div>
    </div>
  </div>
</template>
