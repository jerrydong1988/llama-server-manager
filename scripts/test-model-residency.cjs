const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import { buildResidencyPolicy, orderedResidencyOperations, RESIDENCY_GIB } from './src/components/ClusterPage/residencyReconcile'

  const policy = buildResidencyPolicy({
    enabled: true,
    ramGiB: 24,
    vramGiB: 12,
    drainTimeoutSeconds: 1,
    instanceIds: ['later', 'first'],
    intents: {
      later: { instanceId: 'later', priority: 20, enabled: true },
      first: { instanceId: 'first', priority: 10, enabled: true },
    },
  })
  assert.equal(policy.ramBudgetBytes, 24 * RESIDENCY_GIB)
  assert.equal(policy.vramBudgetBytes, 12 * RESIDENCY_GIB)
  assert.equal(policy.drainTimeoutSeconds, 5)
  assert.deepEqual(policy.intents.map(intent => intent.instanceId), ['first', 'later'])

  const operations = orderedResidencyOperations([
    { sequence: 5, kind: 'warm', instanceId: 'first', deploymentId: 'd', revisionId: 'new', reason: 'replace' },
    { sequence: 2, kind: 'evict', instanceId: 'first', deploymentId: 'd', revisionId: 'old', reason: 'replace' },
    { sequence: 1, kind: 'drain', instanceId: 'first', deploymentId: 'd', revisionId: 'old', reason: 'replace' },
    { sequence: 4, kind: 'warm', instanceId: 'later', deploymentId: 'd2', revisionId: 'r2', reason: 'selected' },
  ])
  assert.deepEqual(operations.map(operation => operation.kind), ['drain', 'evict', 'warm', 'warm'])
  assert.equal(operations[2].instanceId, 'later')
  assert.equal(operations[3].instanceId, 'first')
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
    sourcefile: 'model-residency.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'model-residency.test.cjs'))
testModule.filename = path.join(process.cwd(), 'model-residency.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

const panel = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'ClusterPage', 'ResidencySchedulerPanel.tsx'),
  'utf8',
)
assert.match(panel, /save_model_residency_policy/, 'policy must be persisted before reconciliation')
assert.match(panel, /begin_model_residency_drain/, 'drain must be persisted before polling')
assert.match(panel, /get_model_residency_drain_status/, 'in-flight requests must be polled')
assert.match(panel, /status\.activeRequests === 0/, 'eviction must wait for zero active requests')
assert.match(panel, /complete_model_residency_operation/, 'operation outcomes must be persisted')
assert.match(panel, /startInstance\(operation\.instanceId, false\)/, 'warming must use the existing validated start path')
assert.match(panel, /stopInstance\(operation\.instanceId\)/, 'eviction must use the existing stop path')

console.log('Model residency policy serialization and reconcile ordering passed.')
