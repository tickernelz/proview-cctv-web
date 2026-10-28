<script setup lang="ts">
defineProps<{ name: string; small?: boolean }>()
</script>

<template>
  <svg :class="['i', { 'i-16': small }]" aria-hidden="true"><use :href="'#i-' + name" /></svg>
</template>
