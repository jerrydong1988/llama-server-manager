const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const Module = require('node:module')
const ts = require('typescript')
const root = path.resolve(__dirname, '..')
const handlers = new Map()
let invokeHook = async command => command === 'get_monitoring_series' ? [] : {}
const originalLoad = Module._load
Module._load = function (request) {
  if (request === '../lib/ipc') return { invokeApp: (...args) => invokeHook(...args) }
  if (request === '@tauri-apps/api/event') return { listen: async (name, handler) => { handlers.set(name, handler); return () => {} } }
  return originalLoad.apply(this, arguments)
}
require.extensions['.ts'] = (mod, filename) => mod._compile(ts.transpileModule(fs.readFileSync(filename, 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, esModuleInterop: true }, fileName: filename,
}).outputText, filename)
global.window = new EventTarget()
global.document = { visibilityState: 'visible' }
global.crypto ||= require('node:crypto').webcrypto
const realSetTimeout = global.setTimeout
global.setTimeout = (fn, ms, ...args) => realSetTimeout(fn, ms, ...args).unref()
const { createDownloadSlice } = require(path.join(root, 'src/store/downloadSlice.ts'))
const { registerGlobalStoreListeners } = require(path.join(root, 'src/store/runtimeEvents.ts'))
let state
const get = () => state
const set = update => { state = { ...state, ...(typeof update === 'function' ? update(state) : update) } }
function reset() {
  state = { downloadTasks: {}, downloadQueue: [], modelDirs: [], runtimeWarnings: [],
    addRuntimeWarning: warning => state.runtimeWarnings.push(warning), setSysMetrics: () => {},
    hydrateMonitoringFrames: () => {}, hydrateCheckpointStatuses: () => {}, scanModels: () => {},
    ...createDownloadSlice(set, get),
  }
}
function emit(name, payload) { handlers.get(name)({ payload }) }
async function enqueue(early) {
  reset()
  let payload
  invokeHook = async (command, args) => {
    if (command === 'enqueue_download_queue') {
      const entry = args.entry, file = entry.files[0]
      payload = { queueId: entry.id, taskId: file.task_id, runId: file.run_id, version: file.version,
        fileName: file.name, remotePath: file.path, repoId: entry.repo_id, source: entry.source, total: 16, downloaded: 16 }
      if (early) { emit('download-started', payload); emit('download-complete', payload) }
    }
    return command === 'get_monitoring_series' ? [] : {}
  }
  await state.addToDownloadQueue({ repoId: 'test/tiny', source: 'huggingface', files: [{ name: 'tiny.gguf', size: 16 }], saveDir: 'models' })
  if (!early) { emit('download-started', payload); emit('download-complete', payload) }
  assert.equal(state.downloadTasks[payload.taskId].status, 'completed')
  assert.equal(state.downloadTasks[payload.taskId].downloaded, 16)
  assert.equal(state.downloadQueue.length, 0)
}
async function resume(all) {
  reset()
  state.downloadTasks.one = { id: 'one', runId: 'old', version: 1, fileName: 'tiny.gguf', remotePath: 'tiny.gguf',
    repoId: 'test/tiny', source: 'huggingface', saveDir: 'models', status: 'paused', downloaded: 8, total: 16 }
  const payload = { taskId: 'one', runId: 'new', version: 2, fileName: 'tiny.gguf', remotePath: 'tiny.gguf', repoId: 'test/tiny', source: 'huggingface', downloaded: 16, total: 16 }
  invokeHook = async command => {
    if (command === (all ? 'resume_all_downloads' : 'resume_download_task')) {
      emit('download-started', payload); emit('download-complete', payload)
      const identity = { taskId: 'one', runId: 'new', version: 2 }
      return all ? [identity] : identity
    }
    if (command === 'get_download_manager_snapshot') return { queue: [{ id: 'queue', repo_id: 'test/tiny', source: 'huggingface', save_dir: 'models', status: 'completed',
      files: [{ name: 'tiny.gguf', path: 'tiny.gguf', task_id: 'one', run_id: 'new', version: 2, status: 'completed', downloaded: 16, size: 16 }] }] }
    return command === 'get_monitoring_series' ? [] : {}
  }
  if (all) await state.resumeAllDownloads()
  else await state.resumeDownloadTask('one')
  assert.equal(state.downloadTasks.one.status, 'completed')
  assert.equal(state.downloadTasks.one.downloaded, 16)
  emit('download-error', { ...payload, runId: 'old', version: 1, error: 'late old run' })
  assert.equal(state.downloadTasks.one.status, 'completed', 'stale run cannot replace completion')
}
async function enqueueFailure() {
  reset()
  invokeHook = async command => { if (command === 'enqueue_download_queue') throw new Error('disk unavailable'); return {} }
  assert.equal(await state.addToDownloadQueue({ repoId: 'test/tiny', source: 'huggingface', files: [{ name: 'tiny.gguf', size: 16 }], saveDir: 'models' }), false)
  assert.deepEqual(state.downloadTasks, {})
  assert.deepEqual(state.downloadQueue, [])
  assert.equal(state.runtimeWarnings.length, 1)
}
;(async () => {
  reset()
  registerGlobalStoreListeners({ getState: get, setState: set }, [])
  await Promise.resolve()
  await enqueue(false)
  await enqueue(true)
  await resume(false)
  await resume(true)
  await enqueueFailure()
  console.log('Functional state regressions passed: early completion, resume snapshots, stale runs, enqueue rollback.')
})().catch(error => { console.error(error); process.exitCode = 1 })
