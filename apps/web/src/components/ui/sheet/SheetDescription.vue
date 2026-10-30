<script setup lang="ts">
import { DialogDescription, type DialogDescriptionProps } from 'reka-ui'

const props = defineProps<DialogDescriptionProps>()
</script>

<template>
  <DialogDescription v-bind="props" class="text-13 text-pretty text-ink-2"><slot /></DialogDescription>
</template>
