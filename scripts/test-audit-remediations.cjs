const assert = require('node:assert/strict')
const fs = require('node:fs')
const Module = require('node:module')
const path = require('node:path')
const esbuild = require('esbuild')

const root = path.join(__dirname, '..')
function loadTypeScriptModule(relative) {
  const filename = path.join(root, relative)
  const compiled = esbuild.transformSync(fs.readFileSync(filename, 'utf8'), {
    format: 'cjs',
    loader: 'ts',
    sourcefile: filename,
    target: 'node20',
  })
  const loaded = new Module(filename)
  loaded.filename = filename
  loaded.paths = Module._nodeModulePaths(path.dirname(filename))
  loaded._compile(compiled.code, filename)
  return loaded.exports
}

const { maskStartupCommandSecrets } = loadTypeScriptModule('src/store/commandFormatting.ts')
const { forEachConcurrent } = loadTypeScriptModule('src/utils/async.ts')
const behaviorTests = (async () => {
  assert.equal(
    maskStartupCommandSecrets('llama-server --api-key secret-value --port 8080'),
    'llama-server --api-key ******** --port 8080',
  )
  assert.equal(
    maskStartupCommandSecrets('llama-server --api-key="secret value" --api-key-file key.txt'),
    'llama-server --api-key=******** --api-key-file key.txt',
  )

  let active = 0
  let peak = 0
  const processed = []
  await forEachConcurrent([1, 2, 3, 4, 5, 6, 7], 3, async value => {
    active += 1
    peak = Math.max(peak, active)
    await new Promise(resolve => setTimeout(resolve, 5))
    processed.push(value)
    active -= 1
  })
  assert.equal(peak, 3)
  assert.deepEqual(processed.sort((left, right) => left - right), [1, 2, 3, 4, 5, 6, 7])
})()
const downloadSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'commands', 'download.rs'), 'utf8')
const proxySource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'commands', 'proxy.rs'), 'utf8')
const serverSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'commands', 'server.rs'), 'utf8')
const telemetrySource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'commands', 'telemetry.rs'), 'utf8')
const downloadManagerSource = fs.readFileSync(path.join(root, 'src', 'components', 'DownloadManager.tsx'), 'utf8')
const downloadBrowseSource = fs.readFileSync(path.join(root, 'src', 'components', 'DownloadManager', 'useDownloadBrowse.ts'), 'utf8')
const bigScreenSource = fs.readFileSync(path.join(root, 'src', 'components', 'BigScreenPage.tsx'), 'utf8')
const mainSource = fs.readFileSync(path.join(root, 'src-tauri', 'src', 'main.rs'), 'utf8')
const downloadSliceSource = fs.readFileSync(path.join(root, 'src', 'store', 'downloadSlice.ts'), 'utf8')
const runtimeEventsSource = fs.readFileSync(path.join(root, 'src', 'store', 'runtimeEvents.ts'), 'utf8')
const clusterPageSource = fs.readFileSync(path.join(root, 'src', 'components', 'ClusterPage', 'ClusterPage.tsx'), 'utf8')
const instanceManagerSource = fs.readFileSync(path.join(root, 'src', 'components', 'InstanceManager.tsx'), 'utf8')

