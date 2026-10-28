import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig, type Plugin } from 'vite'

const api = 'http://127.0.0.1:8787'

const desktopToken = (): Plugin => ({
  name: 'proview-desktop-token',
  apply: 'serve',
  async transformIndexHtml(html) {
    const page = await fetch(api + '/').then((r) => r.text()).catch(() => '')
    const bootstrap = page.match(/<script>window\.__PROVIEW_DESKTOP__[^<]*<\/script>/)
    return bootstrap ? html.replace('</head>', bootstrap[0] + '\n</head>') : html
  },
})

export default defineConfig({
  plugins: [vue(), tailwindcss(), desktopToken()],
  build: {
    target: ['chrome111', 'edge111', 'firefox128', 'safari16.4'],
    cssTarget: ['chrome111', 'edge111', 'firefox128', 'safari16.4'],
  },
  server: {
    cors: false,
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': { target: api, changeOrigin: true },
      '/media': { target: api, changeOrigin: true },
      '/ws': { target: api, changeOrigin: true, ws: true },
    },
  },
})
