import { reactive } from 'vue'

export const pip = reactive({
  shown: false,
  swapped: false,
  main: '',
  inset: '',
  crop: { x: 0, y: 0.5, w: 1, h: 0.5 },
  ratio: 16 / 9,
})

export const frameFit = reactive({ height: '' })
