<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{ card: string; busy: boolean; label: string }>()
const emit = defineEmits<{ format: [confirm: string] }>()

const WORD = 'FORMAT'
const typed = ref('')
const ready = computed(() => typed.value.trim() === WORD && props.card === 'ok' && !props.busy)
const reason = computed(() => {
  if (props.card === 'absent') return 'No memory card is in the camera.'
  if (props.card === 'error') return 'The card reports an error; the device must see a partition with space to format it.'
  if (typed.value.trim() !== WORD) return 'Type ' + WORD + ' to confirm. Formatting erases every recording on the card.'
  return ''
})

function run() {
  if (!ready.value) return
  emit('format', typed.value.trim())
  typed.value = ''
}
</script>

<template>
  <div class="row" data-testid="device-format">
    <div class="lab">
      <label for="device-format-confirm">Format the card</label>
      <span class="hint warn-text">{{ reason || 'Every recording on the card will be erased.' }}</span>
    </div>
    <div class="ctl-col">
      <div class="fmt">
        <input id="device-format-confirm" v-model="typed" type="text" autocomplete="off" :placeholder="WORD" :disabled="card !== 'ok'">
        <button class="btn danger" type="button" data-testid="device-format-run" :disabled="!ready" :aria-busy="busy" @click="run">{{ label }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fmt { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: var(--s2); min-width: 0; }
.fmt input { flex: 1 1 120px; width: 160px; min-width: 0; font-family: var(--f-mono); }
@media (max-width: 640px) {
  .fmt { justify-content: flex-start; }
  .fmt input { width: auto; }
}
</style>
