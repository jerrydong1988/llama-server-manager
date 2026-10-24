// Opt-in Windows/ROCm integration campaign. Never uses the default manager root.
// --bin DIR --root NEW_DIR --engine EXE --model GGUF --mtp GGUF [--dflash GGUF]
const fs = require('node:fs')
const path = require('node:path')
const crypto = require('node:crypto')
const cp = require('node:child_process')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const { inspectSlotFile, assertInference, memorySampler, processMemory } = require('./lib/checkpoint-evidence.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + 2 * i].slice(2), process.argv[3 + 2 * i]]))
assert.equal(process.platform, 'win32')
for (const key of ['bin', 'root', 'engine', 'model', 'mtp']) assert.ok(path.isAbsolute(args[key] || ''), `--${key} must be absolute`)
for (const key of ['bin', 'root', 'engine', 'model', 'mtp', ...(args.dflash ? ['dflash'] : [])]) args[key] = path.resolve(args[key])
const context = Number(args.context || 4096), kv = args.kv || 'f16', records = Number(args.records || 100)
assert.ok([4096, 16384].includes(context) && ['f16', 'q8_0'].includes(kv))
assert.ok(Number.isInteger(records) && records >= 100 && records <= 640)
assert.ok((!args.context && !args.kv && !args.records) || args['failure-only'], 'extended shapes require --failure-only true')
assert.ok(!args['scope-checks'] || args['failure-only'], '--scope-checks requires --failure-only true')
for (const key of ['engine', 'model', 'mtp', ...(args.dflash ? ['dflash'] : [])]) assert.ok(fs.statSync(args[key]).isFile(), `missing ${key}`)
assert.ok(!fs.existsSync(args.root), '--root must be new; never reuse user data')
fs.mkdirSync(args.root, { recursive: true })
fs.writeFileSync(path.join(args.root, '.owned-validation'), 'manager-cache-lifecycle-v1')
const profiles = require('../src-tauri/src/commands/persistence_profiles.json')
const profile = profiles.find(p => p.id === args.profile)
assert.ok(profile, '--profile must name an embedded, immutable qualification candidate')
const data = path.join(args.root, 'data'), id = 'checkpoint-acceptance', port = Number(args.port || 18771)
assert.ok(Number.isInteger(port) && port > 1024 && port < 65536)
if (args.fingerprints) {
  assert.ok(path.isAbsolute(args.fingerprints))
  const bytes = fs.readFileSync(args.fingerprints)
  assert.ok(bytes.length < 1024 * 1024)
  JSON.parse(bytes)
  fs.mkdirSync(path.join(data, 'kv-checkpoints'), { recursive: true })
  fs.writeFileSync(path.join(data, 'kv-checkpoints', 'fingerprints-v1.json'), bytes)
}
const client = managementClient({ cli: path.join(args.bin, 'lsm.exe'), app: path.join(args.bin, 'llama-server-manager.exe'), data, timeout: 600000 })
let sampler
const report = { started: new Date().toISOString(), engine: args.engine, scope: args['failure-only'] ? 'native-failure-isolation' : args['idle-only'] ? 'idle-long-prefix' : args['boundary-only'] ? 'zero-checkpoint-boundary' : 'full', fingerprint_cache_seed: args.fingerprints || null, events: [], owned_pids: [] }
const sha = file => {
  const hash = crypto.createHash('sha256'), buffer = Buffer.alloc(1024 * 1024)
  const fd = fs.openSync(file, 'r')
  try {
    for (;;) {
      const count = fs.readSync(fd, buffer, 0, buffer.length, null)
      if (!count) return hash.digest('hex')
      hash.update(buffer.subarray(0, count))
    }
  } finally { fs.closeSync(fd) }
}
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
  const observed = []
  for (;;) {
    const h = await client.call('instances.health', { id })
    const c = await client.call('checkpoints.status', { id })
    const sameProcess = h.running && (!c.expected_pid || c.expected_pid === h.running.pid)
    if (sameProcess) {
      const previous = observed.find(p => p.pid === h.running.pid)
      if (previous) previous.ever_routable ||= c.routable
      else {
        observed.push({ pid: h.running.pid, checkpoint: c, ever_routable: c.routable })
        report.owned_pids.push(h.running.pid)
      }
    }
    if (sameProcess && c.routable) {
      try { if ((await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) })).ok) {
        assert.equal(h.running.private_console, true)
        const log = fs.readFileSync(path.join(data, 'configs', 'logs', `${id}.log`), 'utf8')
        assert.match(log, /ROCm|HIP/)
        const offloads = [...log.matchAll(/offloaded (\d+)\/(\d+) layers to GPU/g)]
        assert.ok(offloads.length && offloads.every(m => Number(m[1]) > 0), 'real GPU offload is required')
        if (args['scope-checks']) {
          assert.ok(offloads.every(m => m[1] === m[2]), 'reviewed scope requires complete GPU offload')
          const types = [...log.matchAll(/K \((\w+)\):[^\n]*V \((\w+)\):/g)]
          assert.ok(types.some(m => m[1] === kv && m[2] === kv), 'requested target KV types not observed')
        }
        report.owned_pids.push(h.running.pid)
        return { pid: h.running.pid, checkpoint: c, observed, gpu_offloads: offloads.map(m => m[0]) }
      } } catch (error) { if (error.code === 'ERR_ASSERTION') throw error }
    }
    if (Date.now() > until) throw Error(`not ready: ${JSON.stringify({ h, c })}`)
    await sleep(500)
  }
}
async function stop() {
  const running = (await client.call('instances.health', { id })).running
  if (running && ownedProcesses().some(p => p.ProcessId === running.pid)) {
    report.process_memory = [...(report.process_memory || []), processMemory(running.pid)]
  }
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
async function failureAcceptance() {
  await update({ spec_type: 'draft-mtp', draft_model_path: args.mtp,
    lookup_cache_dynamic: '', lookup_cache_managed: false,
    kv_checkpoint: { enabled: true, auto_save: true, auto_restore: true,
      idle_save_seconds: 0, preserve_longer: false, storage_limit_gib: 4, minimum_prompt_tokens: 16 } })
  const eligibility = await client.call('checkpoints.eligibility', { id })
  assert.equal(eligibility.eligible, true, JSON.stringify(eligibility))
  record('reviewed-model-scope-eligible', { eligibility, context, kv, records })
  await client.mutate('checkpoints.clear', { id })
  const original = await start()
  const reference = await completion(basePrompt)
  const prompt = Array.from({ length: records }, (_, i) => `Record ${i}: seventeen times twenty-three equals 391.\n`).join('') + basePrompt
  const prefill = await completion(prompt, 24)
  await request('/slots/0?action=save', { filename: 'live-reference.bin' })
  const continuation = prompt + prefill.content, live = await completion(continuation)
  await request('/slots/0?action=restore', { filename: 'live-reference.bin' })
  await stop()
  const pointerFile = walk(path.join(data, 'kv-checkpoints', id)).find(f => path.basename(f) === 'latest.json')
  assert.ok(pointerFile)
  const pointerBytes = fs.readFileSync(pointerFile), pointer = JSON.parse(pointerBytes)
  const generation = path.join(path.dirname(pointerFile), 'generations', pointer.generationId)
  const manifestFile = path.join(generation, 'manifest.json'), manifest = JSON.parse(fs.readFileSync(manifestFile))
  const goodFile = path.join(generation, manifest.slots[0].filename), format = inspectSlotFile(goodFile)
  // GGSQ stores a serialized server_tokens vector, including metadata words.
  // Reuse must cover the logical n_saved count captured in the manifest.
  const savedTokens = manifest.slots[0].promptTokens
  assert.ok(Number.isInteger(savedTokens) && savedTokens > 0)
  const originals = [goodFile, manifestFile].map(file => [file, sha(file)])
  assert.ok(format.liveDraft?.state.bytes > 0 && format.checkpoints.length > 0)
  // Preserve the successful reference. Only disposable generations are edited;
  // updating their hashes intentionally reaches native validation, not the store's
  // earlier checksum gate. Never use this injection against production data.
  await update({ kv_checkpoint: { auto_save: false } })
  const fresh = await start(), recovered = await completion(continuation)
  assert.notEqual(fresh.pid, original.pid)
  assert.equal(fresh.checkpoint.last_outcome, 'success')
  assertInference(live, recovered, { draft: true, reuse: true, minimumReuse: savedTokens })
  for (const [file, digest] of originals) assert.equal(sha(file), digest)
  record('fresh-process-automatic-stop-save-restored', { original, fresh, reference: live, recovered, format, savedTokens, context, kv, records })
  await stop()
  for (const kind of ['invalid-draft-version', 'truncated-main']) {
    const badId = crypto.randomUUID(), badDir = path.join(path.dirname(generation), badId)
    fs.mkdirSync(badDir)
    const badFile = path.join(badDir, manifest.slots[0].filename)
    fs.copyFileSync(goodFile, badFile)
    if (kind === 'truncated-main') fs.truncateSync(badFile, Math.floor(format.mainBytes / 2))
    else {
      const fd = fs.openSync(badFile, 'r+'), bytes = Buffer.alloc(4); bytes.writeUInt32LE(999)
      try { fs.writeSync(fd, bytes, 0, 4, format.liveDraft.offset + 4); fs.fsyncSync(fd) } finally { fs.closeSync(fd) }
    }
    const badManifest = structuredClone(manifest)
    badManifest.generationId = badId; badManifest.createdAt = new Date().toISOString()
    badManifest.slots[0].bytes = fs.statSync(badFile).size; badManifest.slots[0].sha256 = sha(badFile)
    const badManifestFile = path.join(badDir, 'manifest.json')
    fs.writeFileSync(badManifestFile, JSON.stringify(badManifest))
    fs.writeFileSync(pointerFile, JSON.stringify({ ...pointer, generationId: badId, manifestSha256: sha(badManifestFile) }))
    const recovered = await start()
    assert.equal(recovered.checkpoint.phase, 'ready_cold')
    assert.equal(recovered.checkpoint.reason_code, 'cold_after_restore_failure')
    const rejected = recovered.observed.filter(p => p.pid !== recovered.pid)
    assert.ok(rejected.length > 0, 'must observe the failed native PID, not infer restart from health')
    for (const p of rejected) {
      assert.equal(p.ever_routable, false, 'failed restore PID became routable')
      assert.ok(!ownedProcesses().some(q => q.ProcessId === p.pid), 'failed native PID survived')
    }
    assert.ok(fs.existsSync(path.join(badDir, 'restore-rejected.json')))
    const actual = await completion(basePrompt)
    assertInference(reference, actual, { draft: true })
    const desired = JSON.parse(fs.readFileSync(path.join(data, 'runtime', 'runtime-state.json')))
    assert.equal(desired.desired_instances[id].config.kv_checkpoint.auto_restore, true)
    for (const [file, digest] of originals) assert.equal(sha(file), digest)
    record('failed-native-restore-restarted-before-routing', { kind, original, recovered, reference, actual, format })
    await stop()
    fs.writeFileSync(pointerFile, pointerBytes)
    const restoredProcess = await start(), restored = await completion(continuation)
    assert.equal(restoredProcess.checkpoint.last_outcome, 'success')
    assertInference(live, restored, { draft: true, reuse: true, minimumReuse: savedTokens })
    record('fresh-process-previous-good-generation-after-failure', { kind, restoredProcess, restored })
    await stop()
  }
}
async function idleAcceptance(mode) {
  await update({ spec_type: mode === 'mtp' ? 'draft-mtp' : 'none', draft_model_path: mode === 'mtp' ? args.mtp : '',
    lookup_cache_dynamic: '', lookup_cache_managed: false,
    kv_checkpoint: { enabled: true, auto_save: true, auto_restore: true,
      idle_save_seconds: 5, preserve_longer: true, storage_limit_gib: 4, minimum_prompt_tokens: 16 } })
  await client.mutate('checkpoints.clear', { id })
  const original = await start()
  const longPrompt = 'Reference facts for a repetition task:\n' +
    Array.from({ length: 100 }, (_, i) => `Record ${i}: seventeen times twenty-three equals 391.\n`).join('') + basePrompt
  const prefill = await completion(longPrompt, 64)
  const deadline = Date.now() + 240000
  let saved
  for (;;) {
    saved = await client.call('checkpoints.status', { id })
    if (saved.last_operation === 'save' && saved.last_outcome === 'success' && saved.routable) break
    assert.ok(Date.now() < deadline, `idle save did not finish: ${JSON.stringify(saved)}`)
    await sleep(1000)
  }
  assert.equal((await client.call('instances.health', { id })).running.pid, original.pid)
  assert.equal(saved.phase, 'ready_cold', 'saving must not invent a disk restore')
  assert.ok(saved.prompt_tokens > 1000)
  record('idle-save-without-process-restart', { mode, original, saved, prefill: prefill.timings })
  const continuation = longPrompt + prefill.content
  const live = await completion(continuation, 48)
  const retained = await client.call('checkpoints.status', { id })
  const pointers = () => walk(path.join(data, 'kv-checkpoints', id))
    .filter(f => path.basename(f) === 'latest.json').map(f => ({ path: path.relative(data, f), sha256: sha(f) }))
  const beforeTitle = pointers()
  assert.equal(beforeTitle.length, 1)
  await completion('Write a short title for a calculation exercise about multiplication. '.repeat(6), 12)
  await stop()
  const stopped = await client.call('checkpoints.status', { id })
  assert.equal(stopped.reason_code, 'longer_checkpoint_retained', JSON.stringify(stopped))
  assert.equal(stopped.generation_id, retained.generation_id)
  assert.deepEqual(pointers(), beforeTitle)
  record('short-title-cannot-replace-long-checkpoint', { mode, retained, stopped, pointers: beforeTitle })
  const fresh = await start()
  assert.notEqual(fresh.pid, original.pid)
  assert.equal(fresh.checkpoint.last_outcome, 'success')
  const restored = await completion(continuation, 48)
  assertInference(live, restored, { draft: mode === 'mtp', reuse: true, minimumReuse: saved.prompt_tokens })
  record(`fresh-process-${mode}-long-prefix-after-short-title`, { mode, original, fresh, live, restored })
  killOwned(fresh.pid)
  await sleep(1200); await stop()
  assert.deepEqual(pointers(), beforeTitle)
  const afterCrash = await start()
  const recovered = await completion(continuation, 48)
  assert.notEqual(afterCrash.pid, fresh.pid)
  assertInference(live, recovered, { draft: mode === 'mtp', reuse: true, minimumReuse: saved.prompt_tokens })
  record('idle-generation-survives-forced-exit', { mode, afterCrash, recovered })
  await stop()
}
async function main() {
  await new Promise((resolve, reject) => {
    const probe = require('node:net').createServer()
    probe.once('error', reject)
    probe.listen({ host: '127.0.0.1', port, exclusive: true }, () => probe.close(resolve))
  })
  assert.equal(sha(args.engine), profile.launcher_sha256, 'launcher must match the selected fixed build')
  report.profile = profile.id
  report.version = cp.execFileSync(args.engine, ['--version'], { windowsHide: true, encoding: 'utf8' }).trim()
  // Pin auxiliary kernels as evidence too: the manager DLL contract alone does
  // not establish that HIP's external runtime assets are complete or unchanged.
  report.runtime_assets = [...walk(path.dirname(args.engine)), ...walk(path.join(path.dirname(args.engine), '..', '.kpack'))].filter(f => /\.(dll|kpack|dat|co|hsaco|zlib)$/i.test(f))
    .map(f => ({ path: path.relative(path.dirname(args.engine), f), bytes: fs.statSync(f).size, sha256: sha(f) }))
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
  assert.equal(probed.capabilities.persistenceProfile, profile.id)
  assert.ok(!probed.capabilities.checkpointDraftTypes.includes('draft-eagle3'))
  const seed = path.join(args.root, 'external-seed.bin')
  await update({ name: 'Isolated checkpoint acceptance', engine_id: engine.id, model_path: args.model,
    host: '127.0.0.1', port, launch_mode: 'managed', ctx_size: context, ctx_size_auto: false,
    cache_type_k: kv, cache_type_v: kv, cache_type_draft_k: kv, cache_type_draft_v: kv, kv_unified: true,
    gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99, draft_tokens: 3, flash_attn: 'on', fit_mode: 'off',
    warmup: false, parallel: 1, cache_prompt: true, cache_idle_slots: true, cache_ram: 256, ctx_checkpoints: 2,
    slots_enabled: true, metrics: true, props: true, verbose: true, mmproj_auto: false,
    spec_type: 'ngram-cache', lookup_cache_dynamic: seed, lookup_cache_managed: true,
    kv_checkpoint: { enabled: false, auto_save: true, auto_restore: true, storage_limit_gib: 4, minimum_prompt_tokens: 16 } })
  sampler = memorySampler(client, id)
  report.memory = sampler.summary
  if (args['failure-only']) {
    await failureAcceptance()
    if (args['scope-checks']) {
      const baseline = { ctx_size: context, ctx_checkpoints: 2, parallel: 1, draft_tokens: 3,
        spec_type: 'draft-mtp', draft_model_path: args.mtp, kv_unified: true, kv_unified_mode: 'on',
        cache_type_k: kv, cache_type_v: kv, cache_type_draft_k: kv, cache_type_draft_v: kv }
      for (const patch of [{ ctx_size: 8192 }, { ctx_checkpoints: 0 }, { parallel: 2 }, { draft_tokens: 1 },
        { cache_type_draft_k: kv === 'f16' ? 'q8_0' : 'f16' }, { kv_unified_mode: 'off' },
        { draft_model_path: args.model }]) {
        await update({ ...baseline, ...patch })
        const blocked = await client.call('checkpoints.eligibility', { id })
        assert.equal(blocked.eligible, false, JSON.stringify({ patch, blocked }))
        assert.notEqual(blocked.reason_code, 'disabled')
        record('unreviewed-model-scope-blocked', { patch, eligibility: blocked })
      }
      await update(baseline)
    }
    report.passed = true
    return
  }
  if (args['idle-only']) {
    for (const mode of ['mtp', 'none']) await idleAcceptance(mode)
    report.passed = true
    return
  }
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
    { ctx_size: 4096, spec_type: 'draft-eagle3' }, { spec_type: 'draft-mtp', draft_model_path: args.mtp, parallel: 2 }]) {
    await update(patch)
    const eligibility = await client.call('checkpoints.eligibility', { id })
    assert.equal(eligibility.eligible, false, JSON.stringify(eligibility))
    assert.notEqual(eligibility.reason_code, 'disabled')
    record('unqualified-draft-configuration-blocked', { patch, eligibility })
  }
  report.passed = true
}
main().catch(e => { report.error = e.stack; process.exitCode = 1; console.error(e) }).finally(async () => {
  await sampler?.stop()
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
