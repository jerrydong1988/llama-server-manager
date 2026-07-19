import { defineConfig, mergeConfig, type Plugin, type UserConfig } from 'vite'
import productionConfig from './vite.config'

const browserTestMockPlugin: Plugin = {
  name: 'llama-manager-browser-test-tauri-mock',
  apply: 'serve',
  enforce: 'pre',
  transformIndexHtml: {
    order: 'pre',
    handler: () => [{
      tag: 'script',
      attrs: { type: 'module', src: '/browser-tests/tauriMock.ts' },
      injectTo: 'head-prepend',
    }],
  },
}

export default defineConfig(({ command }) => {
  if (command !== 'serve') {
    throw new Error('The browser-test Tauri mock is serve-only and cannot build production artifacts.')
  }

  return mergeConfig(productionConfig as UserConfig, {
    plugins: [browserTestMockPlugin],
    server: {
      host: '127.0.0.1',
      port: 4173,
      strictPort: true,
    },
  })
})
