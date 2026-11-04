import { resolve } from 'node:path'
import { existsSync, renameSync } from 'node:fs'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import Components from 'unplugin-vue-components/vite'
import { ElementPlusResolver } from 'unplugin-vue-components/resolvers'

export default defineConfig({
  base: './',
  publicDir: false,
  plugins: [
    vue(),
    Components({
      resolvers: [ElementPlusResolver()],
    }),
    {
      name: 'rename-liquid-gravity-probe-ui',
      closeBundle() {
        const output = resolve('integration/liquid-gravity-probe/ui-dist')
        const source = resolve(output, 'liquid-gravity-probe.html')
        if (existsSync(source)) renameSync(source, resolve(output, 'index.html'))
      },
    },
  ],
  build: {
    target: 'es2022',
    emptyOutDir: true,
    outDir: resolve('integration/liquid-gravity-probe/ui-dist'),
    assetsDir: 'assets',
    rollupOptions: {
      input: {
        index: resolve('liquid-gravity-probe.html'),
      },
    },
  },
})
