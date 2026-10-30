<script setup lang="ts">
import { computed, onBeforeUnmount, shallowRef, watch } from 'vue'
import Icon from '../../components/Icon.vue'
import { swapLenses } from '../../app/lens'
import { live } from '../store'
import { pip } from './pip'

const canvas = shallowRef<HTMLCanvasElement | null>(null)
const label = computed(() => 'Swap lenses: show the ' + pip.inset.toLowerCase() + ' large')
let video: HTMLVideoElement | null = null
let animationHandle = 0
let drawn = ''

function draw() {
  animationHandle = 0
  const target = canvas.value
  if (!target || !video) return
  const scale = Math.min(window.devicePixelRatio || 1, 2)
  const width = Math.max(1, Math.round(target.clientWidth * scale))
  const height = Math.max(1, Math.round(target.clientHeight * scale))
  if (target.width !== width || target.height !== height) {
    target.width = width
    target.height = height
    drawn = ''
  }
  const context = target.getContext('2d')
  const key = video.currentTime + ':' + video.videoWidth
  if (context && video.readyState >= 2 && video.videoWidth > 0 && key !== drawn) {
    drawn = key
    const crop = pip.crop
    context.save()
    if (live.stage.crop.flipped === 'true') {
      context.translate(width, height)
      context.rotate(Math.PI)
    }
    context.drawImage(video, crop.x * video.videoWidth, crop.y * video.videoHeight, crop.w * video.videoWidth, crop.h * video.videoHeight, 0, 0, width, height)
    context.restore()
  }
  schedule()
}

function schedule() {
  if (video && canvas.value) animationHandle = requestAnimationFrame(draw)
}

function stop() {
  if (animationHandle) cancelAnimationFrame(animationHandle)
  animationHandle = 0
  drawn = ''
}

watch(
  [canvas, () => pip.crop, () => live.stage.crop.flipped, () => live.stage.crop.width],
  () => {
    stop()
    video = document.getElementById('video') as HTMLVideoElement | null
    if (canvas.value) draw()
  },
  { flush: 'post' },
)

onBeforeUnmount(stop)
</script>

<template>
  <button v-if="pip.shown" type="button" class="pip" data-testid="pip-inset" :aria-label="label" :style="{ aspectRatio: String(pip.ratio) }" @click.stop="swapLenses">
    <canvas ref="canvas" class="pip-canvas" aria-hidden="true" />
    <span class="pip-label" data-testid="pip-label">{{ pip.inset }}</span>
    <span class="pip-swap" aria-hidden="true"><Icon name="swap" small /></span>
  </button>
</template>
