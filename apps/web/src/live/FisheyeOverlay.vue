<script setup lang="ts">
import { shallowRef, watch } from 'vue'
import { FisheyeDewarp, lensFromVideo, requestFromUrl, type FisheyeRequest } from './fisheye'

const request = shallowRef<FisheyeRequest | null>(requestFromUrl(location.search))
const canvas = shallowRef<HTMLCanvasElement | null>(null)
let frame = 0

function stop() {
  request.value = null
}

function attach(element: HTMLCanvasElement, wanted: FisheyeRequest) {
  const video = document.getElementById('video') as HTMLVideoElement
  let dewarp: FisheyeDewarp
  try {
    dewarp = new FisheyeDewarp(element)
  } catch (error) {
    console.error('fisheye overlay', error)
    stop()
    return
  }
  dewarp.setMount(wanted.mount)
  dewarp.setView(wanted.view)
  let lensSize = ''
  const tick = () => {
    const width = element.clientWidth || element.width
    const height = element.clientHeight || element.height
    if (element.width !== width || element.height !== height) {
      element.width = width
      element.height = height
    }
    if (video.readyState >= 2 && video.videoWidth > 0) {
      const size = video.videoWidth + 'x' + video.videoHeight
      if (size !== lensSize) {
        dewarp.setLens(lensFromVideo(video))
        lensSize = size
      }
      dewarp.uploadFrame(video)
      dewarp.render()
    }
    frame = requestAnimationFrame(tick)
  }
  frame = requestAnimationFrame(tick)
  Object.assign(window, { ProviewFisheyeLive: { dewarp, canvas: element, stop } })
}

watch(canvas, (element) => {
  cancelAnimationFrame(frame)
  if (element && request.value) attach(element, request.value)
})
</script>

<template>
  <Teleport v-if="request" to="#lens-crop">
    <canvas
      ref="canvas"
      data-testid="fisheye-overlay"
      :data-mode="request.view.mode"
      :data-mount="request.mount"
      class="pointer-events-none absolute inset-0 size-full"
    />
  </Teleport>
</template>
