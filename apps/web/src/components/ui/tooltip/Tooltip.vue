<script setup lang="ts">
import { TooltipContent, TooltipPortal, TooltipProvider, TooltipRoot, TooltipTrigger, injectTooltipProviderContext } from 'reka-ui'
import Kbd from '../kbd/Kbd.vue'

withDefaults(defineProps<{
  content: string
  shortcut?: string
  side?: 'top' | 'right' | 'bottom' | 'left'
  disabled?: boolean
}>(), { side: 'top' })

const provided = injectTooltipProviderContext(null) !== null
const CONTENT = 'z-60 flex max-w-[280px] items-center gap-2 rounded-md border border-line bg-bg-2 px-2 py-1 text-12 text-ink shadow-menu transition-opacity duration-(--dur-1) ease-standard starting:opacity-0'
</script>

<template>
  <TooltipRoot v-if="provided" :disabled="disabled">
    <TooltipTrigger as-child><slot /></TooltipTrigger>
    <TooltipPortal>
      <TooltipContent :side="side" :side-offset="6" :collision-padding="8" :class="CONTENT">
        <span>{{ content }}</span><Kbd v-if="shortcut">{{ shortcut }}</Kbd>
      </TooltipContent>
    </TooltipPortal>
  </TooltipRoot>
  <TooltipProvider v-else :delay-duration="400" :skip-delay-duration="300">
    <TooltipRoot :disabled="disabled">
      <TooltipTrigger as-child><slot /></TooltipTrigger>
      <TooltipPortal>
        <TooltipContent :side="side" :side-offset="6" :collision-padding="8" :class="CONTENT">
          <span>{{ content }}</span><Kbd v-if="shortcut">{{ shortcut }}</Kbd>
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>
