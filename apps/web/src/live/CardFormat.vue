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
    <label class="lab" for="device-format-confirm">Format the card</label>
    <div class="ctl-col">
      <input id="device-format-confirm" v-model="typed" type="text" autocomplete="off" :placeholder="WORD" :disabled="card !== 'ok'">
      <p class="hint warn-text">{{ reason || 'Every recording on the card will be erased.' }}</p>
      <button class="btn" type="button" data-testid="device-format-run" :disabled="!ready" :aria-busy="busy" @click="run">{{ label }}</button>
    </div>
  </div>
</template>
