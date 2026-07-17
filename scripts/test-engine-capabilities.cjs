const assert = require('node:assert/strict')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import { findUnsupportedEngineFlags, getEngineCompatibilityMode, normalizeEngineCapabilityStatus, normalizeEngineVersionStatus } from './src/engineCapabilities'

  const detected = {
    status: 'detected',
    supportedFlags: ['-m', '--temp'],
    helpHash: 'abc',
    executableFingerprint: '1:2',
  }
  const command = ['llama-server', '-m', 'model.gguf', '--temp', '-1', '--future=value', '--future']
  assert.deepEqual(findUnsupportedEngineFlags(command, detected), ['--future'])
  assert.deepEqual(findUnsupportedEngineFlags(command, { ...detected, status: 'partial' }), [])
  assert.deepEqual(findUnsupportedEngineFlags(command, { ...detected, status: 'timeout' }), [])
  assert.equal(normalizeEngineCapabilityStatus(undefined), 'unprobed')
  assert.equal(normalizeEngineVersionStatus(undefined), 'unprobed')
  assert.equal(normalizeEngineVersionStatus({ ...detected, versionStatus: 'unknown' }), 'unknown')
  assert.equal(getEngineCompatibilityMode(detected), 'full')
  assert.equal(getEngineCompatibilityMode({ ...detected, status: 'partial' }), 'recognized')
  assert.equal(getEngineCompatibilityMode({ ...detected, status: 'failed' }), 'minimal')
  console.log('engine capability frontend regression tests passed')
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
    sourcefile: 'engine-capabilities.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'engine-capabilities.test.cjs'))
testModule.filename = path.join(process.cwd(), 'engine-capabilities.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)
