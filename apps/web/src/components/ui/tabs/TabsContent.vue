<script setup lang="ts">
import { TabsContent, type TabsContentProps } from 'reka-ui'

const props = defineProps<TabsContentProps>()
</script>

<template>
  <TabsContent v-bind="props" class="tabs-content">
    <slot />
  </TabsContent>
</template>
