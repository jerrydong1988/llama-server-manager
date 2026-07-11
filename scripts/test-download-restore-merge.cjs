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

const { mergeRestoredDownloadTask } = require(path.join('..', 'src', 'store', 'downloadMerge.ts'))

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

console.log('download restore merge regression passed')
