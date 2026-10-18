const assert = require('assert')
const fs = require('fs')
const path = require('path')
const ts = require('typescript')

require.extensions['.ts'] = function loadTs(module, filename) {
  const source = fs.readFileSync(filename, 'utf8')
  const output = ts.transpileModule(source, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2020,
      esModuleInterop: true,
    },
    fileName: filename,
  }).outputText
  module._compile(output, filename)
}

const { mergeRestoredDownloadTask, restoredDownloadStatus, restoredDownloadTimestamp } = require(path.join('..', 'src', 'store', 'downloadMerge.ts'))

const existingActive = {
  id: 'task-a',
  runId: 'run-current',
  fileName: 'model.gguf',
  remotePath: 'folder/model.gguf',
  fileType: 'model',
  saveDir: 'models',
  repoId: 'repo-a',
  source: 'huggingface',
  downloaded: 640,
  total: 1000,
  speed: 128,
  status: 'active',
  version: 4,
}

const olderPersistedFile = {
  name: 'model.gguf',
  path: 'folder/model.gguf',
  size: 1000,
  file_type: 'model',
  task_id: 'task-a',
  run_id: 'run-old',
  downloaded: 120,
  version: 3,
  status: 'queued',
}

const merged = mergeRestoredDownloadTask(
  existingActive,
  olderPersistedFile,
  {
    repoId: 'repo-a',
    source: 'huggingface',
    saveDir: 'models',
    entryStatus: 'queued',
  },
)

assert.strictEqual(merged.status, 'active')
assert.strictEqual(merged.runId, 'run-current')
assert.strictEqual(merged.downloaded, 640)
assert.strictEqual(merged.version, 4)
assert.strictEqual(merged.error, undefined)

const restoredOnly = mergeRestoredDownloadTask(
  undefined,
  {
    ...olderPersistedFile,
    task_id: 'task-b',
    run_id: 'run-b',
    status: undefined,
    version: undefined,
  },
  {
    repoId: 'repo-a',
    source: 'huggingface',
    saveDir: 'models',
    entryStatus: 'active',
  },
)

assert.strictEqual(restoredOnly.status, 'active')
assert.strictEqual(restoredOnly.runId, 'run-b')
assert.strictEqual(restoredOnly.version, 0)

assert.strictEqual(restoredDownloadStatus('not-a-real-status', 'also-unknown'), 'queued')
assert.strictEqual(restoredDownloadTimestamp(1_700_000_000), 1_700_000_000_000)
assert.strictEqual(restoredDownloadTimestamp(1_700_000_000_000), 1_700_000_000_000)

const existingCancelled = {
  ...existingActive,
  status: 'cancelled',
  version: 4,
  runId: 'run-cancelled',
}
const cancelledMerge = mergeRestoredDownloadTask(
  existingCancelled,
  { ...olderPersistedFile, version: 4, status: 'active', run_id: 'run-restored' },
  {
    repoId: 'repo-a',
    source: 'huggingface',
    saveDir: 'models',
    entryStatus: 'active',
  },
)
assert.strictEqual(cancelledMerge.status, 'cancelled')
assert.strictEqual(cancelledMerge.runId, 'run-cancelled')

const restarted = mergeRestoredDownloadTask(
  { ...existingActive, status: 'completed', downloaded: 1000, completedAt: 123, remoteChanged: true },
  { ...olderPersistedFile, version: 5, run_id: 'new-run', downloaded: 0, size: 2000, status: 'paused' },
  { repoId: 'repo-a', source: 'huggingface', saveDir: 'models', entryStatus: 'paused' },
)
assert.strictEqual(restarted.status, 'paused')
assert.strictEqual(restarted.downloaded, 0)
assert.strictEqual(restarted.total, 2000)
assert.strictEqual(restarted.speed, 0)
assert.strictEqual(restarted.completedAt, undefined)
assert.strictEqual(restarted.remoteChanged, undefined)
assert.strictEqual(restarted.runId, 'new-run')

console.log('download restore merge regression passed')
