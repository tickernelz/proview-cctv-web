<script setup lang="ts">
import { computed, useId } from 'vue'
import Icon from '../../Icon.vue'

const props = defineProps<{
  label: string
  hint?: string
  error?: string
  disabledReason?: string
  id?: string
  required?: boolean
}>()

defineSlots<{
  default(scope: { id: string; describedBy: string | undefined; invalid: boolean; required: boolean }): unknown
}>()

const fallbackId = useId()
const controlId = computed(() => props.id ?? fallbackId)
const hintId = computed(() => controlId.value + '-hint')
const reasonId = computed(() => controlId.value + '-reason')
const errorId = computed(() => controlId.value + '-error')
const describedBy = computed(() => {
  const ids = [props.hint ? hintId.value : '', props.disabledReason ? reasonId.value : '', props.error ? errorId.value : ''].filter(Boolean)
  return ids.length ? ids.join(' ') : undefined
})
</script>

<template>
  <div class="flex flex-col gap-1.5" :data-invalid="error ? '' : undefined">
    <label :for="controlId" class="text-13 font-medium text-ink-2">
      {{ label }}<span v-if="required" class="text-bad" aria-hidden="true"> *</span>
    </label>
    <slot :id="controlId" :described-by="describedBy" :invalid="Boolean(error)" :required="Boolean(required)" />
    <p v-if="hint" :id="hintId" class="text-13 text-pretty text-ink-3">{{ hint }}</p>
    <p v-if="disabledReason" :id="reasonId" class="flex items-start gap-1.5 text-13 text-pretty text-ink-3">
      <Icon name="info" small class="mt-px" /><span>{{ disabledReason }}</span>
    </p>
    <p v-if="error" :id="errorId" role="alert" class="flex items-start gap-1.5 text-13 text-pretty text-bad">
      <Icon name="warning" small class="mt-px" /><span>{{ error }}</span>
    </p>
  </div>
</template>
