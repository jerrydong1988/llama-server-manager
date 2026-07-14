const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const entry = `
  import assert from 'node:assert/strict'
  import {
    buildPerformanceMode,
    buildVectorComparisonRows,
    buildVectorKpis,
    buildVectorSourceState,
    buildVectorTrendSeries,
    getActiveTaskColumns,
    workloadLabel,
  } from './src/components/PerformancePage/vectorPerformance'

  const vectorAnalysis = (overrides = {}) => ({
    workload: 'embedding',
    logAvailable: true,
    proxyAvailable: true,
    completedItems: 80,
    inputTokens: 420,
    averageInputTokensPerSecond: 42,
    averageItemsPerSecond: 8,
    taskDurationP50Ms: 9,
    taskDurationP95Ms: 18,
    proxyRequestCount: 10,
    proxyItemCount: 80,
    proxyDurationP50Ms: 22,
    proxyDurationP95Ms: 50,
    proxySuccessRate: 0.9,
    proxyFailureRate: 0.1,
    trend: [
      { timestamp: 1000, inputTokensPerSecond: 40, itemsPerSecond: 7 },
      { timestamp: 2000, inputTokensPerSecond: null, itemsPerSecond: 0 },
    ],
    ...overrides,
  })

  const session = (workload) => ({ workload })
  const embedding = vectorAnalysis()
  assert.deepEqual(
    buildVectorKpis(embedding, 'zh-CN'),
    [
      { key: 'input', label: '输入吞吐', value: '42.0 tok/s', available: true },
      { key: 'items', label: '向量项吞吐', value: '8.0 项/s', available: true },
      { key: 'p95', label: '任务 P95', value: '18 ms', available: true },
    ],
  )
  assert.equal(buildPerformanceMode(session('inference'), null).kind, 'inference')
  const embeddingMode = buildPerformanceMode(session('embedding'), { vector_analysis: embedding })
  assert.equal(embeddingMode.kind, 'vector')
  assert.equal(embeddingMode.inputThroughput, 42)
  assert.equal(embeddingMode.itemThroughput, 8)
  assert.equal(embeddingMode.proxyRequestCount, 10)
  assert.equal(embeddingMode.itemName, 'vector')

  const reranker = vectorAnalysis({ workload: 'reranker', proxyRequestCount: 3 })
  const rerankerMode = buildPerformanceMode(session('reranker'), { vector_analysis: reranker })
  assert.equal(rerankerMode.kind, 'vector')
  assert.equal(rerankerMode.itemName, 'document')
  assert.equal(workloadLabel('reranker', 'zh-CN'), 'Reranker')
  assert.equal(workloadLabel('inference', 'zh-CN'), '生成')

  const logOnly = vectorAnalysis({ proxyAvailable: false, proxyRequestCount: null })
  const proxyOnly = vectorAnalysis({
    logAvailable: false,
    completedItems: null,
    inputTokens: null,
    averageInputTokensPerSecond: null,
    averageItemsPerSecond: null,
    taskDurationP50Ms: null,
    taskDurationP95Ms: null,
  })
  const noData = vectorAnalysis({
    logAvailable: false,
    proxyAvailable: false,
    completedItems: null,
    inputTokens: null,
    averageInputTokensPerSecond: null,
    averageItemsPerSecond: null,
    taskDurationP50Ms: null,
    taskDurationP95Ms: null,
    proxyRequestCount: null,
    proxyItemCount: null,
    proxyDurationP50Ms: null,
    proxyDurationP95Ms: null,
    proxySuccessRate: null,
    proxyFailureRate: null,
    trend: [],
  })
  assert.equal(buildVectorSourceState(embedding, 'zh-CN').kind, 'full')
  assert.equal(buildVectorSourceState(logOnly, 'zh-CN').kind, 'log-only')
  assert.equal(buildVectorSourceState(proxyOnly, 'zh-CN').kind, 'proxy-only')
  assert.equal(buildVectorSourceState(noData, 'zh-CN').kind, 'none')
  assert.equal(buildPerformanceMode(session('embedding'), { vector_analysis: proxyOnly }).inputThroughput, null)
  assert.equal(buildPerformanceMode(session('embedding'), { vector_analysis: noData }).proxyRequestCount, null)

  assert.deepEqual(buildVectorTrendSeries(embedding, 'input'), [
    { timestamp: 1000, value: 40 },
    { timestamp: 2000, value: null },
  ])
  assert.deepEqual(buildVectorTrendSeries(embedding, 'items'), [
    { timestamp: 1000, value: 7 },
    { timestamp: 2000, value: 0 },
  ])
  assert.deepEqual(buildVectorTrendSeries(proxyOnly, 'items'), [])

  const comparison = buildVectorComparisonRows(
    embedding,
    {
      averageInputTokensPerSecond: 40,
      averageItemsPerSecond: 10,
      taskDurationP95Ms: 20,
      sessionCount: 3,
    },
    'zh-CN',
  )
  assert.equal(comparison.length, 3)
  assert.equal(comparison[0].current, '42.0 tok/s')
  assert.equal(comparison[0].baseline, '40.0 tok/s')
  assert.equal(comparison[1].favorable, false)
  assert.equal(comparison[2].favorable, true)

  assert.deepEqual(getActiveTaskColumns('embedding'), ['slot', 'elapsed', 'workload'])
  assert.deepEqual(getActiveTaskColumns('reranker'), ['slot', 'elapsed', 'workload'])
  assert.deepEqual(getActiveTaskColumns('inference'), ['slot', 'elapsed', 'generated', 'speed', 'speculative'])
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
    sourcefile: 'vector-performance.test.ts',
    loader: 'ts',
  },
})

const testModule = new Module(path.join(process.cwd(), 'vector-performance.test.cjs'))
testModule.filename = path.join(process.cwd(), 'vector-performance.test.cjs')
testModule.paths = Module._nodeModulePaths(process.cwd())
testModule._compile(bundled.outputFiles[0].text, testModule.filename)

const source = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'PerformancePage', 'vectorPerformance.ts'),
  'utf8',
)
assert.doesNotMatch(source, /\.tokens_per_sec\b/, 'vector helpers must not read generation throughput')
assert.doesNotMatch(source, /spec_accept|avg_cached_slots|max_context_tokens/, 'vector helpers must not read generation diagnostics')
assert.doesNotMatch(source, /requests_total/, 'legacy decode count must not be presented as HTTP requests')
assert.match(source, /proxyRequestCount/, 'HTTP request count must come from the proxy source')

const performancePageSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'PerformancePage', 'PerformancePage.tsx'),
  'utf8',
)
assert.match(performancePageSource, /buildPerformanceMode\(/, 'performance page must use the workload view model')
assert.match(performancePageSource, /data-workload-badge/, 'selected session must render a workload badge')
assert.match(performancePageSource, /data-vector-source-state/, 'vector source availability must be visible')
assert.match(performancePageSource, /performanceMode\.kind === 'vector'/, 'primary metrics must branch by workload')
assert.match(performancePageSource, /buildVectorTrendSeries\(/, 'vector trends must use vector task buckets')
assert.match(performancePageSource, /workload=\{performanceMode\.workload\}/, 'active task rows must receive the persisted workload')
assert.match(
  performancePageSource,
  /const history = sessions\.filter\(session =>\s*session\.workload === selectedSession\.workload\s*&&/,
  'historical comparison must not mix workloads after an instance model switch',
)

const primitiveSource = fs.readFileSync(
  path.join(__dirname, '..', 'src', 'components', 'monitoring', 'MonitoringPrimitives.tsx'),
  'utf8',
)
assert.match(primitiveSource, /workload: ModelWorkload/, 'active request primitive must switch fields by workload')
assert.match(primitiveSource, /workload !== 'inference'/, 'generation-only fields must be gated')

console.log('vector performance view-model tests passed')
