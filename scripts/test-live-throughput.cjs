const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import {
    aggregateLiveThroughput,
    appendThroughputPoint,
    buildChartAxis,
    buildLiveThroughput,
    mergeThroughputPoints,
  } from './src/components/monitoring/monitoringViewModel'

  const task = (overrides = {}) => ({
    slot_id: 0,
    task_id: 1,
    started_at_ms: 1000,
    updated_at_ms: 1500,
    n_decoded: 10,
    tg: 20,
    tg_3s: null,
    history: [],
    prompt_tokens: null,
    prompt_time_ms: null,
    prompt_tps: null,
    gen_tokens: null,
    gen_time_ms: null,
    gen_tps: null,
    total_tokens: null,
    total_time_ms: null,
    spec_accept_rate: null,
    spec_accepted: null,
    spec_generated: null,
    spec_gen_time_ms: null,
    completed: false,
    ...overrides,
  })

  assert.deepEqual(
    buildLiveThroughput([
      task({ task_id: 1, tg: 30, tg_3s: 42 }),
      task({ task_id: 2, tg: 11, tg_3s: null }),
      task({ task_id: 3, tg: 99, tg_3s: 99, completed: true }),
    ], 200, 3),
    { value: 53, source: 'active-tasks', activeCount: 2 },
  )
  assert.deepEqual(
    buildLiveThroughput([task({ tg: 50, tg_3s: 0 })], 80, 1),
    { value: 0, source: 'active-tasks', activeCount: 1 },
  )
  assert.deepEqual(
    buildLiveThroughput([], 36, 2),
    { value: 36, source: 'llama-metrics', activeCount: 2 },
  )
  assert.deepEqual(
    buildLiveThroughput([], 36, 0),
    { value: 0, source: 'idle', activeCount: 0 },
  )
  assert.deepEqual(
    buildLiveThroughput([], 36, 2, true),
    { value: 0, source: 'idle', activeCount: 0 },
  )
  assert.deepEqual(
    aggregateLiveThroughput([
      { value: 42, source: 'active-tasks', activeCount: 1 },
      { value: 18, source: 'llama-metrics', activeCount: 2 },
      { value: 0, source: 'idle', activeCount: 0 },
    ]),
    { value: 60, source: 'mixed', activeCount: 3 },
  )

  assert.deepEqual(
    appendThroughputPoint(
      [{ ts: 80, value: 1 }, { ts: 95, value: 2 }, { ts: 100, value: 3 }],
      { ts: 100, value: 4 },
      10,
      3,
    ),
    [{ ts: 95, value: 2 }, { ts: 100, value: 4 }],
  )
  assert.deepEqual(
    mergeThroughputPoints(
      [{ ts: 1, value: 1 }, { ts: 2, value: 2 }],
      [{ ts: 2, value: 20 }, { ts: 3, value: 3 }],
    ),
    [{ ts: 1, value: 1 }, { ts: 2, value: 20 }, { ts: 3, value: 3 }],
  )
  assert.deepEqual(
    mergeThroughputPoints(
      Array.from({ length: 10 }, (_, index) => ({ ts: index, value: index })),
      [],
      0,
      3,
    ),
    [{ ts: 0, value: 0 }, { ts: 5, value: 5 }, { ts: 9, value: 9 }],
  )

  const axis = buildChartAxis([0, 73])
  assert.equal(axis.min, 0)
  assert.equal(axis.max, 80)
  assert.equal(axis.step, 20)
  assert.deepEqual(axis.ticks, [0, 20, 40, 60, 80])
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
    sourcefile: 'live-throughput.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'live-throughput.test.cjs'))
testModule.filename = path.join(process.cwd(), 'live-throughput.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

const root = path.join(__dirname, '..')
const rustSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'commands', 'server.rs'), 'utf8')
const performanceSource = fs.readFileSync(path.join(root, 'src', 'components', 'PerformancePage', 'PerformancePage.tsx'), 'utf8')
const bigScreenSource = fs.readFileSync(path.join(root, 'src', 'components', 'BigScreenPage.tsx'), 'utf8')
const primitiveSource = fs.readFileSync(path.join(root, 'src', 'components', 'monitoring', 'MonitoringPrimitives.tsx'), 'utf8')

assert.match(rustSource, /tg_3s: Option<f64>/, 'task events must expose llama-server rolling throughput')
assert.match(rustSource, /re_tg_3s/, 'the log parser must parse rolling throughput')
for (const source of [performanceSource, bigScreenSource]) {
  assert.match(source, /buildLiveThroughput/, 'monitoring views must use the shared live throughput model')
  assert.match(source, /appendThroughputPoint/, 'monitoring views must append live trend points')
  assert.match(source, /mergeThroughputPoints/, 'monitoring views must merge live and persisted trends')
  assert.doesNotMatch(source, /selectCurrentThroughput/, 'historical throughput must not drive the current value')
}
assert.match(primitiveSource, /buildChartAxis/, 'trend charts must render a numeric axis')
assert.match(primitiveSource, /unit\?: string/, 'trend charts must expose their measurement unit')

console.log('live throughput view-model tests passed')
