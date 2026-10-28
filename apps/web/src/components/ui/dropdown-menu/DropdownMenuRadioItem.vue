<script setup lang="ts">
import { DropdownMenuItemIndicator, DropdownMenuRadioItem } from 'reka-ui'
import Icon from '../../Icon.vue'

defineProps<{ value: string; name: string; sub: string }>()
</script>

<template>
  <DropdownMenuRadioItem :value="value" class="menu-row">
    <span class="menu-check"><DropdownMenuItemIndicator><Icon name="check" small /></DropdownMenuItemIndicator></span>
    <span class="col"><span class="name">{{ name }}</span><span class="sub">{{ sub }}</span></span>
  </DropdownMenuRadioItem>
</template>
