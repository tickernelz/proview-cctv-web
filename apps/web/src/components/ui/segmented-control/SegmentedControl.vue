<script setup lang="ts">
import { ToggleGroupItem, ToggleGroupRoot, type AcceptableValue } from 'reka-ui'
import { computed } from 'vue'
import Icon from '../../Icon.vue'
import Tooltip from '../tooltip/Tooltip.vue'
import type { SegmentedOption } from './types'

const props = withDefaults(defineProps<{
  options: SegmentedOption[]
  modelValue: string
  label: string
  size?: 'sm' | 'md'
  iconOnly?: boolean
  disabled?: boolean
}>(), { size: 'md' })

const emit = defineEmits<{ 'update:modelValue': [value: string] }>()

function onUpdate(value: AcceptableValue | AcceptableValue[] | undefined): void {
  if (typeof value === 'string' && value !== '' && value !== props.modelValue) emit('update:modelValue', value)
}

const SIZES = {
  sm: 'h-7 text-13 pointer-coarse:h-(--hit)',
  md: 'h-[calc(var(--hit)-6px)] text-14 pointer-coarse:h-(--hit)',
} as const

const itemClass = computed(() => [
  'inline-flex cursor-pointer items-center justify-center gap-1.5 rounded-sm whitespace-nowrap text-ink-2 transition-colors duration-(--dur-1) ease-standard not-disabled:hover:bg-bg-3 not-disabled:hover:text-ink focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50 aria-pressed:bg-accent-soft aria-pressed:font-medium aria-pressed:text-accent aria-pressed:hover:bg-accent-soft aria-pressed:hover:text-accent',
  SIZES[props.size],
  props.iconOnly ? 'aspect-square px-0' : 'px-2.5',
])
</script>

<template>
  <ToggleGroupRoot
    type="single"
    :model-value="modelValue"
    :disabled="disabled"
    :aria-label="label"
    class="inline-flex shrink-0 items-center gap-0.5 rounded-md border border-line-soft bg-bg-2 p-0.5 pointer-coarse:border-0 pointer-coarse:p-0"
    @update:model-value="onUpdate"
  >
    <template v-for="option in options" :key="option.value">
      <Tooltip v-if="iconOnly" :content="option.label" side="bottom">
        <ToggleGroupItem :value="option.value" :disabled="option.disabled" :aria-label="option.label" :data-testid="option.testid" :class="itemClass">
          <Icon v-if="option.icon" :name="option.icon" small />
        </ToggleGroupItem>
      </Tooltip>
      <ToggleGroupItem v-else :value="option.value" :disabled="option.disabled" :data-testid="option.testid" :class="itemClass">
        <Icon v-if="option.icon" :name="option.icon" small />
        <span>{{ option.label }}</span>
      </ToggleGroupItem>
    </template>
  </ToggleGroupRoot>
</template>
