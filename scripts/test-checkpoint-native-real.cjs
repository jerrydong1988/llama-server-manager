// Opt-in Windows ROCm candidate qualification. Automatic manager persistence is
// deliberately disabled: an unreviewed artifact must earn its registry entry.
// --bin DIR --root NEW_DIR --engine EXE --model GGUF --draft GGUF
// --mode mtp|dflash|simple --parallel 1|2|4 --port PORT
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const { sha256, inspectSlotFile, assertFullRead, assertInference, memorySampler, processMemory } = require('./lib/checkpoint-evidence.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + 2 * i].slice(2), process.argv[3 + 2 * i]]))
assert.equal(process.platform, 'win32')
for (const key of ['bin', 'root', 'engine', 'model', 'draft']) assert.ok(path.isAbsolute(args[key] || ''), `--${key} must be absolute`)
for (const key of ['engine', 'model', 'draft']) assert.ok(fs.statSync(args[key]).isFile())
assert.ok(['mtp', 'dflash', 'simple'].includes(args.mode))
const parallel = Number(args.parallel || 4), port = Number(args.port || 18849)
assert.ok([1, 2, 4].includes(parallel))
assert.ok(Number.isInteger(port) && port > 1024 && port < 65536)
assert.ok(!fs.existsSync(args.root), '--root must be new')
fs.mkdirSync(args.root, { recursive: true })
fs.writeFileSync(path.join(args.root, '.owned-validation'), 'native-checkpoint-acceptance-v1')
const data = path.join(args.root, 'data'), slots = path.join(args.root, 'slots'), id = 'native-candidate'
fs.mkdirSync(slots)
const client = managementClient({ cli: path.join(args.bin, 'lsm.exe'), app: path.join(args.bin, 'llama-server-manager.exe'), data, timeout: 600000 })
let sampler
const report = { started: new Date().toISOString(), scope: 'native API candidate, not automatic manager qualification',
  mode: args.mode, parallel, engine: args.engine, model: args.model, draft: args.draft,
  environment: { GGML_CUDA_NO_PINNED: process.env.GGML_CUDA_NO_PINNED || null }, events: [], owned_pids: [], stop_verifications: [] }
const logPath = path.join(data, 'configs', 'logs', `${id}.log`)
const record = (name, details = {}) => {
  report.events.push({ ...details, name, at: new Date().toISOString() })
  fs.writeFileSync(path.join(args.root, 'report.json'), JSON.stringify(report, null, 2))
  console.log(name)
}
function processes() {
  const raw = cp.execFileSync('powershell.exe', ['-NoProfile', '-Command',
    "@(Get-CimInstance Win32_Process -Filter \"Name='llama-server.exe' OR Name='llama-server-manager.exe'\" | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:LSM_VALIDATION_ROOT) } | Select-Object ProcessId,ExecutablePath,CommandLine) | ConvertTo-Json -Compress"],
  { windowsHide: true, encoding: 'utf8', env: { ...process.env, LSM_VALIDATION_ROOT: args.root } }).trim()
  const found = raw ? JSON.parse(raw) : []
  return (Array.isArray(found) ? found : [found]).filter(p => [args.engine, path.join(args.bin, 'llama-server-manager.exe')]
    .some(exe => exe.toLowerCase() === p.ExecutablePath?.toLowerCase()))
}
async function request(endpoint, body, allowFailure = false) {
  const response = await fetch(`http://127.0.0.1:${port}${endpoint}`, {
    method: body === undefined ? 'GET' : 'POST', headers: { 'Content-Type': 'application/json' },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(300000) })
  const result = await response.json()
  if (!allowFailure) assert.equal(response.status, 200, JSON.stringify(result))
  return allowFailure ? { status: response.status, body: result } : result
}
const completion = (slot, prompt, count = 40, cachePrompt = true) => request('/completion', {
  id_slot: slot, prompt, n_predict: count, temperature: 0, seed: 42, cache_prompt: cachePrompt, return_tokens: true, ignore_eos: true })
async function update(patch, create = false) {
  await client.mutate(create ? 'instances.create' : 'instances.update', {
    id, revision: (await client.call('service.status')).revision, [create ? 'config' : 'patch']: patch })
}
async function start(label) {
  await client.mutate('instances.start', { id })
  const deadline = Date.now() + 300000
  for (;;) {
    const health = await client.call('instances.health', { id })
    if (health.running) {
      try {
        const response = await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) })
        if (response.ok) {
          assert.equal(health.running.private_console, true)
          const slotList = await request('/slots')
          assert.equal(slotList.length, parallel)
          report.owned_pids.push(health.running.pid)
          const log = fs.readFileSync(logPath, 'utf8')
          assert.match(log, /ROCm|HIP/)
          const offloads = [...log.matchAll(/offloaded (\d+)\/(\d+) layers to GPU/g)]
          assert.ok(offloads.length && offloads.every(m => Number(m[1]) > 0), 'must actually offload layers')
          record('started', { label, pid: health.running.pid, slots: slotList.map(s => ({ id: s.id, n_ctx: s.n_ctx })) })
          return health.running.pid
        }
      } catch (e) { if (e.code === 'ERR_ASSERTION') throw e }
    }
    assert.ok(Date.now() < deadline, `engine did not become ready: ${JSON.stringify(health)}`)
    await sleep(500)
  }
}
async function stop(label) {
  const old = (await client.call('instances.health', { id })).running
  if (old && processes().some(p => p.ProcessId === old.pid)) {
    report.process_memory = [...(report.process_memory || []), processMemory(old.pid)]
  }
  await client.mutate('instances.stop', { id })
  assert.equal((await client.call('instances.health', { id })).running, null)
  if (old) {
    assert.ok(!processes().some(p => p.ProcessId === old.pid), 'old process is still alive')
    report.stop_verifications.push({ pid: old.pid, exited: true, at: new Date().toISOString() })
    if (fs.existsSync(logPath)) fs.copyFileSync(logPath, path.join(args.root, `${label}.log`))
  }
}
const sentinel = slot => `Slot ${slot}. Write the result of 17 times 23 as 391, then repeat 391 separated by spaces. Answer: 391 391 391 `
const longPrompt = (slot, revised = false, rewriteAfter = 120) => 'Reference records for a multiplication exercise.\n' +
  Array.from({ length: 140 }, (_, i) => `Record ${i}: ${revised && i > rewriteAfter ? 'nineteen times twenty-three equals 437' : 'seventeen times twenty-three equals 391'}.\n`).join('') +
  (revised ? `Slot ${slot}. Use the result in the last record. Repeat that result separated by spaces. Answer: 437 437 437 ` : sentinel(slot))
