const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { getActiveParams } from './src/components/ConfigPage/activeParams'
  import { normalizeInstanceConfig } from './src/modelPolicy'
  import { normalizeStoredConfig } from './src/store/bootstrap'
  import {
    applyExplicitOverrides,
    inheritParameters,
    markExplicitOverride,
    migrateParameterIntent,
  } from './src/parameterIntent'

  const defaults = defaultInstanceConfig()
  assert.deepEqual(defaults.explicit_overrides, [])
  assert.equal(defaults.launch_mode, 'managed')

  const legacyDefault = migrateParameterIntent({ ...defaults, explicit_overrides: null })
  assert.deepEqual(legacyDefault.explicit_overrides, [])

  const migrated = migrateParameterIntent({
    ...defaults,
    explicit_overrides: null,
    batch_size: 4096,
    spec_type: 'draft-mtp',
  })
  assert.deepEqual(new Set(migrated.explicit_overrides), new Set(['batch_size', 'spec_type']))

  const explicitDefault = markExplicitOverride(defaults, 'temp', defaults.temp)
  assert.deepEqual(explicitDefault.explicit_overrides, ['temp'])
  const emptyAlias = markExplicitOverride(
    { ...defaults, alias: 'chat', explicit_overrides: ['alias'] },
    'alias',
    '',
  )
  assert.deepEqual(emptyAlias.explicit_overrides, [])

  const inherited = inheritParameters(
    { ...defaults, temp: 0.6, top_k: 20, explicit_overrides: ['temp', 'top_k'] },
    ['temp'],
  )
  assert.equal(inherited.temp, defaults.temp)
  assert.deepEqual(inherited.explicit_overrides, ['top_k'])

  const preset = applyExplicitOverrides(defaults, { temp: defaults.temp, parallel: 1 })
  assert.deepEqual(new Set(preset.explicit_overrides), new Set(['temp', 'parallel']))

  const systemOnly = getActiveParams(defaults, false)
  assert.deepEqual(
    systemOnly,
    new Set(['model_path', 'host', 'port', 'metrics', 'props', 'slots_enabled']),
  )

  const compactSpec = {
    ...defaults,
    spec_type: 'draft-mtp',
    draft_tokens: 2,
    explicit_overrides: ['spec_type', 'draft_tokens'],
  }
  const compactActive = getActiveParams(compactSpec, false)
  assert.equal(compactActive.has('spec_type'), true)
  assert.equal(compactActive.has('draft_tokens'), true)
  assert.equal(compactActive.has('draft_gpu_layers'), false)
  assert.equal(compactActive.has('spec_draft_n_min'), false)
  assert.equal(compactActive.has('spec_draft_p_min'), false)
  assert.equal(compactActive.has('spec_draft_p_split'), false)

  const orphanedSpecChild = getActiveParams({
    ...compactSpec,
    explicit_overrides: ['spec_draft_n_min'],
  }, false)
  assert.equal(orphanedSpecChild.has('spec_draft_n_min'), false)

  const vector = normalizeInstanceConfig({
    ...defaults,
    embedding: true,
    temp: 0.4,
    explicit_overrides: ['embedding', 'temp', 'batch_size'],
  }, null)
  assert.equal(vector.config.explicit_overrides?.includes('temp'), false)
  assert.equal(vector.config.explicit_overrides?.includes('batch_size'), true)

  const freshVector = normalizeInstanceConfig({ ...defaults, embedding: true }, null)
  assert.equal(freshVector.config.batch_size, freshVector.config.ubatch_size)
  assert.equal(freshVector.config.explicit_overrides?.includes('batch_size'), true)

  const manual = {
    ...defaults,
    launch_mode: 'manual',
    manual_command: '-m model.gguf --port 9001',
  }
  assert.deepEqual(getActiveParams(manual, false), new Set(['manual_command']))
  assert.deepEqual(normalizeStoredConfig(manual, []).config, manual)

  console.log('parameter intent regression tests passed')
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
    sourcefile: 'parameter-intent.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'parameter-intent.test.cjs'))
testModule.filename = path.join(process.cwd(), 'parameter-intent.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)
