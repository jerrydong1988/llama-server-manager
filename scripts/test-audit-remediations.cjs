const assert = require('node:assert/strict')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import { maskStartupCommandSecrets } from './src/store/commandFormatting'
  import { forEachConcurrent } from './src/utils/async'

  async function run() {
    assert.equal(
      maskStartupCommandSecrets('llama-server --api-key secret-value --port 8080'),
      'llama-server --api-key ******** --port 8080',
    )
    assert.equal(
      maskStartupCommandSecrets('llama-server --api-key="secret value" --api-key-file key.txt'),
      'llama-server --api-key=******** --api-key-file key.txt',
    )

    let active = 0
    let peak = 0
    const processed = []
    await forEachConcurrent([1, 2, 3, 4, 5, 6, 7], 3, async value => {
      active += 1
      peak = Math.max(peak, active)
      await new Promise(resolve => setTimeout(resolve, 5))
      processed.push(value)
      active -= 1
    })
    assert.equal(peak, 3)
    assert.deepEqual(processed.sort((left, right) => left - right), [1, 2, 3, 4, 5, 6, 7])
  }

  run()
    .then(() => console.log('audit remediation regression tests passed'))
    .catch(error => {
      console.error(error)
      process.exitCode = 1
    })
`

const bundled = esbuild.buildSync({
  bundle: true,
  format: 'cjs',
  platform: 'node',
  packages: 'external',
  write: false,
  stdin: {
    contents: entry,
    resolveDir: process.cwd(),
    sourcefile: 'audit-remediations.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'audit-remediations.test.cjs'))
testModule.filename = path.join(process.cwd(), 'audit-remediations.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)
