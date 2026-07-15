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
    buildFleetThroughputSeries,
    buildLiveThroughput,
    buildRequestPressure,
    buildTelemetryThroughputPoints,
    mergeThroughputPoints,
    monitoringFramePoints,
  } from './src/components/monitoring/monitoringViewModel'
  import {
    appendMonitoringFrame,
    mergeMonitoringFrames,
  } from './src/store/monitoringSlice'

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
    buildTelemetryThroughputPoints([
      { ts: 1, tokens_per_sec: 72.6, requests_processing: 0 },
      { ts: 2, tokens_per_sec: 45, requests_processing: 1 },
      { ts: 3, tokens_per_sec: 30, requests_processing: null },
      { ts: 4, tokens_per_sec: null, requests_processing: 1 },
    ]),
    [
      { ts: 1, value: 0 },
      { ts: 2, value: 45 },
      { ts: 3, value: 30 },
      { ts: 4, value: 0 },
    ],
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
      [{ ts: 1, value: 70 }, { ts: 4, value: 72 }],
      [{ ts: 2, value: 0 }, { ts: 5, value: 0 }],
    ),
    [{ ts: 1, value: 70 }, { ts: 2, value: 0 }, { ts: 5, value: 0 }],
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

  const frame = (overrides = {}) => ({
    instanceId: 'a',
    sessionId: 'session-a',
    sessionStartedAt: 1000,
    ts: 1000,
    workload: 'inference',
    state: 'active',
    throughput: 50,
    throughputUnit: 'tok/s',
    outputTokensPerSecond: 50,
    inputTokensPerSecond: 0,
    itemsPerSecond: null,
    activeRequests: 1,
    queuedRequests: 0,
    slotCapacity: 4,
    busySlots: 1,
    averageLatencyMs: null,
    successRate: null,
    source: 'task',
    dataAgeMs: 0,
    system: null,
    ...overrides,
  })

  const instanceFrames = [frame({ ts: 1000, throughput: 40 }), frame({ ts: 2000, throughput: 50 })]
  assert.deepEqual(monitoringFramePoints(instanceFrames, 'inference'), [
    { ts: 1000, value: 40 },
    { ts: 2000, value: 50 },
  ])
  assert.deepEqual(
    buildFleetThroughputSeries(
      { a: instanceFrames },
      { a: instanceFrames[1] },
      ['a'],
    ).points,
    monitoringFramePoints(instanceFrames, 'inference'),
    'one-instance wallboard and performance view must share the exact same points',
  )

  const vectorFrame = frame({
    instanceId: 'v',
    sessionId: 'session-v',
    workload: 'embedding',
    throughput: 300,
    throughputUnit: 'input tok/s',
    outputTokensPerSecond: null,
    inputTokensPerSecond: 300,
    itemsPerSecond: 2,
    source: 'llama',
  })
  const mixed = buildFleetThroughputSeries(
    { a: [instanceFrames[1]], v: [vectorFrame] },
    { a: instanceFrames[1], v: vectorFrame },
    ['a', 'v'],
  )
  assert.equal(mixed.mode, 'mixed')
  assert.equal(mixed.unit, 'tok/s')
  assert.equal(mixed.current, 50, 'input token throughput must not be added to generation throughput')
  assert.equal(mixed.vectorItemsPerSecond, 2)

  assert.deepEqual(
    appendMonitoringFrame(instanceFrames, frame({ ts: 2000, throughput: 55 })),
    [instanceFrames[0], frame({ ts: 2000, throughput: 55 })],
    'same-bucket updates must replace instead of adding duplicate points',
  )
  assert.deepEqual(
    appendMonitoringFrame(instanceFrames, frame({ sessionId: 'session-b', sessionStartedAt: 3000, ts: 3000 })),
    [frame({ sessionId: 'session-b', sessionStartedAt: 3000, ts: 3000 })],
    'a newer run must clear the previous in-memory session timeline',
  )
  assert.deepEqual(
    mergeMonitoringFrames(
      [frame({ ts: 3000 })],
      [frame({ ts: 1000 }), frame({ ts: 2000 })],
    ).map(point => point.ts),
    [1000, 2000, 3000],
    'hydration must merge and order frames in one batch',
  )
  assert.equal(
    mergeMonitoringFrames(
      [frame({ sessionId: 'session-a', sessionStartedAt: 1000, ts: 3000 })],
      [frame({ sessionId: 'session-b', sessionStartedAt: 3000, ts: 3000 })],
    )[0].sessionId,
    'session-b',
    'a new session in the same one-second bucket must win hydration races',
  )
  assert.deepEqual(
    appendMonitoringFrame(
      [frame({ sessionId: 'session-b', sessionStartedAt: 3000, ts: 4000 })],
      frame({ sessionId: 'session-a', sessionStartedAt: 1000, ts: 5000 }),
    ).map(point => point.sessionId),
    ['session-b'],
    'a delayed frame from an older session must not replace the current session',
  )

  assert.deepEqual(buildRequestPressure(1, 0, 4), {
    active: 1,
    queued: 0,
    capacity: 4,
    percent: 25,
    level: 'normal',
  })
  assert.equal(buildRequestPressure(1, 0).percent, 0)
  assert.equal(buildRequestPressure(1, 1, 4).level, 'high')
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
const monitoringSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'commands', 'monitoring.rs'), 'utf8')
const runtimeEventsSource = fs.readFileSync(path.join(root, 'src', 'store', 'runtimeEvents.ts'), 'utf8')

assert.match(rustSource, /tg_3s: Option<f64>/, 'task events must expose llama-server rolling throughput')
assert.match(rustSource, /re_tg_3s/, 'the log parser must parse rolling throughput')
for (const source of [performanceSource, bigScreenSource]) {
  assert.match(source, /monitoringFramesByInstance/, 'monitoring views must consume the global authoritative timeline')
  assert.match(source, /monitoringCurrentByInstance/, 'monitoring views must consume the same current frame')
  assert.doesNotMatch(source, /listen<MetricsEvent>/, 'views must not register page-local metrics listeners')
  assert.doesNotMatch(source, /listen<PerfUpdateEvent>/, 'views must not register page-local task listeners')
}
assert.match(performanceSource, /monitoringFramePoints/, 'performance view must project selected-instance frames')
assert.match(bigScreenSource, /buildFleetThroughputSeries/, 'wallboard must use workload-aware fleet aggregation')
assert.match(runtimeEventsSource, /listen<MonitoringFrame>\('monitoring-frame'/, 'monitoring frames must be listened to once at application scope')
assert.match(runtimeEventsSource, /get_monitoring_series/, 'the global store must hydrate the backend timeline')
assert.match(monitoringSource, /FRAME_INTERVAL_MS: i64 = 1_000/, 'backend monitoring must use one-second canonical buckets')
assert.match(monitoringSource, /items_per_second/, 'backend frames must expose vector item throughput')
assert.match(monitoringSource, /input_tokens_per_second/, 'backend frames must expose vector input throughput')
assert.match(primitiveSource, /buildChartAxis/, 'trend charts must render a numeric axis')
assert.match(primitiveSource, /unit\?: string/, 'trend charts must expose their measurement unit')
assert.match(primitiveSource, /point\.ts - domainStart/, 'trend charts must position points by real timestamps')

console.log('live throughput view-model tests passed')
