const esbuild = require('esbuild')
const Module = require('node:module')
const path = require('node:path')
const entry = `
import assert from 'node:assert/strict'
import { defaultInstanceConfig } from './src/store/defaults'
import { configForEngine } from './src/gufo'
import { resolveEffectiveEngine } from './src/store/engineResolution'
import { normalizeInstanceConfig } from './src/modelPolicy'
const llama = { id: 'llama', kind: 'llama_cpp' }
const gufo = { id: 'gufo', kind: 'gufo' }
const legacy = defaultInstanceConfig()
assert.equal(resolveEffectiveEngine(legacy, [gufo, llama], 'gufo'), llama)
assert.equal(resolveEffectiveEngine(legacy, [gufo], 'gufo'), null)
assert.equal(resolveEffectiveEngine({ engine_id: '' }, [gufo, llama], ''), llama)
legacy.explicit_overrides = ['gpu_layers', 'spec_type']
legacy.rpc_servers = 'remote:5000'
legacy.kv_checkpoint.enabled = true
const selected = configForEngine(legacy, gufo)
assert.equal(selected.engine_kind, 'gufo')
assert.equal(selected.engine_id, 'gufo')
assert.equal(selected.rpc_servers, '')
assert.equal(selected.kv_checkpoint.enabled, false)
assert.deepEqual(selected.explicit_overrides, [])
assert.equal(selected.gufo.sessions, 1)
assert.equal(resolveEffectiveEngine(selected, [llama], 'llama'), null)
assert.deepEqual(normalizeInstanceConfig(selected).config, selected)
assert.equal(configForEngine(selected, llama).engine_kind, 'llama_cpp')
assert.equal(legacy.kv_checkpoint.enabled, true)
console.log('Gufo engine selection and configuration isolation passed')
`
const built = esbuild.buildSync({ bundle: true, format: 'cjs', platform: 'node', packages: 'external', write: false,
  stdin: { contents: entry, resolveDir: process.cwd(), sourcefile: 'gufo.test.ts', loader: 'ts' } })
const test = new Module(path.join(process.cwd(), 'gufo.test.cjs'))
test.filename = path.join(process.cwd(), 'gufo.test.cjs')
test.paths = Module._nodeModulePaths(process.cwd())
test._compile(built.outputFiles[0].text, test.filename)
