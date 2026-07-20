const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const sharedControlsSource = fs.readFileSync(path.join(process.cwd(), 'src', 'components', 'ConfigPage', 'shared.tsx'), 'utf8')
const launchModeSource = fs.readFileSync(path.join(process.cwd(), 'src', 'components', 'ConfigPage', 'LaunchModePanel.tsx'), 'utf8')
assert.doesNotMatch(sharedControlsSource, /active \? 'text-emerald/, 'control colors must describe values, not internal emission metadata')
assert.doesNotMatch(launchModeSource, /overrideKeys\.map/, 'managed mode must not expose every internal override as a primary UI chip')

const entry = `
  import assert from 'node:assert/strict'
  import { defaultInstanceConfig } from './src/store/defaults'
  import { getActiveParams } from './src/components/ConfigPage/activeParams'
  import { canonicalConfigFields, reviewFieldKeys } from './src/components/ConfigPage/configWorkspace'
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
  const changedThenRestored = markExplicitOverride(
    markExplicitOverride(defaults, 'temp', 0.6),
    'temp',
    defaults.temp,
  )
  assert.deepEqual(changedThenRestored.explicit_overrides, ['temp'])
  const disabledAutoload = markExplicitOverride(defaults, 'models_autoload', false)
  assert.deepEqual(disabledAutoload.explicit_overrides, ['models_autoload'])
  assert.deepEqual(markExplicitOverride(disabledAutoload, 'models_autoload', true).explicit_overrides, ['models_autoload'])
  const emptyAlias = markExplicitOverride(
    { ...defaults, alias: 'chat', explicit_overrides: ['alias'] },
    'alias',
    '',
  )
  assert.deepEqual(emptyAlias.explicit_overrides, ['alias'])

  const inherited = inheritParameters(
    { ...defaults, temp: 0.6, top_k: 20, explicit_overrides: ['temp', 'top_k'] },
    ['temp'],
  )
  assert.equal(inherited.temp, defaults.temp)
  assert.deepEqual(inherited.explicit_overrides, ['top_k'])

  const equalValueInherited = inheritParameters(explicitDefault, ['temp'])
  assert.equal(equalValueInherited.temp, defaults.temp)
  assert.deepEqual(equalValueInherited.explicit_overrides, [])

  const automaticAliases = canonicalConfigFields([
    'gpu_layers', 'gpu_layers_auto', 'ctx_size', 'ctx_size_auto',
  ])
  assert.deepEqual(automaticAliases, ['gpu_layers', 'ctx_size'])
  const automaticInheritKeys = [...new Set(automaticAliases.flatMap(reviewFieldKeys))]
  const automaticInherited = inheritParameters({
    ...defaults,
    gpu_layers: 7,
    gpu_layers_auto: false,
    ctx_size: 4096,
    ctx_size_auto: false,
    explicit_overrides: ['gpu_layers', 'gpu_layers_auto', 'ctx_size', 'ctx_size_auto'],
  }, automaticInheritKeys)
  assert.equal(automaticInherited.gpu_layers, defaults.gpu_layers)
  assert.equal(automaticInherited.gpu_layers_auto, defaults.gpu_layers_auto)
  assert.equal(automaticInherited.ctx_size, defaults.ctx_size)
  assert.equal(automaticInherited.ctx_size_auto, defaults.ctx_size_auto)
  assert.deepEqual(automaticInherited.explicit_overrides, [])

  const preset = applyExplicitOverrides(defaults, { temp: defaults.temp, parallel: 1 })
  assert.deepEqual(preset.explicit_overrides, ['temp', 'parallel'])
  const compactedTracked = migrateParameterIntent({
    ...defaults,
    explicit_overrides: ['temp', 'models_autoload'],
  })
  assert.deepEqual(compactedTracked.explicit_overrides, ['temp', 'models_autoload'])

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
    custom_args: ['--ubatch-size', '256'],
    explicit_overrides: ['embedding', 'temp', 'batch_size', 'custom_args'],
  }, null)
  assert.equal(vector.config.explicit_overrides?.includes('temp'), false)
  assert.equal(vector.config.explicit_overrides?.includes('batch_size'), true)
  assert.equal(vector.config.explicit_overrides?.includes('custom_args'), true)
  assert.deepEqual(vector.config.custom_args, ['--ubatch-size', '256'])

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
