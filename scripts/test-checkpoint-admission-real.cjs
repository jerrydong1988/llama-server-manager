// Opt-in Windows/ROCm local-qualification campaign; uses a NEW manager data root.
// --bin DIR --root NEW_DIR --engine EXE --model GGUF --draft GGUF [--cases JSON]
const fs = require('node:fs')
const path = require('node:path')
const cp = require('node:child_process')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const { sha256 } = require('./lib/checkpoint-evidence.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + i * 2].slice(2), process.argv[3 + i * 2]]))
assert.equal(process.platform, 'win32')
for (const key of ['bin', 'root', 'engine', 'model', 'draft']) assert.ok(path.isAbsolute(args[key] || ''), `--${key} must be absolute`)
assert.ok(!fs.existsSync(args.root), 'validation root must be new')
fs.mkdirSync(args.root, { recursive: true })
const data = path.join(args.root, 'data')
if (args.fingerprints) {
  const bytes = fs.readFileSync(args.fingerprints); assert.ok(bytes.length < 1024 * 1024); JSON.parse(bytes)
  fs.mkdirSync(path.join(data, 'kv-checkpoints'), { recursive: true })
  fs.writeFileSync(path.join(data, 'kv-checkpoints', 'fingerprints-v1.json'), bytes)
}
const client = managementClient({ cli: path.join(args.bin, 'lsm.exe'), app: path.join(args.bin, 'llama-server-manager.exe'), data, timeout: 1500000 })
const cases = JSON.parse(args.cases || '[{"slots":1,"context":4096,"kv":"f16"},{"slots":2,"context":8192,"kv":"f16"},{"slots":4,"context":16384,"kv":"f16"},{"slots":1,"context":8192,"kv":"q8_0"}]')
const report = { started: new Date().toISOString(), scope: 'Windows/ROCm experimental local checkpoint admission', events: [], manager_sha256: sha256(path.join(args.bin, 'llama-server-manager.exe')) }
const record = (name, details = {}) => {
  report.events.push({ ...details, name, at: new Date().toISOString() })
  fs.writeFileSync(path.join(args.root, 'report.json'), JSON.stringify(report, null, 2))
  console.log(name)
}
function owned() {
  const raw = cp.execFileSync('powershell.exe', ['-NoProfile', '-Command',
    "@(Get-CimInstance Win32_Process | Where-Object { $_.Name -in @('llama-server.exe','llama-server-manager.exe') -and $_.CommandLine.Contains($env:LSM_VALIDATION_ROOT) } | Select-Object ProcessId,ExecutablePath) | ConvertTo-Json -Compress"],
  { windowsHide: true, encoding: 'utf8', env: { ...process.env, LSM_VALIDATION_ROOT: args.root } }).trim()
  const result = raw ? JSON.parse(raw) : []; return Array.isArray(result) ? result : [result]
}
function memorySampler(client) {
  const summary = { scope: 'host-wide sampled maxima, not process GPU peaks or allocation upper bounds', samples: 0, host_ram_mb: 0, host_vram_mb: 0, errors: 0, interval_ms: 2000 }
  let pending
  const timer = setInterval(() => {
    if (pending) return
    pending = client.call('system.health').then(system => {
      summary.samples++
      if (system.system_memory_used_mb != null) summary.host_ram_mb = Math.max(summary.host_ram_mb, system.system_memory_used_mb)
      if (system.vram_used_mb != null) summary.host_vram_mb = Math.max(summary.host_vram_mb, system.vram_used_mb)
    }).catch(() => { summary.errors++ }).finally(() => { pending = undefined })
  }, summary.interval_ms)
  return { summary, stop: async () => { clearInterval(timer); await pending } }
}
let sampler
async function update(id, patch) {
  await client.mutate('instances.update', { id, patch, revision: (await client.call('service.status')).revision })
}
async function main() {
  await client.call('service.start')
  for (const [purpose, directory] of [['engine', path.dirname(args.engine)], ...[...new Set([args.model, args.draft].map(file => path.dirname(file)))].map(directory => ['model', directory])]) {
    await client.mutate('paths.authorize', { purpose, path: directory })
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('models.refresh'); await client.mutate('engines.refresh')
  const norm = file => path.toNamespacedPath(file).toLowerCase()
  const engine = (await client.call('engines.list')).engines.find(engine => norm(engine.exe) === norm(args.engine))
  assert.ok(engine); record('engine-probed', await client.mutate('engines.probe', { id: engine.id }))
  for (const [index, shape] of cases.entries()) {
    assert.ok([1, 2, 4].includes(shape.slots)); assert.ok(['f16', 'q8_0'].includes(shape.kv))
    const id = `admission-${index}`
    const config = { name: `Admission ${shape.slots} x ${shape.context} ${shape.kv}`, engine_id: engine.id,
      model_path: args.model, draft_model_path: shape.spec === 'none' ? '' : args.draft, host: '127.0.0.1', port: 19801 + index,
      launch_mode: 'managed', ctx_size: shape.context * shape.slots, ctx_size_auto: false,
      gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99, flash_attn: 'on', fit_mode: 'off',
      kv_unified: true, kv_unified_mode: 'on', cache_type_k: shape.kv, cache_type_v: shape.kv,
      cache_type_draft_k: shape.kv, cache_type_draft_v: shape.kv, spec_type: shape.spec || 'ngram-mod,draft-mtp',
      draft_tokens: 3, ctx_checkpoints: shape.checkpoints || 2, checkpoint_min_step: shape.min_step || 8192, parallel: shape.slots, cache_prompt: true,
      cache_idle_slots: shape.slots === 1, cache_ram: 256, warmup: false, slots_enabled: true, verbose: true, mmproj_auto: false,
      kv_checkpoint: { enabled: true, allow_local_validation: true, auto_save: true, auto_restore: true, storage_limit_gib: 32, minimum_prompt_tokens: 16 } }
    await client.mutate('instances.create', { id, config, revision: (await client.call('service.status')).revision })
    const before = await client.call('checkpoints.eligibility', { id })
    assert.equal(before.can_validate, true, JSON.stringify(before))
    assert.equal(before.eligible, false, 'choose a shape outside the static reviewed scope')
    assert.equal(before.qualification, 'needs_validation')
    record('unreviewed-candidate-is-not-yet-admitted', { shape, before })
    sampler = memorySampler(client, id)
    const receipt = await client.mutate('checkpoints.validate', { id })
    await sampler.stop(); const memory = sampler.summary; sampler = undefined
    const after = await client.call('checkpoints.eligibility', { id })
    assert.equal(after.eligible, true, JSON.stringify(after)); assert.equal(after.qualification, 'local_validated')
    assert.equal(receipt.old_process_exited, true); assert.equal(receipt.slots.length, shape.slots)
    assert.ok(receipt.slots.every(slot => slot.passed && slot.generated.timings.cache_n > 0))
    record('fresh-process-inference-locally-admitted', { shape, after, receipt, memory })
    await update(id, { ctx_size: config.ctx_size + shape.slots * 1024 })
    const changed = await client.call('checkpoints.eligibility', { id })
    assert.equal(changed.eligible, false); assert.equal(changed.qualification, 'needs_validation')
    await update(id, { ctx_size: config.ctx_size, kv_checkpoint: { allow_local_validation: false } })
    assert.equal((await client.call('checkpoints.eligibility', { id })).eligible, false)
    await update(id, { kv_checkpoint: { allow_local_validation: true } })
    assert.equal((await client.call('checkpoints.eligibility', { id })).eligible, true)
    record('changed-tuple-and-disabled-policy-rejected', { shape })
    if (index === 0) {
      const receiptFile = path.join(data, 'kv-checkpoints', '.qualifications', `${receipt.fingerprint.digest}.json`)
      const receiptBefore = sha256(receiptFile)
      const task = await client.call('checkpoints.validate', { id })
      const startedBy = Date.now() + 60000
      while (!owned().some(p => norm(p.ExecutablePath) === norm(args.engine))) {
        const status = await client.call('tasks.get', { id: task.task_id })
        assert.ok(['queued', 'running'].includes(status.status), JSON.stringify(status))
        assert.ok(Date.now() < startedBy, 'revalidation must start an owned process before cancellation')
        await sleep(500)
      }
      await client.call('tasks.cancel', { id: task.task_id })
      const cancelledBy = Date.now() + 200000
      for (;;) {
        const status = await client.call('tasks.get', { id: task.task_id })
        if (status.status === 'cancelled') break
        assert.equal(status.status, 'running', JSON.stringify(status))
        assert.ok(Date.now() < cancelledBy, 'cancellation must finish after bounded process cleanup')
        await sleep(500)
      }
      assert.equal(sha256(receiptFile), receiptBefore)
      assert.equal((await client.call('checkpoints.eligibility', { id })).eligible, true)
      assert.equal(owned().filter(p => norm(p.ExecutablePath) === norm(args.engine)).length, 0)
      record('cancellation-preserves-receipt-and-reclaims-engine', { shape, task_id: task.task_id })
    }
  }
  report.passed = true
}
main().catch(error => { report.passed = false; report.error = error.stack; process.exitCode = 1; console.error(error) }).finally(async () => {
  await sampler?.stop()
  try { await client.mutate('service.stop') } catch (error) { report.stop_error = error.message }
  await sleep(1500)
  for (const ownedProcess of owned()) {
    report.passed = false; process.exitCode = 1
    report.forced_cleanup = [...(report.forced_cleanup || []), ownedProcess.ProcessId]
    cp.execFileSync('taskkill.exe', ['/PID', String(ownedProcess.ProcessId), '/T', '/F'], { windowsHide: true })
  }
  report.processes_after_cleanup = owned()
  assert.equal(report.processes_after_cleanup.length, 0)
  report.finished = new Date().toISOString(); record('finished')
})
