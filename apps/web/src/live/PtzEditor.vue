<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { addPreset, addTourStop, clearTour, deletePreset, removeTourStop, renamePreset } from '../app/controls'
import { live } from './store'

const NAME_MAX = 16
const LIMIT = 32
const pad = live.pad
const edit = pad.edit
const fresh = ref('')
const names = reactive<Record<string, string>>({})
const picks = reactive<Record<number, string>>({})
const tours = computed(() => (edit.lines.length ? edit.lines : [{ channel: 0, id: 0, name: '', points: [] as number[] }]))

watch(
  () => edit.presets.map((preset) => preset.token + '=' + preset.name).join('|'),
  () => {
    for (const key of Object.keys(names)) delete names[key]
    for (const preset of edit.presets) names[preset.token] = preset.name
  },
  { immediate: true },
)

function nextId() {
  const used = new Set(edit.presets.map((preset) => Number(preset.token)))
  let id = 1
  while (used.has(id)) id += 1
  return id
}

function presetName(id: number) {
  const found = edit.presets.find((preset) => Number(preset.token) === id)
  return found ? found.name : 'Preset' + id
}

async function save() {
  await addPreset(fresh.value)
  fresh.value = ''
}

function addStop(line: { id: number; points: number[] }) {
  const pick = picks[line.id] || (edit.presets[0] && edit.presets[0].token)
  if (pick) addTourStop(line.id, pick, line.points.length)
}
</script>

<template>
  <div id="ptz-editor" class="ptz-edit" data-testid="ptz-editor" role="group" aria-label="Edit presets and tours">
    <p v-if="!edit.loaded" class="ptz-edit-why" data-testid="ptz-editor-loading">Reading the presets and tours stored on the camera.</p>
    <template v-else>
      <p class="ptz-edit-h">Presets</p>
      <form class="ptz-edit-row" @submit.prevent="save">
        <input v-model="fresh" data-testid="preset-new-name" type="text" :maxlength="NAME_MAX" :placeholder="'Preset' + nextId()" aria-label="Name for the new preset" />
        <button class="pad-btn" data-testid="preset-add" type="submit" :disabled="pad.busy || edit.presets.length >= LIMIT" :title="edit.presets.length >= LIMIT ? 'Limit reached: the camera holds 32 presets.' : 'Saves where the moving lens points now.'">Save here</button>
      </form>
      <p v-if="edit.presets.length >= LIMIT" class="ptz-edit-why">Limit reached: the camera holds 32 presets.</p>
      <ul class="ptz-edit-list">
        <li v-for="preset in edit.presets" :key="preset.token" data-testid="preset-row" :data-token="preset.token">
          <input v-model="names[preset.token]" type="text" :maxlength="NAME_MAX" :aria-label="'Name of preset ' + preset.token" @keydown.enter.prevent="renamePreset(preset.token, names[preset.token] || '')" />
          <button class="pad-btn" data-testid="preset-rename" type="button" :disabled="pad.busy || (names[preset.token] || '').trim() === preset.name" @click="renamePreset(preset.token, names[preset.token] || '')">Rename</button>
          <button class="pad-btn" data-testid="preset-delete" type="button" :disabled="pad.busy" @click="deletePreset(preset.token, preset.name)">Delete</button>
        </li>
      </ul>
      <template v-for="line in tours" :key="line.id">
        <p class="ptz-edit-h">{{ edit.lines.length ? 'Tour ' + (line.name || line.id) : 'Tour (none yet)' }}</p>
        <ul class="ptz-edit-stops" data-testid="tour-stops" :data-tour="line.id">
          <li v-for="(stop, index) in line.points" :key="index + '-' + stop">
            <span>{{ presetName(stop) }}</span>
            <button class="pad-btn" data-testid="tour-stop-remove" type="button" :aria-label="'Remove ' + presetName(stop) + ' from the tour'" :disabled="pad.busy" @click="removeTourStop(line.id, stop)">Remove</button>
          </li>
          <li v-if="!line.points.length" class="ptz-edit-why">No stops yet. Add a preset to start the tour.</li>
        </ul>
        <div class="ptz-edit-row">
          <select v-model="picks[line.id]" data-testid="tour-pick" :aria-label="'Preset to add to tour ' + (line.name || line.id)" :disabled="!edit.presets.length">
            <option v-for="preset in edit.presets" :key="preset.token" :value="preset.token">{{ preset.name }}</option>
          </select>
          <button class="pad-btn" data-testid="tour-add" type="button" :disabled="pad.busy || !edit.presets.length" @click="addStop(line)">Add stop</button>
          <button class="pad-btn" data-testid="tour-clear" type="button" :disabled="pad.busy || !edit.lines.length" @click="clearTour(line.id)">Clear</button>
        </div>
        <p v-if="!edit.presets.length" class="ptz-edit-why">Add stop needs a preset: save one first.</p>
        <p v-if="!edit.lines.length" class="ptz-edit-why" data-testid="tour-clear-why">Clear is off: the camera stores no tour yet.</p>
      </template>
    </template>
  </div>
</template>
