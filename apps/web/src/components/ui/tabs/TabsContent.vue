<script setup lang="ts">
import { TabsContent, type TabsContentProps } from 'reka-ui'

const props = defineProps<TabsContentProps>()
</script>

<template>
  <TabsContent v-bind="props" class="tabs-content rounded-md focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus">
    <slot />
  </TabsContent>
</template>
