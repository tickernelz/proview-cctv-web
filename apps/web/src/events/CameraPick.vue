<script setup lang="ts">
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../components/ui/select'
import { cameras, ev } from './events'

function pick(value: unknown) {
  if (typeof value === 'string' && value) ev.query.serial = value
}
</script>

<template>
  <Select :model-value="ev.query.serial || undefined" :disabled="!cameras.length" @update:model-value="pick">
    <SelectTrigger data-device data-testid="alarm-camera" :value="ev.query.serial" aria-label="Camera" class="ev-camera">
      <SelectValue placeholder="No camera" />
    </SelectTrigger>
    <SelectContent>
      <SelectItem v-for="device in cameras" :key="device.id" :value="device.id" data-testid="alarm-camera-option">{{ device.name || device.id }}</SelectItem>
    </SelectContent>
  </Select>
</template>
