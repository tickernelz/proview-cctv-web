<script setup lang="ts">
import { TabsList, type TabsListProps } from 'reka-ui'

const props = defineProps<TabsListProps>()
</script>

<template>
  <TabsList v-bind="props" class="tabs-list">
    <slot />
  </TabsList>
</template>
