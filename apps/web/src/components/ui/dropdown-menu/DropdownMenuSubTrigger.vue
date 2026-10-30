<script setup lang="ts">
import { DropdownMenuSubTrigger, type DropdownMenuSubTriggerProps } from 'reka-ui'
import Icon from '../../Icon.vue'

const props = defineProps<DropdownMenuSubTriggerProps & { icon?: string }>()
</script>

<template>
  <DropdownMenuSubTrigger :as="props.as" :as-child="props.asChild" :disabled="props.disabled" :text-value="props.textValue" class="menu-row data-disabled:cursor-not-allowed data-disabled:opacity-50">
    <Icon v-if="props.icon" :name="props.icon" small />
    <span class="name min-w-0 flex-1 truncate"><slot /></span>
    <Icon name="chevron-right" small class="ml-auto text-ink-3" />
  </DropdownMenuSubTrigger>
</template>
