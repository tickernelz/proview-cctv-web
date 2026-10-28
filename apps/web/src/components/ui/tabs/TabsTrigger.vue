<script setup lang="ts">
import { TabsTrigger, type TabsTriggerProps } from 'reka-ui'

const props = defineProps<TabsTriggerProps>()
</script>

<template>
  <TabsTrigger v-bind="props" class="tabs-trigger">
    <slot />
  </TabsTrigger>
</template>
