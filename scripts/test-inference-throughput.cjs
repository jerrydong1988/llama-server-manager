const assert = require('node:assert/strict')
const path = require('node:path')
const Module = require('node:module')
const { buildSync } = require('esbuild')
const result = buildSync({ entryPoints: ['src/components/monitoring/inferenceThroughput.ts'], bundle: true, platform: 'node', format: 'cjs', write: false })
const loaded = new Module(path.resolve('inference-throughput-test.cjs'))
loaded.filename = path.resolve('inference-throughput-test.cjs')
loaded._compile(result.outputFiles[0].text, loaded.filename)
const { currentInferenceRate, aggregateInferenceRates, inferenceTrend, fleetInferenceTrend, activityBands, summarizeWallDecode, formatInferenceRate, unavailableRate } = loaded.exports
const rate = (value, ts = 1000) => ({ value, state: 'live', source: 'log', sampledAt: ts, windowMs: 3000, covered: 1, expected: 1 })
const idle = { ...rate(0), state: 'idle', sampledAt: null }
const frame = { ts: 1000, workload: 'inference', inference: { prefill: rate(1200), decode: idle, tasks: [] } }
assert.equal(currentInferenceRate(frame, 'prefill', 1000).value, 1200)
assert.equal(currentInferenceRate(frame, 'prefill', 7000).state, 'stale')
assert.equal(currentInferenceRate(frame, 'decode', 10000).state, 'unavailable')
assert.equal(currentInferenceRate({ ...frame, inference: undefined, throughput: 99 }, 'decode', 1000).value, null)
assert.equal(formatInferenceRate(idle), '0 tok/s')
assert.equal(formatInferenceRate(unavailableRate()), '—')
const partial = aggregateInferenceRates([rate(1200), unavailableRate()])
assert.equal(partial.state, 'partial')
assert.equal(partial.value, 1200)
assert.equal(partial.covered, 1)
assert.equal(partial.expected, 2)
assert.equal(aggregateInferenceRates([idle, idle]).state, 'idle')
const points = inferenceTrend([frame], [{ ts: 0, tokens_per_sec: 100, prompt_tokens_per_sec: 999 }], 'prefill', 0)
assert.deepEqual(points.map(({ ts, value, state }) => [ts, value, state]), [[0, null, 'unavailable'], [1000, 1200, 'live']])
const fleet = fleetInferenceTrend({ a: [frame] }, ['a', 'b'], 'prefill', 0)[0]
assert.deepEqual([fleet.ts, fleet.value, fleet.state, fleet.covered, fleet.expected], [1000, 1200, 'partial', 1, 2])
assert.equal(aggregateInferenceRates([partial, rate(20)]).state, 'partial')
const resumed = { ...frame, ts: 20000, inference: { ...frame.inference, prefill: rate(1400, 20000) } }
assert.deepEqual(inferenceTrend([frame, resumed], [], 'prefill', 0).map(point => point.value), [1200, null, 1400], 'disconnected periods must break the line')
assert.deepEqual(fleetInferenceTrend({ a: [frame, resumed] }, ['a'], 'prefill', 0).map(point => point.value), [1200, null, 1400])
const timeline = [
  frame,
  { ...frame, ts: 2000, inference: { prefill: rate(0, 2000), decode: rate(32, 2000) } },
  { ...frame, ts: 3000, inference: { prefill: idle, decode: idle } },
  resumed,
]
const bands = activityBands(...['prefill', 'decode'].map(phase => inferenceTrend(timeline, [], phase, 0)), 0, 23000)
assert.deepEqual(bands.map(({ from, to, state }) => [from, to, state]), [
  [0, 1000, 'unknown'], [1000, 2000, 'prefill'], [2000, 3000, 'both'],
  [3000, 4000, 'idle'], [4000, 20000, 'unknown'], [20000, 21000, 'prefill'], [21000, 23000, 'unknown'],
])
assert.equal(activityBands([fleet], [{ ...fleet, active: false }], 1000, 2000)[0].partial, true)
const observed = (ts, value, state = 'live') => ({ ...fleet, ts, value, state })
const readings = [observed(1000, 0), observed(2000, 30), observed(3000, 0, 'idle'), observed(4000, 16),
  observed(5000, 8, 'partial'), observed(6000, null, 'stale'), observed(7000, 19), observed(16000, 29)]
assert.deepEqual(summarizeWallDecode(readings).map(({ ts, value }) => [ts, value]),
  [[1500, 15], [3000, 0], [4000, 16], [5000, 8], [6000, null], [7000, 19], [16000, 29]])
assert.equal(readings[0].value, 0)
console.log('Inference throughput semantics passed: freshness, zero/unknown, coverage and historical gaps.')
