// Opt-in, real Windows/ROCm campaign. All data/processes belong to a NEW root.
// --bin DIR --root NEW_DIR --engine EXE --model GGUF [--cases JSON]
// Uses a debug candidate's existing isolated-runtime login test hook, never OS registration.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process')
const net = require('node:net'), crypto = require('node:crypto'), assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const { sha256, inspectSlotFile, memorySampler } = require('./lib/checkpoint-evidence.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + 2 * i].slice(2), process.argv[3 + 2 * i]]))
assert.equal(process.platform, 'win32')
for (const k of ['bin', 'root', 'engine', 'model']) assert.ok(path.isAbsolute(args[k] || ''), k)
assert.ok(!fs.existsSync(args.root), 'new isolated root required')
fs.mkdirSync(args.root, { recursive: true })
fs.writeFileSync(path.join(args.root, '.owned-validation'), 'b12469-manager-acceptance-v1')
const data = path.join(args.root, 'data'), app = path.join(args.bin, 'llama-server-manager.exe')
fs.copyFileSync(__filename, path.join(args.root, 'harness-snapshot.cjs'))
if (args.fingerprints) {
  assert.ok(path.isAbsolute(args.fingerprints)); const bytes = fs.readFileSync(args.fingerprints)
  assert.ok(bytes.length < 1024 * 1024); JSON.parse(bytes)
  fs.mkdirSync(path.join(data, 'kv-checkpoints'), { recursive: true })
  fs.writeFileSync(path.join(data, 'kv-checkpoints', 'fingerprints-v1.json'), bytes)
}
const client = managementClient({ cli: path.join(args.bin, 'lsm.exe'), app, data, timeout: 1500000 })
const shapes = JSON.parse(args.cases || '[[1,"f16"],[1,"q8_0"],[2,"f16"],[2,"q8_0"],[4,"f16"]]')
const report = { started: new Date().toISOString(), production_modified: false, events: [],
  harness_sha256: sha256(__filename), fingerprint_cache_seed: args.fingerprints || null,
  manager_sha256: sha256(app), engine_sha256: sha256(args.engine), model_sha256: sha256(args.model) }
const record = (name, detail = {}) => {
  report.events.push({ name, at: new Date().toISOString(), ...detail })
  fs.writeFileSync(path.join(args.root, 'report.json'), JSON.stringify(report, null, 2)); console.log(name)
}
const norm = p => path.toNamespacedPath(p).toLowerCase()
const walk = p => fs.existsSync(p) ? fs.readdirSync(p, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(p, e.name)) : [path.join(p, e.name)]) : []
function owned() {
  const raw = cp.execFileSync('powershell.exe', ['-NoProfile', '-Command', "@(Get-CimInstance Win32_Process | Where-Object { $_.Name -in @('llama-server-manager.exe','llama-server.exe') -and $_.CommandLine -and $_.CommandLine.Contains($env:LSM_VALIDATION_ROOT) } | Select-Object ProcessId,ExecutablePath,CommandLine) | ConvertTo-Json -Compress"], { windowsHide: true, encoding: 'utf8', env: { ...process.env, LSM_VALIDATION_ROOT: args.root } }).trim()
  const rows = raw ? JSON.parse(raw) : []; return (Array.isArray(rows) ? rows : [rows]).filter(p => [app, args.engine].some(e => norm(e) === norm(p.ExecutablePath)))
}
async function gone(pid) {
  const end = Date.now() + 45000
  while (owned().some(p => p.ProcessId === pid)) { assert.ok(Date.now() < end, `PID ${pid} did not exit`); await sleep(500) }
}
async function stopHost() { await client.mutate('service.stop'); for (const p of owned()) await gone(p.ProcessId) }
async function runtime(command, payload) {
  const token = fs.readFileSync(path.join(data, 'runtime', 'control-token'), 'utf8').trim()
  const endpoint = `\\\\.\\pipe\\llama-server-manager-runtime-${crypto.createHash('sha256').update(token).digest('hex').slice(0, 32)}`
  const body = Buffer.from(JSON.stringify({ protocol_version: 1, request_id: crypto.randomUUID(), token, command: { command, ...(payload ? { payload } : {}) } }))
  const frame = Buffer.alloc(body.length + 4); frame.writeUInt32LE(body.length); body.copy(frame, 4)
  const response = await new Promise((resolve, reject) => {
    const socket = net.createConnection(endpoint), chunks = []
    socket.setTimeout(300000, () => socket.destroy(new Error('runtime timeout')))
    socket.once('connect', () => socket.write(frame)); socket.on('data', d => chunks.push(d)); socket.once('error', reject)
    socket.once('end', () => { try { const b = Buffer.concat(chunks); assert.ok(b.length >= 4 && b.length === b.readUInt32LE(0) + 4); resolve(JSON.parse(b.subarray(4))) } catch (e) { reject(e) } })
  })
  assert.equal(response.error, null, JSON.stringify(response)); assert.ok(response.reply)
  return response.reply.payload
}
let daemon, currentId, port, sampler
async function spawnBackground() {
  const log = fs.openSync(path.join(args.root, 'background.log'), 'a')
  daemon = cp.spawn(app, ['--runtime-service', '--runtime-data-dir', data], { windowsHide: true, stdio: ['ignore', log, log], env: { ...process.env, LSM_RUNTIME_TEST_LOGIN_REGISTERED: '1' } }); fs.closeSync(log)
  const end = Date.now() + 45000
  for (;;) { try { const st = await runtime('get_status'); assert.equal(st.service_pid, daemon.pid); break } catch (e) { assert.equal(daemon.exitCode, null, e.message); assert.ok(Date.now() < end, e.message); await sleep(250) } }
  await runtime('set_background_enabled', { enabled: true })
}
async function request(endpoint, body) {
  const res = await fetch(`http://127.0.0.1:${port}${endpoint}`, { method: body ? 'POST' : 'GET', headers: { 'Content-Type': 'application/json', Connection: 'close' }, ...(body ? { body: JSON.stringify(body) } : {}), signal: AbortSignal.timeout(180000) })
  const json = await res.json(); assert.equal(res.status, 200, JSON.stringify(json)); return json
}
const compact = r => ({ content: r.content, tokens: r.tokens, timings: r.timings })
const completion = (id, prompt, cache = true, n = 64) => request('/completion', { id_slot: id, prompt, n_predict: n, temperature: 0, seed: 42, cache_prompt: cache, return_tokens: true, ignore_eos: true }).then(compact)
const replay = states => Promise.all(states.map(s => completion(s.id, s.prompt)))
function compare(expected, actual, minimum) {
  assert.deepEqual(actual.tokens, expected.tokens); assert.equal(actual.content, expected.content)
  assert.ok(actual.timings.cache_n >= minimum - 1 && actual.timings.draft_n_accepted > 0)
  for (const key of ['draft_n', 'draft_n_accepted']) assert.ok(Math.abs(actual.timings[key] - expected.timings[key]) <= 3)
  assert.ok(actual.timings.draft_n_accepted / actual.timings.draft_n + 0.05 >= expected.timings.draft_n_accepted / expected.timings.draft_n)
}
async function update(patch) { await client.mutate('instances.update', { id: currentId, patch, revision: (await client.call('service.status')).revision }) }
async function eligibility() {
  // The read-only endpoint can outlive the CLI transport's 60s timeout while
  // hashing a new 35GB model. Never retry a mutation; allow the hash to finish.
  for (let attempt = 0; attempt < 10; attempt++) {
    const response = await client.call('checkpoints.eligibility', { id: currentId }, { allowFailure: true })
    if (response.ok) return response.data
    assert.ok(response.error?.code === 'CLIENT_ERROR' && response.error.message.includes('timed out'), JSON.stringify(response))
    record('eligibility-read-awaits-first-model-hash', { attempt }); await sleep(5000)
  }
  throw new Error('eligibility read exceeded the bounded hash wait')
}
async function ready(slots, background = false) {
  const end = Date.now() + 300000, observed = []
  for (;;) {
    const st = background ? await runtime('get_status') : await client.call('runtime.status')
    const running = st.running[currentId], c = st.checkpoints[currentId]
    if (running && c && (!c.expected_pid || running.pid === c.expected_pid)) {
      let row = observed.find(p => p.pid === running.pid)
      if (!row) { row = { pid: running.pid, ever_routable: false }; observed.push(row) }
      row.ever_routable ||= c.routable
      if (c.routable) {
        const list = await request('/slots'); assert.equal(list.length, slots)
        assert.equal(running.private_console, true)
        return { pid: running.pid, checkpoint: c, observed }
      }
    }
    assert.ok(Date.now() < end, JSON.stringify(st)); await sleep(250)
  }
}
const start = async slots => { await client.mutate('instances.start', { id: currentId }); return ready(slots) }
async function stop(pid) { await client.mutate('instances.stop', { id: currentId }); await gone(pid) }
function generation() {
  const pointerFile = walk(path.join(data, 'kv-checkpoints', currentId)).find(p => path.basename(p) === 'latest.json'); assert.ok(pointerFile)
  const pointerBytes = fs.readFileSync(pointerFile), pointer = JSON.parse(pointerBytes)
  const dir = path.join(path.dirname(pointerFile), 'generations', pointer.generationId), mf = path.join(dir, 'manifest.json'), manifest = JSON.parse(fs.readFileSync(mf))
  assert.equal(sha256(mf), pointer.manifestSha256)
  const formats = manifest.slots.map(s => { const f = path.join(dir, s.filename); assert.equal(sha256(f), s.sha256); const v = inspectSlotFile(f); assert.ok(v.checkpoints.length && v.liveDraft.state.bytes > 0); return v })
  return { pointerFile, pointerBytes, pointer, dir, manifest, formats }
}
async function seed(slots) {
  const states = []
  for (let id = 0; id < slots; id++) {
    const marker = 391 + id * 17, prefix = Array.from({ length: 240 }, (_, n) => `Record ${n}: code ${marker}; secret ${['cobalt','amber','violet','silver'][id]} for slot ${id}.\n`).join('')
    const p = `${prefix}Slot ${id}. Repeat ${marker} separated by spaces. Answer: ${marker} ${marker} ${marker} `
    const warm = await completion(id, p, false, 24)
    states.push({ id, prompt: p + warm.content, filename: `reference-${id}.bin` })
  }
  for (const s of states) s.save = await request(`/slots/${s.id}?action=save`, { filename: s.filename })
  const live = await replay(states); states.forEach((s, i) => { s.live = live[i] })
  for (const s of states) { const v = await request(`/slots/${s.id}?action=restore`, { filename: s.filename }); assert.equal(v.n_read, s.save.n_written) }
  return states
}
async function verify(states, g, name) {
  const actual = await replay(states)
  states.forEach((s, i) => compare(s.live, actual[i], g.manifest.slots.find(v => v.id === s.id).promptTokens))
  record(name, { id: currentId, rows: states.map((s, i) => ({ slot: s.id, expected: s.live, actual: actual[i] })) })
}
async function main() {
  assert.equal(report.model_sha256, '6c6b816537abad90b250a0972b345466028d861ddfe316d5f0de31ca6440f781')
  await client.call('service.start')
  for (const [purpose, p] of [['engine', path.dirname(args.engine)], ['model', path.dirname(args.model)]]) {
    await client.mutate('paths.authorize', { purpose, path: p }); await client.mutate('directories.add', { purpose, path: p, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('models.refresh'); await client.mutate('engines.refresh')
  const engine = (await client.call('engines.list')).engines.find(e => norm(e.exe) === norm(args.engine)); assert.ok(engine)
  const probe = await client.mutate('engines.probe', { id: engine.id })
  assert.equal(probe.capabilities.persistenceProfile, 'windows-rocm-b12469-qwen36-embedded-mtp'); assert.equal(probe.capabilities.contextCheckpointPersistence, true)
  record('complete-bundle-capability-recognized', { probe })
  for (const [index, [slots, kv]] of shapes.entries()) {
    currentId = `b12469-${slots}-${kv}`; port = 23469 + index
    await new Promise((resolve, reject) => { const s = net.createServer(); s.once('error', reject); s.listen({ host: '127.0.0.1', port, exclusive: true }, () => s.close(resolve)) })
    const config = { name: currentId, engine_id: engine.id, model_path: args.model, draft_model_path: '', host: '127.0.0.1', port, launch_mode: 'managed', ctx_size: 8192 * slots, ctx_size_auto: false, gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99, flash_attn: 'on', fit_mode: 'off', kv_unified: true, kv_unified_mode: 'on', batch_size: 512, ubatch_size: 512, checkpoint_min_step: 512, ctx_checkpoints: 2, parallel: slots, cache_type_k: kv, cache_type_v: kv, cache_type_draft_k: kv, cache_type_draft_v: kv, spec_type: 'draft-mtp', draft_tokens: 3, cache_prompt: true, cache_idle_slots: slots === 1, cache_ram: 256, slots_enabled: true, verbose: true, warmup: false, mmproj_auto: false, kv_checkpoint: { enabled: true, allow_local_validation: true, auto_save: true, auto_restore: true, idle_save_seconds: 0, preserve_longer: false, storage_limit_gib: 32, minimum_prompt_tokens: 16 } }
    await client.mutate('instances.create', { id: currentId, config, revision: (await client.call('service.status')).revision })
    const before = await eligibility(); assert.equal(before.can_validate, true, JSON.stringify(before)); assert.equal(before.eligible, false)
    record('candidate-needs-local-receipt', { id: currentId, before })
    sampler = memorySampler(client, currentId)
    const receipt = await client.mutate('checkpoints.validate', { id: currentId })
    assert.equal(receipt.schema, 4); assert.equal(receipt.old_process_exited, true); assert.equal(receipt.slots.length, slots)
    const nativeLog = fs.readFileSync(path.join(receipt.evidence_directory, 'before.log'), 'utf8')
    assert.match(nativeLog, /8060S/); assert.match(nativeLog, /draft-mtp/)
    assert.ok([...nativeLog.matchAll(/K \((\w+)\):[^\n]*V \((\w+)\):/g)].some(m => m[1] === kv && m[2] === kv))
    assert.equal((await eligibility()).qualification, 'local_validated')
    record('manager-fresh-process-qualification', { id: currentId, receipt })
    for (const patch of [{ parallel: 4, ctx_size: 32768, cache_type_k: 'q8_0', cache_type_v: 'q8_0', cache_type_draft_k: 'q8_0', cache_type_draft_v: 'q8_0' }, { draft_model_path: args.model }, { ctx_size: 4096 * slots }, { batch_size: 2048 }, { ctx_checkpoints: 0 }, { checkpoint_min_step: 8192 }, { spec_type: 'ngram-mod,draft-mtp' }]) {
      await update({ ...config, ...patch }); const no = await eligibility(); assert.equal(no.can_validate, false); assert.equal(no.eligible, false); record('unverified-combination-rejected', { patch, no })
    }
    await update(config)
    if (args['qualification-only'] === 'true') {
      await sampler.stop(); record('qualification-only-complete', { id: currentId, memory: sampler.summary }); sampler = null
      continue
    }
    let running = await start(slots), states = await seed(slots)
    await stop(running.pid); const good = generation(); assert.equal(good.manifest.slots.length, slots)
    record('foreground-controlled-automatic-save', { id: currentId, pid: running.pid, manifest: good.manifest, formats: good.formats })
    const restored = await start(slots); assert.notEqual(restored.pid, running.pid); assert.equal(restored.checkpoint.phase, 'ready'); await verify(states, good, 'foreground-controlled-fresh-restore')
    fs.chmodSync(good.pointerFile, 0o444)
    try { await stop(restored.pid) } finally { fs.chmodSync(good.pointerFile, 0o666) }
    const denied = await client.call('checkpoints.status', { id: currentId }); assert.equal(denied.last_outcome, 'failed'); assert.deepEqual(fs.readFileSync(good.pointerFile), good.pointerBytes); generation()
    record('failed-save-preserves-old-generation', { id: currentId, denied })
    await update({ kv_checkpoint: { auto_save: false } })
    // Corrupt the final draft appendix while retaining valid outer manifest hashes.
    const bad = structuredClone(good.manifest), badId = crypto.randomUUID(), dir = path.join(path.dirname(good.dir), badId); fs.mkdirSync(dir); bad.generationId = badId
    for (const s of bad.slots) fs.copyFileSync(path.join(good.dir, s.filename), path.join(dir, s.filename))
    const last = bad.slots.at(-1), badFile = path.join(dir, last.filename), fd = fs.openSync(badFile, 'r+'), v = Buffer.alloc(4); v.writeUInt32LE(999)
    fs.writeSync(fd, v, 0, 4, good.formats.at(-1).liveDraft.offset + 4); fs.fsyncSync(fd); fs.closeSync(fd); last.sha256 = sha256(badFile)
    const mf = path.join(dir, 'manifest.json'); fs.writeFileSync(mf, JSON.stringify(bad)); fs.writeFileSync(good.pointerFile, JSON.stringify({ ...good.pointer, generationId: badId, manifestSha256: sha256(mf) }))
    running = await start(slots); assert.equal(running.checkpoint.reason_code, 'cold_after_restore_failure'); const rejected = running.observed.filter(p => p.pid !== running.pid); assert.ok(rejected.length)
    for (const p of rejected) { assert.equal(p.ever_routable, false); await gone(p.pid) }
    const healthy = await completion(0, 'Repeat 391 separated by spaces. Answer: 391 391 391 ', false, 24); assert.ok(healthy.content.includes('391') && healthy.timings.draft_n_accepted > 0)
    record('partial-restore-process-discarded', { id: currentId, running, healthy }); await stop(running.pid)
    fs.writeFileSync(good.pointerFile, good.pointerBytes); running = await start(slots); await verify(states, good, 'good-generation-after-failed-restore')
    assert.ok(owned().some(p => p.ProcessId === running.pid && norm(p.ExecutablePath) === norm(args.engine)))
    cp.execFileSync('taskkill.exe', ['/PID', String(running.pid), '/T', '/F'], { windowsHide: true }); await gone(running.pid)
    await client.mutate('instances.stop', { id: currentId }); assert.deepEqual(fs.readFileSync(good.pointerFile), good.pointerBytes); generation()
    record('forced-exit-retains-published-generation', { id: currentId, pid: running.pid })
    running = await start(slots); await verify(states, good, 'fresh-restore-after-forced-exit'); await stop(running.pid)
    // Obtain an actual manager-authorized launch spec; never fabricate admission.
    await update({ kv_checkpoint: { auto_save: true } }); running = await start(slots); states = await seed(slots)
    const spec = JSON.parse(fs.readFileSync(path.join(data, 'runtime', 'runtime-state.json'))).desired_instances[currentId]
    assert.equal(spec.checkpoint.eligibility.qualification, 'local_validated'); await stopHost(); const shutdownGood = generation(); await gone(running.pid)
    record('management-host-shutdown-saved', { id: currentId, manifest: shutdownGood.manifest })
    await spawnBackground(); await runtime('start_instance', { spec }); let bg = await ready(slots, true); await verify(states, shutdownGood, 'standalone-background-fresh-restore')
    states = await seed(slots); await runtime('shutdown', { stop_instances: true }); await gone(bg.pid); await gone(daemon.pid); daemon = null
    const bgSaved = generation(); record('background-shutdown-saved', { id: currentId, manifest: bgSaved.manifest })
    await spawnBackground(); await runtime('start_instance', { spec }); bg = await ready(slots, true); await verify(states, bgSaved, 'background-shutdown-fresh-restore')
    await runtime('shutdown', { stop_instances: true }); await gone(bg.pid); await gone(daemon.pid); daemon = null
    await sampler.stop(); record('case-complete', { id: currentId, memory: sampler.summary }); sampler = null
    await client.call('service.start'); await client.mutate('checkpoints.clear', { id: currentId })
  }
  report.passed = true
}
main().catch(e => { report.passed = false; report.error = e.stack; process.exitCode = 1; console.error(e) }).finally(async () => {
  await sampler?.stop()
  if (daemon) { try { await runtime('shutdown', { stop_instances: true }); await gone(daemon.pid) } catch (e) { record('background-cleanup-error', { error: e.message }) } }
  try { await stopHost() } catch (e) { record('host-cleanup-error', { error: e.message }) }
  for (const p of owned()) { report.passed = false; process.exitCode = 1; record('forced-owned-cleanup', { pid: p.ProcessId }); cp.execFileSync('taskkill.exe', ['/PID', String(p.ProcessId), '/T', '/F'], { windowsHide: true }) }
  report.processes_after = owned(); report.finished = new Date().toISOString(); record('finished')
})