assert.match(downloadSource, /fn verified_managed_cleanup_path/, 'cleanup must canonicalize managed download paths')
assert.match(downloadSource, /download_shutting_down\.load\(Ordering::SeqCst\)/, 'the scheduler must stop admitting work during shutdown')
assert.match(downloadSource, /let terminal_persisted = if let Some\(entry\)/, 'inflight recovery must remain until terminal state is durable')
assert.match(downloadSource, /fn quarantine_corrupt_state/, 'a corrupt queue must be preserved before creating replacement state')
assert.match(downloadSource, /Some\("completed" \| "cancelled"\)/, 'cancelled files must be terminal and excluded from retries')
assert.match(proxySource, /proxy_lifecycle_lock\.lock\(\)\.await/, 'proxy lifecycle transitions must be serialized')
assert.match(serverSource, /stdout_pump\.join\(\)/, 'server exit must drain stdout before final telemetry parsing')
assert.match(serverSource, /stderr_pump\.join\(\)/, 'server exit must drain stderr before final telemetry parsing')
assert.doesNotMatch(serverSource, /"metrics-update"/, 'the backend must not emit an event without a frontend consumer')
assert.match(mainSource, /terminate_all_servers_for_exit/, 'application exit must terminate managed server processes')
assert.match(telemetrySource, /TELEMETRY_DROPPED_WRITES\.fetch_add/, 'telemetry queue pressure must be observable')
assert.match(downloadSliceSource, /resumeAllDownloads:[\s\S]*error: undefined[\s\S]*completedAt: undefined/, 'bulk resume must clear stale terminal state')
for (const operation of [
  'download cancellation failed',
  'download pause failed',
  'download resume failed',
  'resume all downloads failed',
  'pause all downloads failed',
  'cancel all downloads failed',
]) {
  assert.match(downloadSliceSource, new RegExp(operation), `${operation} must be visible to the user`)
}
assert.match(
  downloadSliceSource,
  /task\.status !== 'pausing'[\s\S]*status: 'active'/,
  'a rejected pause request must roll an optimistic pausing state back to active',
)
assert.match(clusterPageSource, /labels\.workerLoadFailed/, 'worker load failures must be visible to the user')
for (const operation of [
  'secure Agent test failed',
  'secure Agent stop failed',
  'secure Agent removal failed',
  'secure Agent audit failed',
]) {
  assert.match(clusterPageSource, new RegExp(operation), `${operation} must be visible to the user`)
}
assert.doesNotMatch(
  clusterPageSource,
  /scan_workers_tcp|scan_workers_mdns|connect_worker_ssh|launch_local_rpc|'start_worker_agent'/,
  'the Cluster UI must not restore legacy or fail-closed unauthenticated worker paths',
)
assert.match(clusterPageSource, /<Button disabled variant="success"/, 'Agent compute startup must be disabled in the renderer')
assert.match(
  instanceManagerSource,
  /catch \(error\) \{[\s\S]*setPortStatus\(labels\.portCheckFailed\)[\s\S]*addRuntimeWarning[\s\S]*return/,
  'instance creation must stop and warn when port validation cannot run',
)
assert.match(runtimeEventsSource, /delete lastProgressUpdate\[taskId\]/, 'terminal download events must clear progress throttling state')
const removeManagerFileSource = downloadSource.slice(
  downloadSource.indexOf('fn remove_manager_file'),
  downloadSource.indexOf('fn cleanup_requested'),
)
assert.match(removeManagerFileSource, /update_download_state/, 'paused task cancellation must remove persisted queue state')
assert.match(removeManagerFileSource, /update_inflight_state/, 'paused task cancellation must remove crash-recovery state')
assert.match(
  downloadSource,
  /remove_manager_file\(&state, &task_id\)\?;[\s\S]*emit\("download-removed"/,
  'the backend must emit removal only after durable task cleanup succeeds',
)
const cancelCleanupSource = downloadSource.slice(
  downloadSource.indexOf('pub async fn cancel_and_cleanup_download'),
  downloadSource.indexOf('// HuggingFace data structures and browse.'),
)
assert.doesNotMatch(
  cancelCleanupSource,
  /file_path|frontend_path/,
  'paused task cancellation must derive cleanup paths from registered backend state',
)
assert.doesNotMatch(
  downloadSliceSource,
  /cancel_and_cleanup_download'[^]*filePath/,
  'the frontend must not submit a locally reconstructed path for paused task cancellation',
)
const cancelPersistedSource = downloadManagerSource.slice(
  downloadManagerSource.indexOf('const handleCancelPersisted'),
  downloadManagerSource.indexOf('const taskLocalPath'),
)
assert.doesNotMatch(cancelPersistedSource, /status: 'cancelled'/, 'the frontend must not hide a paused task before backend confirmation')
assert.match(telemetrySource, /completed_at = inference_requests\.completed_at/, 'log replay must preserve the original completion time')
assert.match(downloadBrowseSource, /useAppStore\.setState\(state =>/, 'local file discovery must merge into the latest download state')
assert.match(downloadBrowseSource, /latest\.updatedAt[\s\S]*browseStartedAt/, 'local discovery must not overwrite concurrent progress')
assert.doesNotMatch(bigScreenSource, /const loadInitialData = useAppStore/, 'wallboard must not start a duplicate bootstrap scan')

behaviorTests
  .then(() => console.log('audit remediation regression tests passed'))
  .catch(error => {
    console.error(error)
    process.exitCode = 1
  })
