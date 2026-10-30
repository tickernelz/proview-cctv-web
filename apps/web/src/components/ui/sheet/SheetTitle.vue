<script setup lang="ts">
import { DialogTitle, type DialogTitleProps } from 'reka-ui'

const props = defineProps<DialogTitleProps>()
</script>

<template>
  <DialogTitle v-bind="props" class="text-15 font-semibold text-ink"><slot /></DialogTitle>
</template>
