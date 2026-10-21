// Opt-in Windows/ROCm integration campaign. Never uses the default manager root.
// --bin DIR --root NEW_DIR --engine EXE --model GGUF --mtp GGUF [--dflash GGUF]
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const cp = require('node:child_process')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + 2 * i].slice(2), process.argv[3 + 2 * i]]))
assert.equal(process.platform, 'win32')
for (const key of ['bin', 'root', 'engine', 'model', 'mtp']) assert.ok(path.isAbsolute(args[key] || ''), `--${key} must be absolute`)
for (const key of ['engine', 'model', 'mtp', ...(args.dflash ? ['dflash'] : [])]) assert.ok(fs.statSync(args[key]).isFile(), `missing ${key}`)
assert.ok(!fs.existsSync(args.root), '--root must be new; never reuse user data')
fs.mkdirSync(args.root, { recursive: true })
fs.writeFileSync(path.join(args.root, '.owned-validation'), 'manager-cache-lifecycle-v1')
const data = path.join(args.root, 'data'), id = 'b11769-cache-test', port = 18771
if (args.fingerprints) {
  assert.ok(path.isAbsolute(args.fingerprints))
  const bytes = fs.readFileSync(args.fingerprints)
  assert.ok(bytes.length < 1024 * 1024)
  JSON.parse(bytes)
  fs.mkdirSync(path.join(data, 'kv-checkpoints'), { recursive: true })
  fs.writeFileSync(path.join(data, 'kv-checkpoints', 'fingerprints-v1.json'), bytes)
}
const client = managementClient({ cli: path.join(args.bin, 'lsm.exe'), app: path.join(args.bin, 'llama-server-manager.exe'), data, timeout: 600000 })
const report = { started: new Date().toISOString(), engine: args.engine, scope: args['boundary-only'] ? 'zero-checkpoint-boundary' : 'full', fingerprint_cache_seed: args.fingerprints || null, events: [], owned_pids: [] }
const sha = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
const record = (name, details = {}) => {
  if (name.startsWith('fresh-process-')) fs.copyFileSync(path.join(data, 'configs', 'logs', `${id}.log`), path.join(args.root, `${name}.log`))
  report.events.push({ ...details, name, recorded_at: new Date().toISOString() })
  fs.writeFileSync(path.join(args.root, 'report.json'), JSON.stringify(report, null, 2)); console.log(name)
}
const walk = dir => fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)]) : []
const lookupRoot = path.join(data, 'lookup-cache', id)
const latest = () => walk(lookupRoot).find(f => path.basename(f) === 'latest.json')
const results = () => walk(lookupRoot).filter(f => path.basename(f) === 'last-result.json').map(f => JSON.parse(fs.readFileSync(f)))
function ownedProcesses() {
  const output = cp.execFileSync('powershell.exe', ['-NoProfile', '-Command',
    "@(Get-CimInstance Win32_Process -Filter \"Name = 'llama-server-manager.exe' OR Name = 'llama-server.exe'\" | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:LSM_VALIDATION_ROOT) } | Select-Object ProcessId,ExecutablePath,CommandLine) | ConvertTo-Json -Compress"],
  { windowsHide: true, encoding: 'utf8', env: { ...process.env, LSM_VALIDATION_ROOT: args.root } }).trim()
  const parsed = output ? JSON.parse(output) : []
  return (Array.isArray(parsed) ? parsed : [parsed]).filter(p =>
    [args.engine, path.join(args.bin, 'llama-server-manager.exe')].some(exe => exe.toLowerCase() === p.ExecutablePath?.toLowerCase()))
}
async function shutdownService() {
  await client.mutate('service.stop')
  const deadline = Date.now() + 45000
  while (ownedProcesses().length && Date.now() < deadline) await sleep(1000)
  assert.deepEqual(ownedProcesses(), [], 'owned engine and controller processes must exit without respawning')
}
async function request(endpoint, body) {
  const response = await fetch(`http://127.0.0.1:${port}${endpoint}`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body), signal: AbortSignal.timeout(180000) })
  const json = await response.json(); assert.equal(response.status, 200, JSON.stringify(json)); return json
}
async function completion(prompt, count = 48) {
  return request('/completion', { prompt, n_predict: count, temperature: 0, seed: 42, cache_prompt: true, id_slot: 0, return_tokens: true, ignore_eos: true })
}
async function update(patch) {
  const old = await client.call('instances.get', { id }, { allowFailure: true })
  await client.mutate(old.ok ? 'instances.update' : 'instances.create', { id, revision: (await client.call('service.status')).revision, [old.ok ? 'patch' : 'config']: patch })
}
async function start() {
  await client.mutate('instances.start', { id })
  const until = Date.now() + 240000
  for (;;) {
    const h = await client.call('instances.health', { id })
    const c = await client.call('checkpoints.status', { id })
    if (h.running && c.routable) {
      try { if ((await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) })).ok) {
        assert.equal(h.running.private_console, true)
        report.owned_pids.push(h.running.pid)
        return { pid: h.running.pid, checkpoint: c }
      } } catch {}
    }
    if (Date.now() > until) throw Error(`not ready: ${JSON.stringify({ h, c })}`)
    await sleep(500)
  }
}
async function stop() {
  const running = (await client.call('instances.health', { id })).running
  const result = await client.mutate('instances.stop', { id })
  assert.equal((await client.call('instances.health', { id })).running, null)
  if (running) {
    assert.ok(!ownedProcesses().some(p => p.ProcessId === running.pid), 'old engine PID must exit before a new process is launched')
    report.stop_verifications = [...(report.stop_verifications || []), { pid: running.pid, exited: true, checked_at: new Date().toISOString() }]
  }
  return result
}
function killOwned(pid) {
  assert.ok(report.owned_pids.includes(pid))
  assert.ok(ownedProcesses().some(p => p.ProcessId === pid), 'refuse to terminate a PID outside this validation root')
  cp.execFileSync('taskkill.exe', ['/PID', String(pid), '/T', '/F'], { windowsHide: true })
}
const basePrompt = 'Write the result of 17 times 23 as 391, then repeat 391 separated by spaces. Answer: 391 391 391 '
async function main() {
  assert.equal(sha(args.engine), '5c081b5db3fc7d62ba11f7b79253df7edf39975835fa8e9da5932301723410dd')
  await client.call('service.start')
  for (const [purpose, directory] of [['engine', path.dirname(args.engine)], ...[...new Set([args.model, args.mtp, args.dflash].filter(Boolean).map(p => path.dirname(p)))].map(p => ['model', p])]) {
    await client.mutate('paths.authorize', { purpose, path: directory })
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('models.refresh'); await client.mutate('engines.refresh')
  const normalizePath = p => p.replace(/^\\\\\?\\/, '').toLowerCase()
  const engine = (await client.call('engines.list')).engines.find(e => normalizePath(e.exe) === normalizePath(args.engine))
  assert.ok(engine)
  const probed = await client.mutate('engines.probe', { id: engine.id })
  record('capability-probe', probed)
  assert.equal(probed.capabilities.contextCheckpointPersistence, true)
  assert.equal(probed.capabilities.persistenceProfile, 'windows-rocm-b11769-sckp1-sdft1')
  assert.ok(!probed.capabilities.checkpointDraftTypes.includes('draft-eagle3'))
  const seed = path.join(args.root, 'external-seed.bin')
  await update({ name: 'Isolated b11769 cache validation', engine_id: engine.id, model_path: args.model,
    host: '127.0.0.1', port, launch_mode: 'managed', ctx_size: 4096, ctx_size_auto: false,
    gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99, draft_tokens: 3, flash_attn: 'on', fit_mode: 'off',
    warmup: false, parallel: 1, cache_prompt: true, cache_idle_slots: true, cache_ram: 256, ctx_checkpoints: 2,
    slots_enabled: true, metrics: true, props: true, mmproj_auto: false,
    spec_type: 'ngram-cache', lookup_cache_dynamic: seed, lookup_cache_managed: true,
    kv_checkpoint: { enabled: false, auto_save: true, auto_restore: true, storage_limit_gib: 4, minimum_prompt_tokens: 16 } })
  if (!args['boundary-only']) {
    let started = await start(); const generated = await completion(basePrompt, 80)
    const gpu = (await client.call('instances.health', { id })).monitoring
    assert.ok(generated.content.includes('391'))
    const stopped = await stop()
    assert.ok(latest(), JSON.stringify(results()))
    assert.equal(results()[0].outcome, 'candidate_committed_flush_unverified')
    assert.equal(fs.existsSync(seed), false)
    record('windows-graceful-stop-publishes-private-candidate', { started, stopped, gpu, timings: generated.timings, results: results() })
    const oldManifest = sha(latest())
    started = await start(); await completion(basePrompt + '391 391 ', 64)
    // Denied native write must not be reported as a verified flush or replace old bytes.
    const work = walk(lookupRoot).find(f => path.basename(f).startsWith('work-') && f.endsWith('.bin'))
    assert.ok(work); fs.chmodSync(work, 0o444)
    try { await stop() } finally { if (fs.existsSync(work)) fs.chmodSync(work, 0o666) }
    assert.equal(sha(latest()), oldManifest)
    assert.equal(results()[0].full_flush_verified, false)
    record('denied-write-retains-old-generation', { results: results() })
    started = await start(); await completion(basePrompt + '391 ', 64); killOwned(started.pid)
    await sleep(1200); await stop()
    assert.equal(sha(latest()), oldManifest)
    record('forced-exit-retains-old-generation', { pid: started.pid, results: results() })
    started = await start(); await completion(basePrompt + '391 391 391 ', 64)
    await shutdownService()
    assert.equal(results()[0].clean_exit, true)
    record('background-service-shutdown-flushes-private-copy', { pid: started.pid, results: results() })
    await client.call('service.start')
  }

  const cases = [...(args['boundary-only'] ? [] : [['mtp', args.mtp, 2], ...(args.dflash ? [['dflash', args.dflash, 2]] : []), ['simple', args.model, 2]]),
    ['mtp', args.mtp, 0], ...(args.dflash ? [['dflash', args.dflash, 0]] : [])]
  for (const [mode, draft, checkpoints] of cases) {
    await update({ spec_type: `draft-${mode}`, draft_model_path: draft, lookup_cache_dynamic: '', lookup_cache_managed: false,
      ctx_checkpoints: checkpoints,
      kv_checkpoint: { enabled: true, auto_save: true, auto_restore: true, storage_limit_gib: 4, minimum_prompt_tokens: 16 } })
    const eligibility = await client.call('checkpoints.eligibility', { id })
    if (checkpoints === 0) {
      assert.equal(eligibility.eligible, false, JSON.stringify(eligibility))
      assert.ok(eligibility.reasons.includes('context_checkpoints_required'))
      record('zero-checkpoint-automatic-policy-blocked', { mode, eligibility })
      const nativePath = path.join(args.root, 'native-slot-boundary')
      fs.mkdirSync(nativePath, { recursive: true })
      await update({ slot_save_path: nativePath, kv_checkpoint: { enabled: false } })
    } else assert.equal(eligibility.eligible, true, JSON.stringify(eligibility))
    await client.mutate('checkpoints.clear', { id })
    const original = await start(); const prefill = await completion(basePrompt, 24)
    const filename = `reference-${mode}.bin`
    await request('/slots/0?action=save', { filename })
    const continuationPrompt = basePrompt + prefill.content
    const live = await completion(continuationPrompt)
    await request('/slots/0?action=restore', { filename })
    await stop()
    const saved = await client.call('checkpoints.status', { id })
    if (checkpoints !== 0) assert.equal(saved.last_outcome, 'success', JSON.stringify(saved))
    const fresh = await start(); assert.notEqual(fresh.pid, original.pid)
    if (checkpoints === 0) await request('/slots/0?action=restore', { filename })
    else {
      assert.equal(fresh.checkpoint.last_operation, 'restore')
      assert.equal(fresh.checkpoint.last_outcome, 'success', JSON.stringify(fresh.checkpoint))
    }
    const restored = await completion(continuationPrompt)
    assert.ok(Array.isArray(restored.tokens) && restored.tokens.length > 0)
    assert.ok(restored.content.includes('391'))
    assert.ok(restored.timings.draft_n > 0 && restored.timings.draft_n_accepted > 0)
    assert.deepEqual(restored.tokens, live.tokens)
    assert.equal(restored.content, live.content)
    assert.equal(restored.timings.draft_n_accepted, live.timings.draft_n_accepted)
    assert.equal(restored.timings.draft_n, live.timings.draft_n)
    assert.ok(restored.timings.cache_n > 0)
    record(`fresh-process-${mode}-cp${checkpoints}-restore-and-inference`, { restore_kind: checkpoints === 0 ? 'native-api-only-automatic-policy-blocked' : 'manager-automatic', original, fresh, saved, live, restored,
      monitoring: (await client.call('instances.health', { id })).monitoring })
    await stop()
  }
  await update({ ctx_checkpoints: 2, slot_save_path: '', kv_checkpoint: { enabled: true } })
  for (const patch of [{ cache_type_draft_k: 'q8_0' }, { cache_type_draft_k: '', ctx_size: 8192 },
    { ctx_size: 4096, spec_type: 'draft-eagle3' }]) {
    await update(patch)
    const eligibility = await client.call('checkpoints.eligibility', { id })
    assert.equal(eligibility.eligible, false, JSON.stringify(eligibility))
    assert.notEqual(eligibility.reason_code, 'disabled')
    record('unqualified-draft-configuration-blocked', { patch, eligibility })
  }
  report.passed = true
}
main().catch(e => { report.error = e.stack; process.exitCode = 1; console.error(e) }).finally(async () => {
  try { await stop() } catch (e) { report.cleanup_stop_error = e.message }
  try { await shutdownService() } catch (e) { report.cleanup_service_error = e.message; report.passed = false; process.exitCode = 1 }
  for (const process of ownedProcesses()) {
    if (!ownedProcesses().some(p => p.ProcessId === process.ProcessId)) continue
    report.passed = false; global.process.exitCode = 1
    report.forced_cleanup_pids = [...(report.forced_cleanup_pids || []), process.ProcessId]
    try { cp.execFileSync('taskkill.exe', ['/PID', String(process.ProcessId), '/T', '/F'], { windowsHide: true }) }
    catch (e) { if (ownedProcesses().some(p => p.ProcessId === process.ProcessId)) report.cleanup_service_error = e.message }
  }
  report.finished = new Date().toISOString(); record('finished')
})