const immutable = new Map()
const assertImmutable = () => { for (const [file, hash] of immutable) assert.equal(sha256(file), hash, `old checkpoint changed: ${file}`) }
async function inferenceEverySlot(references, name) {
  const results = []
  for (let slot = 0; slot < parallel; slot++) {
    const result = await completion(slot, sentinel(slot))
    assertInference(references[slot], result, { draft: true })
    results.push({ slot, content: result.content, tokens: result.tokens, timings: result.timings })
  }
  record(name, { results, monitoring: (await client.call('instances.health', { id })).monitoring })
}
async function main() {
  await new Promise((resolve, reject) => {
    const probe = require('node:net').createServer(); probe.once('error', reject)
    probe.listen({ host: '127.0.0.1', port, exclusive: true }, () => probe.close(resolve))
  })
  report.launcher_sha256 = sha256(args.engine)
  report.model_sha256 = sha256(args.model); report.draft_sha256 = sha256(args.draft)
  await client.call('service.start')
  for (const [purpose, directory] of [['engine', path.dirname(args.engine)], ...[...new Set([args.model, args.draft].map(p => path.dirname(p)))].map(p => ['model', p])]) {
    await client.mutate('paths.authorize', { purpose, path: directory })
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('models.refresh'); await client.mutate('engines.refresh')
  const norm = p => p.replace(/^\\\\\?\\/, '').toLowerCase()
  const engine = (await client.call('engines.list')).engines.find(e => norm(e.exe) === norm(args.engine))
  assert.ok(engine)
  const probe = await client.mutate('engines.probe', { id: engine.id }); record('capability-probe', probe)
  await update({ name: 'Isolated native checkpoint qualification', engine_id: engine.id, model_path: args.model,
    host: '127.0.0.1', port, launch_mode: 'managed', ctx_size: 4096 * parallel, ctx_size_auto: false,
    gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99, draft_tokens: 3, flash_attn: 'on', fit_mode: 'off',
    warmup: false, parallel, cache_prompt: true, cache_idle_slots: false, cache_ram: 1, ctx_checkpoints: 2,
    slots_enabled: true, metrics: true, props: true, verbose: true, mmproj_auto: false, slot_save_path: slots,
    spec_type: `draft-${args.mode}`, draft_model_path: args.draft, kv_unified: true,
    kv_checkpoint: { enabled: false, auto_save: false, auto_restore: false } }, true)
  sampler = memorySampler(client, id)
  report.memory = sampler.summary
  const eligibility = await client.call('checkpoints.eligibility', { id })
  assert.equal(eligibility.eligible, false); record('automatic-persistence-disabled', eligibility)
  const firstPid = await start('baseline'), references = [], checkpoints = [], lives = []
  const rewriteReference = await completion(0, longPrompt(0, true), 40, false)
  assert.ok(rewriteReference.content.includes('437') && !rewriteReference.content.includes('391'), 'rewrite fixture must change the answer')
  record('cold-reference-for-rewritten-prefix', { result: rewriteReference })
  const earlyRewriteReference = await completion(0, longPrompt(0, true, 75), 40, false)
  assert.ok(earlyRewriteReference.content.includes('437') && !earlyRewriteReference.content.includes('391'))
  record('cold-reference-for-rewrite-before-checkpoint', { result: earlyRewriteReference })
  for (let slot = 0; slot < parallel; slot++) {
    references.push(await completion(slot, sentinel(slot)))
    assert.ok(references[slot].content.includes('391'))
    const prompt = longPrompt(slot), prefill = await completion(slot, prompt, 24)
    assert.ok(prefill.timings.prompt_n > 1000)
    const filename = `good-${slot}.bin`, saved = await request(`/slots/${slot}?action=save`, { filename })
    const file = path.join(slots, filename), format = inspectSlotFile(file)
    assert.equal(format.bytes, saved.n_written)
    assert.ok(format.targetStateBytes > 0 && format.checkpoints.length > 0)
    assert.ok(format.liveDraft?.state.bytes > 0, 'full draft context missing')
    immutable.set(file, sha256(file))
    const continuation = prompt + prefill.content, live = await completion(slot, continuation)
    checkpoints.push({ slot, filename, file, saved, format, continuation }); lives.push(live)
    record('saved-complete-native-state', { slot, saved, format, sha256: immutable.get(file),
      speculative_nonempty: format.liveDraft.speculative.bytes > 0 })
  }
  await stop('baseline')
  const secondPid = await start('restore'); assert.notEqual(secondPid, firstPid)
  for (const state of checkpoints) assertFullRead(state.saved, await request(`/slots/${state.slot}?action=restore`, { filename: state.filename }))
  for (const state of checkpoints) {
    const restored = await completion(state.slot, state.continuation)
    assertInference(lives[state.slot], restored, { draft: true, reuse: true, minimumReuse: state.saved.n_saved })
    record('fresh-process-restored-inference', { slot: state.slot, old_pid: firstPid, new_pid: secondPid, live: lives[state.slot], restored })
  }
  const rewritten = await completion(0, longPrompt(0, true))
  assertInference(rewriteReference, rewritten, { draft: true, marker: '437' })
  assert.notEqual(rewritten.content, lives[0].content, 'rewritten prompt reused the stale answer')
  assert.ok(rewritten.timings.cache_n > 0 && rewritten.timings.cache_n < checkpoints[0].saved.n_saved,
    'late rewrite must reuse a valid earlier checkpoint and re-evaluate the changed suffix')
  record('long-prefix-rewrite', { result: rewritten })
  const earlyRewrite = await completion(0, longPrompt(0, true, 75))
  assertInference(earlyRewriteReference, earlyRewrite, { draft: true, marker: '437' })
  assert.equal(earlyRewrite.timings.cache_n, 0, 'rewrite before retained checkpoints must re-prefill')
  record('rewrite-before-retained-checkpoint-reprefills', { result: earlyRewrite })
  await inferenceEverySlot(references, 'all-slots-healthy-after-prefix-rewrite')
  // Corrupt only disposable copies. A successful native response may still have
  // silently dropped a draft appendix: consuming fewer bytes is not full restore.
  const source = checkpoints[0]
  for (const kind of ['truncated-main', 'invalid-draft-version']) {
    const file = path.join(slots, `${kind}.bin`); fs.copyFileSync(source.file, file)
    if (kind === 'truncated-main') fs.truncateSync(file, Math.floor(source.format.mainBytes / 2))
    else {
      const fd = fs.openSync(file, 'r+'), b = Buffer.alloc(4); b.writeUInt32LE(999)
      try { fs.writeSync(fd, b, 0, 4, source.format.liveDraft.offset + 4); fs.fsyncSync(fd) } finally { fs.closeSync(fd) }
    }
    const result = await request('/slots/0?action=restore', { filename: path.basename(file) }, true)
    assert.ok(result.status !== 200 || result.body.n_read !== fs.statSync(file).size, 'invalid file falsely accepted as complete')
    record('invalid-native-restore-detected', { kind, result, bytes: fs.statSync(file).size })
    await inferenceEverySlot(references, `all-slots-infer-after-${kind}`)
    assertImmutable()
  }
  await stop('faults')
  // Exercise rejected RAM prompt-cache candidates, not a real disk-full event.
  await update({ cache_idle_slots: true, cache_ram: 1 })
  const pressurePid = await start('ram-rejection')
  await completion(0, longPrompt(0), 24)
  await completion(parallel > 1 ? 1 : 0, sentinel(1))
  const pressureRewrite = await completion(0, longPrompt(0, true))
  assertInference(rewriteReference, pressureRewrite, { draft: true, marker: '437' })
  assert.match(fs.readFileSync(logPath, 'utf8'), /exceeds cache size limit/, 'RAM rejection branch was not exercised')
  await inferenceEverySlot(references, 'all-slots-healthy-after-ram-cache-rejection')
  assertImmutable(); await stop('ram-rejection')
  await update({ cache_idle_slots: false })
  const finalPid = await start('final-recovery'); assert.notEqual(finalPid, pressurePid)
  for (const state of checkpoints) {
    assertFullRead(state.saved, await request(`/slots/${state.slot}?action=restore`, { filename: state.filename }))
    const restored = await completion(state.slot, state.continuation)
    assertInference(lives[state.slot], restored, { draft: true, reuse: true, minimumReuse: state.saved.n_saved })
  }
  record('old-checkpoints-recover-in-new-process-after-faults', { pid: finalPid }); assertImmutable()
  await stop('final-recovery'); report.passed = true
}
main().catch(error => { report.error = error.stack; report.passed = false; process.exitCode = 1; console.error(error) })
  .finally(async () => {
    await sampler?.stop()
    try { await stop('cleanup') } catch (error) { report.cleanup_stop_error = error.message }
    try { await client.mutate('service.stop') } catch (error) { report.cleanup_service_error = error.message }
    const deadline = Date.now() + 45000
    while (processes().length && Date.now() < deadline) await sleep(1000)
    const remaining = processes()
    if (remaining.length) {
      report.passed = false; process.exitCode = 1; report.forced_cleanup = remaining.map(p => p.ProcessId)
      for (const p of remaining) if (processes().some(q => q.ProcessId === p.ProcessId))
        cp.execFileSync('taskkill.exe', ['/PID', String(p.ProcessId), '/T', '/F'], { windowsHide: true })
    }
    report.processes_after_cleanup = processes(); report.finished = new Date().toISOString(); record('finished')
  })
