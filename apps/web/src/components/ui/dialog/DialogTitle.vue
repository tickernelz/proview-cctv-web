<script setup lang="ts">
import { DialogTitle, type DialogTitleProps } from 'reka-ui'

const props = defineProps<DialogTitleProps>()
</script>

<template>
  <DialogTitle v-bind="props" class="text-17 font-semibold text-balance text-ink"><slot /></DialogTitle>
</template>
