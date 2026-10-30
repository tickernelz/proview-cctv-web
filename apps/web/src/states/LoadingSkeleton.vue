<script setup lang="ts">
import { computed } from 'vue'
import { Skeleton } from '../components/ui/skeleton'
import type { SkeletonKind } from './types'

const props = withDefaults(defineProps<{ kind?: SkeletonKind; count?: number; label: string }>(), { kind: 'rows', count: 3 })
const items = computed(() => Array.from({ length: Math.max(1, props.count) }, (_, index) => index))
</script>

<template>
  <div aria-busy="true" :data-kind="kind">
    <p class="sr-only" role="status">{{ label }}</p>
    <div v-if="kind === 'tiles'" class="grid grid-cols-1 gap-2 sm:grid-cols-2">
      <Skeleton v-for="item in items" :key="item" class="aspect-video" />
    </div>
    <div v-else-if="kind === 'form'" class="flex flex-col gap-4">
      <div v-for="item in items" :key="item" class="flex flex-col gap-2">
        <Skeleton class="h-3.5 w-32" />
        <Skeleton class="h-(--hit) w-full" />
      </div>
    </div>
    <ul v-else class="flex flex-col">
      <li v-for="item in items" :key="item" class="flex items-center gap-3 border-b border-line-soft py-2">
        <Skeleton class="h-[54px] w-24 shrink-0" />
        <div class="flex min-w-0 flex-1 flex-col gap-2">
          <Skeleton class="h-3.5 w-2/5" />
          <Skeleton class="h-3 w-3/5" />
        </div>
      </li>
    </ul>
  </div>
</template>
