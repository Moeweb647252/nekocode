import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueJsx from '@vitejs/plugin-vue-jsx'
import vueDevTools from 'vite-plugin-vue-devtools'
import Components from 'unplugin-vue-components/vite';
import {PrimeVueResolver} from '@primevue/auto-import-resolver';
import AutoImport from 'unplugin-auto-import/vite';
import UnoCSS from 'unocss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    UnoCSS(),

    vueJsx(),
    vueDevTools(),
    Components({
      resolvers: [PrimeVueResolver()],
      dts: 'src/interfaces/components.d.ts',
      directoryAsNamespace: true,
      dirs: ['src/components'],
    }),
    AutoImport({
      imports: ['vue'],
      dts: 'src/interfaces/auto-imports.d.ts',
    }),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    },
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8000',
        changeOrigin: true,
        secure: false,
        ws: true,
      },
    },
  }
})
