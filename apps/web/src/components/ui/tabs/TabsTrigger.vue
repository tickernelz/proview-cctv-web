<script setup lang="ts">
import { TabsTrigger, type TabsTriggerProps } from 'reka-ui'

const props = defineProps<TabsTriggerProps>()
</script>

<template>
  <TabsTrigger
    v-bind="props"
    class="tabs-trigger inline-flex min-h-[max(36px,var(--hit))] flex-1 cursor-pointer items-center justify-center gap-1.5 rounded-sm px-2 text-14 whitespace-nowrap text-ink-2 transition-colors duration-(--dur-1) ease-standard not-disabled:hover:text-ink focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-offset-2 focus-visible:outline-focus disabled:cursor-not-allowed disabled:opacity-50 data-[state=active]:bg-bg-2 data-[state=active]:text-ink data-[state=active]:shadow-[inset_0_0_0_1px_var(--line)]"
  >
    <slot />
  </TabsTrigger>
</template>
