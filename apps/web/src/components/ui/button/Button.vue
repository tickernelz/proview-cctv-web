<script setup lang="ts">
import { Primitive, type PrimitiveProps } from 'reka-ui'
import { computed } from 'vue'
import Icon from '../../Icon.vue'
import type { ButtonSize, ButtonVariant } from './types'

const props = withDefaults(defineProps<PrimitiveProps & {
  variant?: ButtonVariant
  size?: ButtonSize
  icon?: string
  iconEnd?: string
  loading?: boolean
  block?: boolean
  disabled?: boolean
  type?: 'button' | 'submit' | 'reset'
}>(), { as: 'button', variant: 'secondary', size: 'md', type: 'button' })

const VARIANTS: Record<ButtonVariant, string> = {
  primary: 'border-transparent bg-accent text-accent-ink not-disabled:hover:brightness-95',
  secondary: 'border-line bg-bg-2 text-ink not-disabled:hover:bg-bg-3',
  ghost: 'border-transparent bg-transparent text-ink-2 not-disabled:hover:bg-bg-3 not-disabled:hover:text-ink',
  danger: 'border-line bg-bg-2 text-bad not-disabled:hover:border-bad not-disabled:hover:bg-bg-3',
}

const SIZES: Record<ButtonSize, string> = {
  sm: 'h-8 gap-1.5 px-3 text-13 pointer-coarse:h-(--hit)',
  md: 'h-(--hit) gap-2 px-3.5 text-14',
  lg: 'h-11 gap-2 px-4 text-15',
}

const isButton = computed(() => props.as === 'button' && !props.asChild)
const classes = computed(() => [
  'relative shrink-0 items-center justify-center whitespace-nowrap rounded-md border font-medium select-none transition-[color,background-color,border-color,filter] duration-(--dur-1) ease-standard focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus',
  VARIANTS[props.variant],
  SIZES[props.size],
  props.block ? 'flex w-full' : 'inline-flex',
  props.disabled ? 'cursor-not-allowed opacity-50' : props.loading ? 'cursor-progress' : 'cursor-pointer',
])
</script>

<template>
  <Primitive
    :as="as"
    :as-child="asChild"
    :type="isButton ? type : undefined"
    :disabled="isButton ? disabled || loading : undefined"
    :aria-disabled="!isButton && disabled ? 'true' : undefined"
    :aria-busy="loading ? 'true' : undefined"
    :data-variant="variant"
    :class="classes"
  >
    <slot v-if="asChild" />
    <template v-else>
      <span v-if="loading" class="absolute inset-0 grid place-items-center" aria-hidden="true">
        <Icon name="loader" small class="motion-safe:animate-spin" />
      </span>
      <span :class="['inline-flex min-w-0 items-center justify-center gap-[inherit]', loading ? 'invisible' : '']">
        <Icon v-if="icon" :name="icon" :small="size !== 'lg'" />
        <slot />
        <Icon v-if="iconEnd" :name="iconEnd" :small="size !== 'lg'" />
      </span>
    </template>
  </Primitive>
</template>
