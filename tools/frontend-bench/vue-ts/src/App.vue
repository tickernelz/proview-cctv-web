<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { type Device, loadDevices } from './api';

const devices = ref<Device[]>([]);
const selected = ref<string | null>(null);
const online = computed(() => devices.value.filter((d) => d.online).length);
const title = computed(() => devices.value.find((d) => d.id === selected.value)?.name ?? 'No device');

async function refresh() {
  devices.value = await loadDevices();
}

onMounted(refresh);
</script>

<template>
  <main>
    <nav class="rail">
      <p>{{ online }} of {{ devices.length }} online</p>
      <button @click="refresh">Refresh</button>
      <button v-for="d in devices" :key="d.id" :aria-pressed="d.id === selected" @click="selected = d.id">{{ d.name }}</button>
    </nav>
    <video muted :title="title"></video>
  </main>
</template>
