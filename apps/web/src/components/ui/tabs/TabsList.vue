<script setup lang="ts">
import { TabsList, type TabsListProps } from 'reka-ui'

const props = defineProps<TabsListProps>()
</script>

<template>
  <TabsList v-bind="props" class="tabs-list flex gap-1 rounded-md bg-bg-3 p-1">
    <slot />
  </TabsList>
</template>
