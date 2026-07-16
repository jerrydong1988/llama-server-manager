const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import {
    buildSpeculativeDecodingSummary,
    isSpeculativeDecodingConfigured,
  } from './src/components/monitoring/speculativeDecoding'

  const request = (overrides = {}) => ({
    session_id: 'session',
    task_id: 1,
    slot_id: 0,
    completed_at: 1000,
    source: 'log',
    model: null,
    target_instance_id: null,
    http_status: null,
    error_text: null,
    prompt_tokens: null,
    prompt_time_ms: null,
    prompt_tps: null,
    generated_tokens: null,
    generation_time_ms: null,
    generation_tps: null,
    total_tokens: null,
    total_time_ms: null,
    spec_accept_rate: null,
    spec_accepted: null,
    spec_generated: null,
    spec_gen_time_ms: null,
    ...overrides,
  })
  const task = (overrides = {}) => ({
    slot_id: 0,
    task_id: 10,
    started_at_ms: 1000,
    updated_at_ms: 2000,
    n_decoded: 20,
    tg: 10,
    tg_3s: 10,
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

  assert.equal(isSpeculativeDecodingConfigured({ spec_type: 'draft-mtp' }), true)
  assert.equal(isSpeculativeDecodingConfigured({ spec_type: 'ngram-cache' }), true)
  assert.equal(isSpeculativeDecodingConfigured({ spec_type: 'none' }), false)
  assert.equal(isSpeculativeDecodingConfigured({ spec_type: '' }), false)

  const historical = buildSpeculativeDecodingSummary({
    requests: [
      request({ task_id: 1, spec_accept_rate: 0.8, spec_accepted: 8, spec_generated: 10, spec_gen_time_ms: 10 }),
      request({ task_id: 2, spec_accept_rate: 0.2, spec_accepted: 2, spec_generated: 30, spec_gen_time_ms: 30 }),
    ],
  })
  assert.equal(historical.source, 'history')
  assert.equal(historical.requestCount, 2)
  assert.equal(historical.acceptedTokens, 10)
  assert.equal(historical.generatedTokens, 40)
  assert.equal(historical.acceptanceRate, 0.25, 'acceptance must be weighted by draft tokens')
  assert.equal(historical.avgGenerationTimeMs, 20)

  const canonical = buildSpeculativeDecodingSummary({
    analysis: {
      request_count: 2,
      acceptance_rate: 0.25,
      accepted_tokens: 10,
      generated_tokens: 40,
      avg_generation_time_ms: 20,
    },
    requests: [request({ spec_accepted: 8, spec_generated: 10 })],
    activeTasks: [task({ spec_accepted: 3, spec_generated: 5, spec_gen_time_ms: 10 })],
  })
  assert.equal(canonical.source, 'mixed')
  assert.equal(canonical.requestCount, 3)
  assert.equal(canonical.acceptedTokens, 13, 'request rows already represented by analysis must not be counted twice')
  assert.equal(canonical.generatedTokens, 45)
  assert.equal(canonical.acceptanceRate, 13 / 45)
  assert.equal(canonical.latestAcceptanceRate, 0.6)
  assert.equal(canonical.liveRequestCount, 1)

  const mtpStatsOnly = buildSpeculativeDecodingSummary({
    activeTasks: [task({ spec_accepted: 9, spec_generated: 12 })],
  })
  assert.equal(mtpStatsOnly.source, 'live')
  assert.equal(mtpStatsOnly.acceptanceRate, 0.75)
  assert.equal(mtpStatsOnly.latestAcceptanceRate, 0.75)

  const deduplicatedCompletion = buildSpeculativeDecodingSummary({
    requests: [request({ task_id: 77, spec_accepted: 6, spec_generated: 10 })],
    lastCompletedTasks: [{
      ...task({ task_id: 77, spec_accepted: 6, spec_generated: 10, completed: true }),
      session_id: 'session',
    }],
  })
  assert.equal(deduplicatedCompletion.requestCount, 1)
  assert.equal(deduplicatedCompletion.acceptedTokens, 6)
  assert.equal(deduplicatedCompletion.generatedTokens, 10)

  const waiting = buildSpeculativeDecodingSummary({ configured: true })
  assert.equal(waiting.configured, true)
  assert.equal(waiting.hasData, false)
  assert.equal(waiting.source, 'waiting')
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
    sourcefile: 'speculative-decoding.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'speculative-decoding.test.cjs'))
testModule.filename = path.join(process.cwd(), 'speculative-decoding.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

const performancePage = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'PerformancePage', 'PerformancePage.tsx'),
  'utf8',
)
const bigScreenPage = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'BigScreenPage.tsx'),
  'utf8',
)
const telemetrySource = fs.readFileSync(
  path.join(__dirname, '..', 'src-tauri', 'src', 'commands', 'telemetry.rs'),
  'utf8',
)
const monitoringSlice = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'monitoringSlice.ts'),
  'utf8',
)
const runtimeEvents = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'store', 'runtimeEvents.ts'),
  'utf8',
)

assert.match(performancePage, /analysis\?\.speculative_analysis/, 'performance view must use the complete session aggregate')
assert.match(performancePage, /data-speculative-analysis/, 'performance view must expose speculative analysis')
assert.match(bigScreenPage, /data-wall-speculative-analysis/, 'big screen must expose speculative analysis')
assert.match(bigScreenPage, /latest inference session|recentSessionSource|speculativeRequests/i, 'big screen must retain recent inference statistics')
assert.match(telemetrySource, /1\.0 \* COALESCE\(SUM\(spec_accepted\), 0\) \/ SUM\(spec_generated\)/, 'backend must use token-weighted acceptance')
assert.match(monitoringSlice, /lastCompletedTaskByInstance/, 'the shared store must retain the immediate completion event')
assert.match(runtimeEvents, /lastCompletedTaskByInstance:[\s\S]*null/, 'a new server session must clear stale completion data')

console.log('speculative decoding analysis tests passed')
