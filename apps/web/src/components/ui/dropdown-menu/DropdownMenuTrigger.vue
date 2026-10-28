<script setup lang="ts">
import { DropdownMenuTrigger, injectDropdownMenuRootContext, type DropdownMenuTriggerProps } from 'reka-ui'

const props = defineProps<DropdownMenuTriggerProps & { id?: string }>()
const root = injectDropdownMenuRootContext()
if (props.id) root.triggerId = props.id
</script>

<template>
  <DropdownMenuTrigger :as="props.as" :as-child="props.asChild" :disabled="props.disabled">
    <slot />
  </DropdownMenuTrigger>
</template>
