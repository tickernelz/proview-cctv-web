<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../../Icon.vue'
import Tooltip from '../tooltip/Tooltip.vue'

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  label: string
  icon: string
  shortcut?: string
  variant?: 'ghost' | 'secondary'
  size?: 'sm' | 'md'
  pressed?: boolean
  disabled?: boolean
  side?: 'top' | 'right' | 'bottom' | 'left'
  type?: 'button' | 'submit' | 'reset'
}>(), { variant: 'ghost', size: 'md', pressed: undefined, side: 'top', type: 'button' })

const VARIANTS = {
  ghost: 'border-transparent bg-transparent text-ink-2 not-disabled:hover:bg-bg-3 not-disabled:hover:text-ink',
  secondary: 'border-line bg-bg-2 text-ink not-disabled:hover:bg-bg-3',
} as const

const SIZES = {
  sm: 'size-8 pointer-coarse:size-(--hit)',
  md: 'size-(--hit)',
} as const

const classes = computed(() => [
  'inline-flex shrink-0 cursor-pointer items-center justify-center rounded-md border transition-colors duration-(--dur-1) ease-standard focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50 aria-pressed:bg-accent-soft aria-pressed:text-accent',
  VARIANTS[props.variant],
  SIZES[props.size],
])
</script>

<template>
  <Tooltip :content="label" :shortcut="shortcut" :side="side" :disabled="disabled">
    <button
      v-bind="$attrs"
      :type="type"
      :class="classes"
      :aria-label="label"
      :aria-pressed="pressed === undefined ? undefined : pressed ? 'true' : 'false'"
      :aria-keyshortcuts="shortcut"
      :disabled="disabled"
    >
      <Icon :name="icon" :small="size === 'sm'" />
    </button>
  </Tooltip>
</template>
