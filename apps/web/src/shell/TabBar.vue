<script setup lang="ts">
import { computed } from 'vue'
import Icon from '../components/Icon.vue'
import { live } from '../live/store'
import { shell } from './layout'
import { EVENTS_HASH, liveHash, settingsHash } from './router'

const tabs = computed(() => [
  { id: 'live', label: 'Live', icon: 'cctv', href: liveHash(shell.mode) },
  { id: 'events', label: 'Events', icon: 'bell', href: EVENTS_HASH },
  { id: 'settings', label: 'Settings', icon: 'gear', href: settingsHash(live.settings.section) },
])
</script>

<template>
  <nav class="tabbar" data-testid="tab-bar" aria-label="Main">
    <a
      v-for="tab in tabs"
      :key="tab.id"
      :href="tab.href"
      class="tabbar-item"
      :data-testid="'tab-' + tab.id"
      :aria-current="shell.view === tab.id ? 'page' : undefined"
    >
      <Icon :name="tab.icon" />
      <span>{{ tab.label }}</span>
    </a>
  </nav>
</template>
