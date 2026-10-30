<script setup lang="ts">
import { reactive, watch } from 'vue'

export type RecordState = {
  mode?: string | null
  full_time?: boolean | null
  packet_length?: number | null
  pre_record?: number | null
  schedule?: string[]
  record_stream?: string | null
  record_stream_supported?: boolean
  splicing?: boolean | null
  splicing_supported?: boolean
  audio?: boolean | null
}

const props = defineProps<{ record: RecordState; busy: boolean; label: string }>()
const emit = defineEmits<{ save: [patch: Record<string, string>] }>()

const PACKET_MIN = 5
const PACKET_MAX = 120

const form = reactive({ full_time: false, packet_length: '' as number | string, record_stream: 'main', splicing: false, audio: false })

function fill() {
  form.full_time = Boolean(props.record.full_time)
  form.packet_length = props.record.packet_length ?? ''
  form.record_stream = props.record.record_stream === 'extra' ? 'extra' : 'main'
  form.splicing = Boolean(props.record.splicing)
  form.audio = Boolean(props.record.audio)
}

watch(() => props.record, fill, { immediate: true })

function save() {
  const record = props.record
  const patch: Record<string, string> = {}
  if (record.full_time !== null && record.full_time !== undefined && form.full_time !== record.full_time) patch.full_time = String(form.full_time)
  if (form.packet_length !== '') {
    const minutes = Math.min(PACKET_MAX, Math.max(PACKET_MIN, Math.round(Number(form.packet_length)) || PACKET_MIN))
    form.packet_length = minutes
    if (minutes !== record.packet_length) patch.packet_length = String(minutes)
  }
  if (record.record_stream_supported && form.record_stream !== (record.record_stream || 'main')) patch.record_stream = form.record_stream
  if (record.splicing_supported && form.splicing !== Boolean(record.splicing)) patch.splicing = String(form.splicing)
  if (record.audio !== null && record.audio !== undefined && form.audio !== record.audio) patch.audio = String(form.audio)
  emit('save', patch)
}
</script>

<template>
  <div id="device-record-block" class="set-card" data-testid="device-record-block">
    <div class="set-card-h">
      <div>
        <h3 class="set-card-t">Recording</h3>
        <p class="set-card-d">When the camera records, how long each file runs and which stream it keeps.</p>
      </div>
    </div>
    <div class="row">
      <div class="lab">
        Full-time recording
        <span class="hint">Off records on alarms only. Writes the Record schedule for every day, as the mobile app does.</span>
        <span v-if="record.full_time === null || record.full_time === undefined" class="hint">The camera reports no RecordMode, so the schedule cannot be read.</span>
        <span v-else class="hint">Now: {{ record.mode || 'unknown' }}, {{ (record.schedule && record.schedule[0]) || 'no time section' }}</span>
      </div>
      <div class="ctl-col">
        <label class="switchrow"><input id="device-full-time" v-model="form.full_time" type="checkbox" role="switch" :disabled="record.full_time === null || record.full_time === undefined"><span>Record around the clock</span></label>
      </div>
    </div>
    <div class="row">
      <div class="lab">
        <label for="device-packet-length">Clip length (minutes)</label>
        <span class="hint">{{ PACKET_MIN }} to {{ PACKET_MAX }} minutes per recorded file.</span>
      </div>
      <div class="ctl-col">
        <input id="device-packet-length" v-model="form.packet_length" type="number" step="1" :min="PACKET_MIN" :max="PACKET_MAX">
      </div>
    </div>
    <div class="row">
      <div class="lab">Pre-record</div>
      <div class="ctl-col"><p class="val"><span class="code">{{ record.pre_record ?? 'not reported' }}</span> <span class="hint">seconds, read only: the mobile app does not change it.</span></p></div>
    </div>
    <div class="row">
      <div class="lab">
        <label for="device-record-stream">Recorded stream</label>
        <span v-if="!record.record_stream_supported" class="hint">The camera does not report SupportRecMainOrExtUseMainType, so it always records the main stream.</span>
      </div>
      <div class="ctl-col">
        <select id="device-record-stream" v-model="form.record_stream" :disabled="!record.record_stream_supported">
          <option value="main">Main stream (HD)</option>
          <option value="extra">Extra stream (low HD)</option>
        </select>
      </div>
    </div>
    <div class="row">
      <div class="lab">
        Lens splicing
        <span v-if="!record.splicing_supported" class="hint">The camera does not report SupportMultiLensSplicingWfsRecordStream.</span>
      </div>
      <div class="ctl-col">
        <label class="switchrow"><input id="device-splicing" v-model="form.splicing" type="checkbox" role="switch" :disabled="!record.splicing_supported"><span>Record both lenses as one spliced picture</span></label>
      </div>
    </div>
    <div class="row">
      <div class="lab">
        Record audio
        <span v-if="record.audio === null || record.audio === undefined" class="hint">The encoder block carries no AudioEnable field.</span>
      </div>
      <div class="ctl-col">
        <label class="switchrow"><input id="device-record-audio" v-model="form.audio" type="checkbox" role="switch" :disabled="record.audio === null || record.audio === undefined"><span>Keep sound in recordings</span></label>
      </div>
    </div>
    <div class="set-card-f">
      <button class="btn btn-fill" type="button" data-testid="device-save-record" :aria-busy="busy" @click="save">{{ label }}</button>
    </div>
  </div>
</template>
