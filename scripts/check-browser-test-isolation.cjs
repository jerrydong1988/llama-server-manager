const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const read = relative => fs.readFileSync(path.join(root, relative), 'utf8')
const markers = ['@tauri-apps/api/mocks', '__TAURI_BROWSER_TEST__', '__LLAMA_MANAGER_BROWSER_TEST_BACKEND__', 'browser-tests/tauriMock']

const productionConfig = read('vite.config.ts')
const productionMain = read('src/main.tsx')
const browserConfig = read('vite.browser-test.config.ts')
const mockSource = read('browser-tests/tauriMock.ts')

for (const marker of markers) {
  assert.doesNotMatch(productionConfig, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), `production Vite config must not reference ${marker}`)
  assert.doesNotMatch(productionMain, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), `production entry must not reference ${marker}`)
}

assert.match(browserConfig, /apply: 'serve'/, 'browser-test mock plugin must be serve-only')
assert.match(browserConfig, /command !== 'serve'/, 'browser-test config must reject build commands')
assert.match(browserConfig, /\/browser-tests\/tauriMock\.ts/, 'browser-test config must inject the isolated mock entry')
assert.match(mockSource, /mockIPC/, 'browser-test backend must use the official Tauri IPC mock')
assert.match(mockSource, /shouldMockEvents: true/, 'browser-test backend must support Tauri event listeners')
assert.match(mockSource, /Unhandled browser-test Tauri command/, 'unhandled IPC must fail visibly')
assert.match(mockSource, /dataset\.tauriMockUnhandled/, 'browser-test backend must expose an automation health probe')

if (process.argv.includes('--dist')) {
  const dist = path.join(root, 'dist')
  assert.ok(fs.existsSync(dist), 'dist must exist before production artifact isolation checking')
  const files = []
  const visit = directory => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const target = path.join(directory, entry.name)
      if (entry.isDirectory()) visit(target)
      else if (/\.(?:html|js|css|map)$/i.test(entry.name)) files.push(target)
    }
  }
  visit(dist)
  const artifact = files.map(file => fs.readFileSync(file, 'utf8')).join('\n')
  for (const marker of markers) {
    assert.ok(!artifact.includes(marker), `production artifact contains browser-test marker: ${marker}`)
  }
}

console.log(`Browser-test Tauri mock isolation passed${process.argv.includes('--dist') ? ' (including dist)' : ''}.`)
