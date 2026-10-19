// Opt-in real network test. All outputs and configurations are owned by --root.
// --bin DIR --root DIR --engine FILE. Does not touch existing user models.
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + i * 2].slice(2), process.argv[3 + i * 2]]))
for (const field of ['bin', 'root', 'engine']) assert.ok(args[field] && path.isAbsolute(args[field]), `--${field} must be absolute`)
const extension = process.platform === 'win32' ? '.exe' : ''
const root = args.root, data = path.join(root, 'data')
fs.mkdirSync(root, { recursive: true })
const client = managementClient({ cli: path.join(args.bin, `lsm${extension}`), app: path.join(args.bin, `llama-server-manager${extension}`), data })
const report = { started_at: new Date().toISOString(), kind: 'real-Hugging-Face-CLI-MCP', checks: [], files: [] }
const reportPath = path.join(root, 'download-cli-mcp.json')
const record = (name, details = {}) => { report.checks.push({ name, ...details }); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2)); console.log(JSON.stringify({ name, ...details })) }
const source = 'huggingface', repo = 'ggml-org/models'
const downloadDir = path.join(root, `download-${Date.now()}`)
fs.mkdirSync(downloadDir)
let mcp, started = false
const snapshot = () => client.call('downloads.list')
const files = s => s.queue.flatMap(entry => entry.files)
async function until(label, predicate, timeout = 120000) {
  const end = Date.now() + timeout
  let s
  while (Date.now() < end) { s = await snapshot(); if (predicate(s)) return s; await sleep(250) }
  throw new Error(`${label}: ${JSON.stringify(s)}`)
}
async function main() {
  await client.call('service.start')
  started = true
  await client.mutate('paths.authorize', { purpose: 'download', path: downloadDir })
  await client.mutate('paths.authorize', { purpose: 'model', path: downloadDir })
  await client.mutate('paths.authorize', { purpose: 'engine', path: path.dirname(args.engine) })
  for (const [purpose, directory] of [['model', downloadDir], ['engine', path.dirname(args.engine)]]) {
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('engines.refresh')
  const engine = (await client.call('engines.list')).engines.find(e => path.basename(e.exe).toLowerCase() === path.basename(args.engine).toLowerCase())
  assert.ok(engine)
  await client.mutate('engines.probe', { id: engine.id })
  mcp = await client.mcp()
  const catalog = await client.call('downloads.browse', { source, repo })
  const remote = 'tinyllamas/stories15M-q4_0.gguf'
  const tiny = 'tinyllamas/stories260K.gguf'
  assert.ok(catalog.some(f => f.path === remote) && catalog.some(f => f.path === tiny))
  assert.deepEqual((await mcp.tool('downloads.browse', { source, repo })).map(f => f.path), catalog.map(f => f.path))
  record('repository-catalog-equal', { files: catalog.length })
  await client.mutate('downloads.settings', { concurrency: 1, bytes_per_second: 65536, low_priority: false, resume_policy: 'manual' })
  const batch = await mcp.mutate('downloads.add', { source, repo, files: [remote, tiny], directory: downloadDir })
  const first = batch.files[0].task_id
  const s = await until('partial progress', s => files(s).some(f => f.task_id === first && f.downloaded > 65536 && f.status === 'active'))
  assert.ok(files(s).filter(f => f.status === 'active').length <= 1)
  // The scheduler may finish the small file first; array order is not a queue guarantee.
  // Add a separate large transfer only after the first one is observed active.
  const queuedDir = path.join(downloadDir, 'queued-guard')
  fs.mkdirSync(queuedDir)
  const queued = await mcp.mutate('downloads.add', { source, repo, files: [remote], directory: queuedDir })
  const queuedId = queued.files[0].task_id
  await until('separate transfer queued', s => files(s).some(f => f.task_id === queuedId && f.status === 'queued'))
  for (const id of [first, queuedId]) {
    const blocked = await mcp.mutate('downloads.delete_file', { id }, true)
    assert.equal(blocked.status, 'failed')
    assert.match(blocked.error.message, /Pause or cancel/)
  }
  await client.mutate('downloads.cancel', { id: queued.download_id, file: remote })
  await until('queued cancellation acknowledged', s => !files(s).some(f => f.task_id === queuedId && ['active', 'pausing', 'queued'].includes(f.status)))
  record('active-and-queued-output-deletion-refused')
  await mcp.mutate('downloads.pause', { id: batch.download_id, file: remote })
  const paused = await until('pause acknowledgement', s => files(s).some(f => f.task_id === first && f.status === 'paused'))
  const progress = files(paused).find(f => f.task_id === first).downloaded
  await sleep(600)
  assert.equal(files(await snapshot()).find(f => f.task_id === first).downloaded, progress)
  record('multi-file-pause-and-limit', { task_id: first, downloaded: progress })
  await mcp.mutate('downloads.pause_all')
  await until('all paused', s => files(s).every(f => !['active', 'pausing', 'queued'].includes(f.status)))
  await mcp.close(); mcp = null
  const pid = (await client.call('service.status')).pid
  await client.mutate('service.stop'); await sleep(3000)
  const restarted = await client.call('service.start')
  assert.notEqual(restarted.pid, pid)
  const restored = files(await snapshot()).find(f => f.task_id === first)
  assert.equal(restored.status, 'paused'); assert.ok(restored.downloaded >= progress)
  record('host-restart-preserves-resumable-progress', { old_pid: pid, new_pid: restarted.pid, downloaded: restored.downloaded })
  mcp = await client.mcp()
  await mcp.mutate('downloads.settings', { concurrency: 2, bytes_per_second: 0 })
  await client.mutate('downloads.resume_all')
  await until('resume to completed', s => batch.files.every(f => files(s).some(current => current.task_id === f.task_id && current.status === 'completed')), 180000)
  const local = remotePath => path.join(downloadDir, repo, remotePath)
  for (const item of batch.files) {
    const file = local(item.file)
    const stat = fs.statSync(file)
    assert.equal(stat.size, catalog.find(f => f.path === item.file).size)
    report.files.push({ remote: item.file, path: file, size: stat.size, sha256: crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex'), task_id: item.task_id })
  }
  record('resumed-files-complete-and-sized', { files: report.files })
  // A foreign destination causes a recoverable error; retry must not overwrite it.
  const retryRemote = 'tinyllamas/stories260K-infill.gguf'
  const retryPath = local(retryRemote)
  fs.mkdirSync(path.dirname(retryPath), { recursive: true }); fs.writeFileSync(retryPath, 'foreign-owned-sentinel')
  const retry = await client.mutate('downloads.add', { source, repo, files: [retryRemote], directory: downloadDir })
  const retryId = retry.files[0].task_id
  await until('foreign destination rejected', s => files(s).some(f => f.task_id === retryId && f.status === 'error'))
  assert.equal(fs.readFileSync(retryPath, 'utf8'), 'foreign-owned-sentinel')
  fs.unlinkSync(retryPath) // only the sentinel created above
  await mcp.mutate('downloads.retry', { id: retryId })
  await until('retry completed', s => files(s).some(f => f.task_id === retryId && f.status === 'completed'))
  record('foreign-file-preserved-and-error-retry-completes')
  // Cancellation removes owned partials, preserves unrelated files.
  await client.mutate('downloads.settings', { concurrency: 1, bytes_per_second: 65536 })
  const cancelRemote = 'tinyllamas/stories15M-q8_0.gguf'
  const cancel = await mcp.mutate('downloads.add', { source, repo, files: [cancelRemote], directory: downloadDir })
  const cancelId = cancel.files[0].task_id
  await until('cancel partial progress', s => files(s).some(f => f.task_id === cancelId && f.downloaded > 0 && f.status === 'active'))
  const sentinel = path.join(path.dirname(local(cancelRemote)), 'preserve.txt'); fs.writeFileSync(sentinel, 'preserve')
  await client.mutate('downloads.cancel', { id: cancel.download_id, file: cancelRemote })
  await until('cancel acknowledgement', s => !files(s).some(f => f.task_id === cancelId && ['active', 'pausing', 'queued'].includes(f.status)))
  await sleep(500)
  assert.equal(fs.existsSync(local(cancelRemote)), false)
  assert.equal(fs.readFileSync(sentinel, 'utf8'), 'preserve')
  assert.ok(!fs.readdirSync(path.dirname(sentinel)).some(name => name.startsWith(path.basename(cancelRemote)) && name.includes('.part')))
  record('cancel-cleans-owned-partials-only')
  // Actual multi-file GGUF: every shard is protected by an instance naming shard one.
  await mcp.mutate('downloads.settings', { concurrency: 3, bytes_per_second: 0 })
  const shards = [1, 2, 3].map(i => `tinyllamas/split/stories15M-q8_0-0000${i}-of-00003.gguf`)
  const split = await client.mutate('downloads.add', { source, repo, files: shards, directory: downloadDir })
  await until('shards completed', s => split.files.every(f => files(s).some(current => current.task_id === f.task_id && current.status === 'completed')), 180000)
  await client.mutate('models.refresh')
  const id = 'download-reference-validation'
  await client.mutate('instances.create', { id, revision: (await client.call('service.status')).revision, config: { name: 'Download reference test', engine_id: engine.id, model_path: local(shards[0]), port: 18186, ctx_size: 512, ctx_size_auto: false, gpu_layers: 0, gpu_layers_auto: false, fit_mode: 'off', warmup: false } })
  const preview = await mcp.tool('models.delete_preview', { path: local(shards[0]) })
  assert.equal(preview.artifactCount, 3); assert.equal(preview.referencedBy.length, 1)
  const rejected = await mcp.mutate('downloads.delete_file', { id: split.files[1].task_id }, true)
  assert.equal(rejected.status, 'failed')
  assert.ok(split.files.every(f => fs.existsSync(local(f.file))))
  const libraryRejected = await client.call('models.delete', { path: local(shards[0]) }, { mutate: true, allowFailure: true })
  assert.equal(libraryRejected.ok, false)
  record('shard-reference-protected-through-download-and-model-interfaces', { artifacts: preview.artifactCount, error: rejected.error })
  await mcp.mutate('instances.delete', { id, revision: (await client.call('service.status')).revision })
  const deleted = await client.mutate('models.delete', { path: local(shards[0]) })
  assert.equal(deleted.artifactCount, 3); assert.ok(shards.every(f => !fs.existsSync(local(f))))
  record('unreferenced-shard-group-deleted-together')
  await mcp.mutate('downloads.delete_file', { id: retryId })
  assert.equal(fs.existsSync(retryPath), false)
  await client.mutate('downloads.clear', { statuses: ['completed', 'cancelled', 'error'] })
  assert.ok(report.files.every(f => fs.existsSync(f.path)), 'clearing history must preserve completed files')
  record('owned-file-delete-and-history-clear-have-distinct-effects')
  report.status = 'passed'
}
main().catch(error => { report.status = 'failed'; report.error = error.stack; console.error(error); process.exitCode = 1 })
  .finally(async () => {
    if (mcp) await mcp.close()
    if (started) {
      try { await client.mutate('downloads.pause_all'); await client.mutate('service.stop') }
      catch (error) { report.cleanup_error = error.message; process.exitCode = 1 }
    }
    report.finished_at = new Date().toISOString(); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2))
  })
