<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import { pictureMissing, srcOf, type Alarm } from './events'

const props = defineProps<{ entry: Alarm; large?: boolean }>()
const src = computed(() => srcOf(props.entry))
</script>

<template>
  <img v-if="src" :class="['thumb', 'ev-thumb', { 'ev-thumb-lg': large }]" alt="" loading="lazy" :src="src" @error="pictureMissing(entry)">
  <span v-else :class="['thumb', 'ev-thumb', 'ev-thumb-none', { 'ev-thumb-lg': large }]"><Icon name="no-picture" :small="!large" /></span>
</template>
