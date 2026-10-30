<script setup lang="ts">
import { Primitive, type PrimitiveProps } from 'reka-ui'

withDefaults(defineProps<PrimitiveProps>(), { as: 'h3' })
</script>

<template>
  <Primitive :as="as" :as-child="asChild" class="text-15 font-semibold text-ink"><slot /></Primitive>
</template>
