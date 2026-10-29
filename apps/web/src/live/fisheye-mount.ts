import { createApp } from 'vue'
import FisheyeOverlay from './FisheyeOverlay.vue'
import { FisheyeDewarp, MODEL, webgl2Supported } from './fisheye'

Object.assign(window, {
  ProviewFisheye: { create: (canvas: HTMLCanvasElement) => new FisheyeDewarp(canvas), supported: webgl2Supported, model: MODEL },
})

const host = document.createElement('div')
document.body.append(host)
createApp(FisheyeOverlay).mount(host)
