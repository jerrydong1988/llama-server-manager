// Opt-in Windows/ROCm integration campaign. Never uses the default manager root.
// --bin DIR --root NEW_DIR --engine EXE --model GGUF --mtp GGUF
// --profile PROFILE_ID [--parallel 4] [--port PORT] [--spec-type ngram-mod,draft-mtp]
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const cp = require('node:child_process');
const assert = require('node:assert/strict');
const { managementClient, sleep } = require('./lib/management-client.cjs');
const { inspectSlotFile, assertInference, memorySampler, processMemory, speculativeTrace } = require('./lib/checkpoint-evidence.cjs');
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + 2 * i].slice(2), process.argv[3 + 2 * i]]));
assert.equal(process.platform, 'win32');
for (const key of ['bin', 'root', 'engine', 'model', 'mtp'])
    assert.ok(path.isAbsolute(args[key] || ''), `--${key} must be absolute`);
for (const key of ['bin', 'root', 'engine', 'model', 'mtp'])
    args[key] = path.resolve(args[key]);
const parallel = Number(args.parallel || 4), context = Number(args['context-budget'] || 4096) * parallel, kv = args.kv || 'f16';
const localValidation = args['local-validation'] === 'true';
assert.ok(['f16', 'q8_0'].includes(kv));
const specType = args['spec-type'] || 'draft-mtp';
assert.ok(['draft-mtp', 'ngram-mod,draft-mtp'].includes(specType));
assert.ok([2, 4].includes(parallel));
for (const key of ['engine', 'model', 'mtp'])
    assert.ok(fs.statSync(args[key]).isFile(), `missing ${key}`);
assert.ok(!fs.existsSync(args.root), '--root must be new; never reuse user data');
fs.mkdirSync(args.root, { recursive: true });
fs.writeFileSync(path.join(args.root, '.owned-validation'), 'manager-multislot-acceptance-v1');
const profiles = require('../src-tauri/src/commands/persistence_profiles.json');
const profile = profiles.find(p => p.id === args.profile);
assert.ok(profile, '--profile must name an embedded, immutable qualification candidate');
const data = path.join(args.root, 'data'), id = 'checkpoint-acceptance', port = Number(args.port || 18771);
assert.ok(Number.isInteger(port) && port > 1024 && port < 65536);
if (args.fingerprints) {
    assert.ok(path.isAbsolute(args.fingerprints));
    const bytes = fs.readFileSync(args.fingerprints);
    assert.ok(bytes.length < 1024 * 1024);
    JSON.parse(bytes);
    fs.mkdirSync(path.join(data, 'kv-checkpoints'), { recursive: true });
    fs.writeFileSync(path.join(data, 'kv-checkpoints', 'fingerprints-v1.json'), bytes);
}
const client = managementClient({ cli: path.join(args.bin, 'lsm.exe'), app: path.join(args.bin, 'llama-server-manager.exe'), data, timeout: 600000 });
let sampler;
const report = { started: new Date().toISOString(), engine: args.engine, scope: 'manager multi-slot automatic persistence', specType, parallel, context, fingerprint_cache_seed: args.fingerprints || null, events: [], owned_pids: [] };
const sha = file => {
    const hash = crypto.createHash('sha256'), buffer = Buffer.alloc(1024 * 1024);
    const fd = fs.openSync(file, 'r');
    try {
        for (;;) {
            const count = fs.readSync(fd, buffer, 0, buffer.length, null);
            if (!count)
                return hash.digest('hex');
            hash.update(buffer.subarray(0, count));
        }
    }
    finally {
        fs.closeSync(fd);
    }
};
const record = (name, details = {}) => {
    if (name.startsWith('fresh-process-'))
        fs.copyFileSync(path.join(data, 'configs', 'logs', `${id}.log`), path.join(args.root, `${name}.log`));
    report.events.push({ ...details, name, recorded_at: new Date().toISOString() });
    fs.writeFileSync(path.join(args.root, 'report.json'), JSON.stringify(report, null, 2));
    console.log(name);
};
const walk = dir => fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(dir, e.name)) : [path.join(dir, e.name)]) : [];
function ownedProcesses() {
    const output = cp.execFileSync('powershell.exe', ['-NoProfile', '-Command',
        "@(Get-CimInstance Win32_Process -Filter \"Name = 'llama-server-manager.exe' OR Name = 'llama-server.exe'\" | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($env:LSM_VALIDATION_ROOT) } | Select-Object ProcessId,ExecutablePath,CommandLine) | ConvertTo-Json -Compress"], { windowsHide: true, encoding: 'utf8', timeout: 30000, env: { ...process.env, LSM_VALIDATION_ROOT: args.root } }).trim();
    const parsed = output ? JSON.parse(output) : [];
    return (Array.isArray(parsed) ? parsed : [parsed]).filter(p => [args.engine, path.join(args.bin, 'llama-server-manager.exe')].some(exe => exe.toLowerCase() === p.ExecutablePath?.toLowerCase()));
}
async function shutdownService() {
    await client.mutate('service.stop');
    const deadline = Date.now() + 45000;
    while (ownedProcesses().length && Date.now() < deadline)
        await sleep(1000);
    assert.deepEqual(ownedProcesses(), [], 'owned engine and controller processes must exit without respawning');
}
async function request(endpoint, body) {
    const response = await fetch(`http://127.0.0.1:${port}${endpoint}`, { method: body === undefined ? 'GET' : 'POST', headers: { 'Content-Type': 'application/json', Connection: 'close' }, ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(180000) });
    const json = await response.json();
    assert.equal(response.status, 200, JSON.stringify(json));
    return json;
}
async function completion(slot, prompt, count = 48) {
    return request('/completion', { prompt, n_predict: count, temperature: 0, seed: 42, cache_prompt: true, id_slot: slot, return_tokens: true, ignore_eos: true });
}
async function update(patch) {
    const old = await client.call('instances.get', { id }, { allowFailure: true });
    await client.mutate(old.ok ? 'instances.update' : 'instances.create', { id, revision: (await client.call('service.status')).revision, [old.ok ? 'patch' : 'config']: patch });
}
async function start() {
    await client.mutate('instances.start', { id });
    const until = Date.now() + 600000;
    const observed = [];
    for (;;) {
        const h = await client.call('instances.health', { id });
        const c = await client.call('checkpoints.status', { id });
        const sameProcess = h.running && (!c.expected_pid || c.expected_pid === h.running.pid);
        if (sameProcess) {
            const previous = observed.find(p => p.pid === h.running.pid);
            if (previous)
                previous.ever_routable ||= c.routable;
            else {
                observed.push({ pid: h.running.pid, checkpoint: c, ever_routable: c.routable });
                report.owned_pids.push(h.running.pid);
            }
        }
        if (sameProcess && c.routable) {
            try {
                if ((await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(2000) })).ok) {
                    assert.equal(h.running.private_console, true);
                    const log = fs.readFileSync(path.join(data, 'configs', 'logs', `${id}.log`), 'utf8');
                    assert.match(log, /ROCm|HIP/);
                    const offloads = [...log.matchAll(/offloaded (\d+)\/(\d+) layers to GPU/g)];
                    assert.ok(offloads.length && offloads.every(m => Number(m[1]) > 0), 'real GPU offload is required');
                    assert.ok(offloads.every(m => m[1] === m[2]), 'reviewed scope requires complete GPU offload');
                    const types = [...log.matchAll(/K \((\w+)\):[^\n]*V \((\w+)\):/g)];
                    assert.ok(types.some(m => m[1] === kv && m[2] === kv), 'requested target KV types not observed');
                    report.owned_pids.push(h.running.pid);
                    return { pid: h.running.pid, checkpoint: c, observed, gpu_offloads: offloads.map(m => m[0]) };
                }
            }
            catch (error) {
                if (error.code === 'ERR_ASSERTION')
                    throw error;
            }
        }
        if (Date.now() > until)
            throw Error(`not ready: ${JSON.stringify({ h, c })}`);
        await sleep(500);
    }
}
async function stop() {
    const running = (await client.call('instances.health', { id })).running;
    if (running && ownedProcesses().some(p => p.ProcessId === running.pid)) {
        try {
            report.process_memory = [...(report.process_memory || []), processMemory(running.pid)];
        } catch (error) {
            // Diagnostic sampling must never prevent the requested stop/cleanup.
            // Keep the gap explicit; host telemetry is collected separately.
            report.process_memory_errors = [...(report.process_memory_errors || []), { pid: running.pid, message: error.message }];
        }
    }
    const result = await client.mutate('instances.stop', { id });
    assert.equal((await client.call('instances.health', { id })).running, null);
    if (running) {
        assert.ok(!ownedProcesses().some(p => p.ProcessId === running.pid), 'old engine PID must exit before a new process is launched');
        report.stop_verifications = [...(report.stop_verifications || []), { pid: running.pid, exited: true, checked_at: new Date().toISOString() }];
        const log = path.join(data, 'configs', 'logs', `${id}.log`);
        if (fs.existsSync(log)) record('native-speculation-trace', { pid: running.pid, trace: speculativeTrace(fs.readFileSync(log, 'utf8')) });
    }
    return result;
}
function killOwned(pid) {
    assert.ok(report.owned_pids.includes(pid));
    assert.ok(ownedProcesses().some(p => p.ProcessId === pid), 'refuse to terminate a PID outside this validation root');
    cp.execFileSync('taskkill.exe', ['/PID', String(pid), '/T', '/F'], { windowsHide: true });
}
const sentinel = slot => {
    const answer = [391, 437, 493, 551][slot];
    return `Slot ${slot}. Repeat ${answer} separated by spaces. Answer: ${answer} ${answer} ${answer} `;
};
const prompt = (slot, count = 140) => Array.from({ length: count }, (_, i) => `Record ${i}: the answer for slot ${slot} is ${[391, 437, 493, 551][slot]}.\n`).join('') + sentinel(slot);
const marker = slot => String([391, 437, 493, 551][slot]);
const checkpointRoot = path.join(data, 'kv-checkpoints', id);
function generation() {
    const pointerFile = walk(checkpointRoot).find(f => path.basename(f) === 'latest.json');
    assert.ok(pointerFile, 'no published generation');
    const pointerBytes = fs.readFileSync(pointerFile), pointer = JSON.parse(pointerBytes);
    const dir = path.join(path.dirname(pointerFile), 'generations', pointer.generationId);
    const manifestFile = path.join(dir, 'manifest.json'), manifest = JSON.parse(fs.readFileSync(manifestFile));
    assert.equal(manifest.schemaVersion, 2);
    assert.equal(manifest.slotCount, parallel);
    assert.equal(sha(manifestFile), pointer.manifestSha256);
    return { pointerFile, pointerBytes, pointer, dir, manifestFile, manifest };
}
function verifyFiles(g) {
    return g.manifest.slots.map(s => {
        const file = path.join(g.dir, s.filename), format = inspectSlotFile(file);
        assert.equal(sha(file), s.sha256);
        assert.equal(format.bytes, s.bytes);
        assert.ok(format.liveDraft?.state.bytes > 0 && format.checkpoints.length > 0);
        return { slot: s.id, sha256: s.sha256, bytes: s.bytes, promptTokens: s.promptTokens, format };
    });
}
async function seedAll() {
    const states = [];
    for (let slot = 0; slot < parallel; slot++) {
        const p = prompt(slot), prefill = await completion(slot, p, 24);
        assert.ok(prefill.content.includes(marker(slot)));
        const filename = `reference-${slot}.bin`;
        await request(`/slots/${slot}?action=save`, { filename });
        const continuation = p + prefill.content, live = await completion(slot, continuation);
        await request(`/slots/${slot}?action=restore`, { filename });
        states.push({ slot, continuation, live });
    }
    return states;
}
async function verifyAll(states, g, label) {
    const rows = [];
    for (const { slot, continuation, live } of states) {
        const actual = await completion(slot, continuation);
        assertInference(live, actual, { draft: true, reuse: true, minimumReuse: g.manifest.slots.find(s => s.id === slot).promptTokens, marker: marker(slot) });
        rows.push({ slot, live, actual });
    }
    record(label, { rows });
}
async function main() {
    report.manager_sha256 = sha(path.join(args.bin, 'llama-server-manager.exe'));
    report.cli_sha256 = sha(path.join(args.bin, 'lsm.exe'));
    await new Promise((resolve, reject) => { const p = require('node:net').createServer(); p.once('error', reject); p.listen({ host: '127.0.0.1', port, exclusive: true }, () => p.close(resolve)); });
    assert.equal(sha(args.engine), profile.launcher_sha256);
    report.profile = profile.id;
    report.model_sha256 = sha(args.model);
    report.draft_sha256 = sha(args.mtp);
    await client.call('service.start');
    for (const [purpose, directory] of [['engine', path.dirname(args.engine)], ...new Set([args.model, args.mtp].map(p => path.dirname(p)))].map(row => Array.isArray(row) ? row : ['model', row])) {
        await client.mutate('paths.authorize', { purpose, path: directory });
        await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision });
    }
    await client.mutate('models.refresh');
    await client.mutate('engines.refresh');
    const norm = p => path.toNamespacedPath(p).toLowerCase();
    const engine = (await client.call('engines.list')).engines.find(e => norm(e.exe) === norm(args.engine));
    assert.ok(engine);
    const probed = await client.mutate('engines.probe', { id: engine.id });
    assert.equal(probed.capabilities.persistenceProfile, profile.id);
    record('fixed-build-profile', probed);
    await update({ name: 'Isolated multi-slot acceptance', engine_id: engine.id, model_path: args.model, draft_model_path: args.mtp,
        host: '127.0.0.1', port, launch_mode: 'managed', ctx_size: context, ctx_size_auto: false,
        cache_type_k: kv, cache_type_v: kv, cache_type_draft_k: kv, cache_type_draft_v: kv, kv_unified: true, kv_unified_mode: 'on',
        gpu_layers: 99, gpu_layers_auto: false, draft_gpu_layers: 99, draft_tokens: 3, flash_attn: 'on', fit_mode: 'off',
        warmup: false, parallel, cache_prompt: true, cache_idle_slots: false, cache_ram: 256, ctx_checkpoints: 2,
        slots_enabled: true, metrics: true, props: true, verbose: true, mmproj_auto: false, spec_type: specType,
        kv_checkpoint: { enabled: true, allow_local_validation: localValidation, auto_save: true, auto_restore: true, idle_save_seconds: 0, preserve_longer: true, storage_limit_gib: 32, minimum_prompt_tokens: 16 } });
    if (localValidation) record('local-qualification-receipt', await client.mutate('checkpoints.validate', { id }));
    const eligible = await client.call('checkpoints.eligibility', { id });
    assert.equal(eligible.eligible, true, JSON.stringify(eligible));
    assert.equal(eligible.slot_count, parallel);
    record('qualified-topology', eligible);
    sampler = memorySampler(client, id);
    report.memory = sampler.summary;
    const original = await start(), states = await seedAll();
    await stop();
    const good = generation();
    assert.equal(good.manifest.slots.length, parallel);
    const formats = verifyFiles(good);
    record('all-slots-automatic-stop-save', { original, generation: good.manifest, formats });
    let fresh = await start();
    assert.notEqual(fresh.pid, original.pid);
    assert.equal(fresh.checkpoint.phase, 'ready');
    await verifyAll(states, good, 'fresh-process-all-slots-restored-inference');
    // Deny the final atomic pointer replacement, after all native slot saves.
    // This is a test-owned file; no production ACL or configuration is touched.
    fs.chmodSync(good.pointerFile, 0o444);
    try { await stop(); }
    finally { fs.chmodSync(good.pointerFile, 0o666); }
    const denied = await client.call('checkpoints.status', { id });
    assert.equal(denied.last_outcome, 'failed', JSON.stringify(denied));
    assert.deepEqual(fs.readFileSync(good.pointerFile), good.pointerBytes);
    verifyFiles(good);
    record('denied-latest-replacement-retains-previous-generation', { denied });
    await update({ kv_checkpoint: { auto_save: false } });
    // Corrupt only the LAST sequence. Earlier slots have already been restored
    // when native validation fails. Rehash only this disposable test generation.
    for (const kind of ['invalid-draft-version', 'truncated-main']) {
        const bad = structuredClone(good.manifest), badId = crypto.randomUUID(), badDir = path.join(path.dirname(good.dir), badId);
        fs.mkdirSync(badDir);
        bad.generationId = badId;
        bad.createdAt = new Date().toISOString();
        for (const s of bad.slots)
            fs.copyFileSync(path.join(good.dir, s.filename), path.join(badDir, s.filename));
        const s = bad.slots.at(-1), file = path.join(badDir, s.filename), format = formats.at(-1).format;
        if (kind === 'truncated-main')
            fs.truncateSync(file, Math.floor(format.mainBytes / 2));
        else {
            const fd = fs.openSync(file, 'r+'), b = Buffer.alloc(4);
            b.writeUInt32LE(999);
            try {
                fs.writeSync(fd, b, 0, 4, format.liveDraft.offset + 4);
                fs.fsyncSync(fd);
            }
            finally {
                fs.closeSync(fd);
            }
        }
        s.bytes = fs.statSync(file).size;
        s.sha256 = sha(file);
        const mf = path.join(badDir, 'manifest.json');
        fs.writeFileSync(mf, JSON.stringify(bad));
        fs.writeFileSync(good.pointerFile, JSON.stringify({ ...good.pointer, generationId: badId, manifestSha256: sha(mf) }));
        fresh = await start();
        assert.equal(fresh.checkpoint.reason_code, 'cold_after_restore_failure');
        const rejected = fresh.observed.filter(p => p.pid !== fresh.pid);
        assert.ok(rejected.length > 0, 'failed native PID must be observed');
        for (const p of rejected) {
            assert.equal(p.ever_routable, false);
            assert.ok(!ownedProcesses().some(q => q.ProcessId === p.pid));
        }
        assert.ok(fs.existsSync(path.join(badDir, 'restore-rejected.json')));
        const actual = [];
        for (let slot = 0; slot < parallel; slot++) {
            const r = await completion(slot, sentinel(slot));
            assert.ok(r.content.includes(marker(slot)));
            assert.ok(r.timings.draft_n_accepted > 0);
            actual.push({ slot, result: r });
        }
        assert.equal(JSON.parse(fs.readFileSync(path.join(data, 'runtime', 'runtime-state.json'))).desired_instances[id].config.kv_checkpoint.auto_restore, true);
        verifyFiles(good);
        record('partial-restore-entire-process-isolated', { kind, fresh, rejected, actual });
        await stop();
        fs.writeFileSync(good.pointerFile, good.pointerBytes);
        fresh = await start();
        assert.equal(fresh.checkpoint.phase, 'ready');
        await verifyAll(states, good, `fresh-process-good-after-${kind}`);
        await stop();
    }
    // Save while idle; a short request in slot zero must not discard its longer
    // disk state or prevent slot N-1 from publishing a newer long conversation.
    await update({ kv_checkpoint: { auto_save: true, idle_save_seconds: 15, preserve_longer: true } });
    fresh = await start();
    const busyPointer = sha(good.pointerFile);
    const busyRequest = completion(parallel - 1, prompt(parallel - 1), 1536);
    let busyFinished = false;
    busyRequest.then(() => { busyFinished = true; }, () => { busyFinished = true; });
    const busyUntil = Date.now() + 18000;
    let observedBusy = false;
    while (Date.now() < busyUntil) {
        const slots = await request('/slots');
        if (slots.some(s => s.is_processing))
            observedBusy = true;
        if (busyFinished)
            break;
        assert.equal(sha(good.pointerFile), busyPointer, 'an occupied slot must block the entire idle save');
        await sleep(1000);
    }
    assert.ok(observedBusy);
    const busyResult = await busyRequest;
    record('nonzero-busy-slot-blocks-idle-save', { slot: parallel - 1, result: busyResult });
    await completion(0, 'Write one short title. Title:', 12);
    const last = parallel - 1, p = prompt(last, 170), prefill = await completion(last, p, 24), filename = 'idle-reference.bin';
    await request(`/slots/${last}?action=save`, { filename });
    const continuation = p + prefill.content, live = await completion(last, continuation);
    await request(`/slots/${last}?action=restore`, { filename });
    let idle, deadline = Date.now() + 600000;
    for (;;) {
        idle = await client.call('checkpoints.status', { id });
        if (idle.last_operation === 'save' && idle.last_outcome === 'success' && idle.routable && idle.generation_id !== good.manifest.generationId)
            break;
        assert.ok(Date.now() < deadline, JSON.stringify(idle));
        await sleep(1000);
    }
    const retained = generation();
    assert.equal(retained.manifest.slots[0].sha256, good.manifest.slots[0].sha256);
    assert.ok(retained.manifest.slots.at(-1).promptTokens > good.manifest.slots.at(-1).promptTokens);
    assert.equal((await client.call('instances.health', { id })).running.pid, fresh.pid);
    verifyFiles(good);
    record('idle-per-slot-prefix-protection', { fresh, idle, manifest: retained.manifest });
    killOwned(fresh.pid);
    await sleep(1500);
    await stop();
    assert.equal(sha(retained.pointerFile), crypto.createHash('sha256').update(retained.pointerBytes).digest('hex'));
    await update({ kv_checkpoint: { auto_save: false, idle_save_seconds: 0 } });
    fresh = await start();
    assert.equal(fresh.checkpoint.phase, 'ready');
    await verifyAll([states[0], { slot: last, continuation, live }], retained, 'fresh-process-idle-save-after-force-exit');
    await stop();
    // A normal controller shutdown saves all slots, and both old engine and
    // controller must be gone before the next controller starts.
    await update({ kv_checkpoint: { auto_save: true, preserve_longer: false } });
    fresh = await start();
    const shutdownStates = await seedAll();
    await shutdownService();
    const shutdownGeneration = generation();
    verifyFiles(shutdownGeneration);
    await client.call('service.start');
    await update({ kv_checkpoint: { auto_save: false } });
    const newService = await start();
    assert.notEqual(newService.pid, fresh.pid);
    assert.equal(newService.checkpoint.phase, 'ready');
    await verifyAll(shutdownStates, shutdownGeneration, 'fresh-process-controller-shutdown-restore');
    await stop();
    // Empty slot zero is not a reason to discard a useful state in the last slot.
    await client.mutate('checkpoints.clear', { id });
    await update({ kv_checkpoint: { auto_save: true, preserve_longer: false } });
    fresh = await start();
    const sparsePrompt = prompt(last), sparsePrefill = await completion(last, sparsePrompt, 24);
    await request(`/slots/${last}?action=save`, { filename: 'sparse-reference.bin' });
    const sparseContinuation = sparsePrompt + sparsePrefill.content, sparseLive = await completion(last, sparseContinuation);
    await request(`/slots/${last}?action=restore`, { filename: 'sparse-reference.bin' });
    await stop();
    const sparse = generation();
    assert.deepEqual(sparse.manifest.slots.map(s => s.id), [last]);
    await update({ kv_checkpoint: { auto_save: false } });
    fresh = await start();
    assert.equal(fresh.checkpoint.phase, 'ready');
    await verifyAll([{ slot: last, continuation: sparseContinuation, live: sparseLive }], sparse, 'fresh-process-sparse-generation');
    await stop();
    // Exact topology and model/config qualifications must fail closed.
    for (const patch of [{ cache_idle_slots: true }, { parallel: 3 }, { ctx_size: context + 1024 }, { draft_model_path: args.model }, { cache_type_draft_k: 'q8_0' }, { ctx_checkpoints: 0 },
        { spec_type: 'ngram-cache,draft-mtp' }, { spec_type: 'ngram-mod,draft-dflash' }, { custom_args: ['--spec-ngram-mod-n-max 32'] }]) {
        await update({ parallel, ctx_size: context, draft_model_path: args.mtp, cache_type_draft_k: kv, ctx_checkpoints: 2, cache_idle_slots: false, spec_type: specType, custom_args: [], ...patch });
        const e = await client.call('checkpoints.eligibility', { id });
        assert.equal(e.eligible, false);
        record('unqualified-shape-rejected', { patch, eligibility: e });
    }
    report.passed = true;
}
main().catch(e => { report.error = e.stack; report.passed = false; process.exitCode = 1; console.error(e); }).finally(async () => {
    await sampler?.stop();
    try {
        await stop();
    }
    catch (e) {
        report.cleanup_stop_error = e.message;
    }
    try {
        await shutdownService();
    }
    catch (e) {
        report.cleanup_service_error = e.message;
        report.passed = false;
        process.exitCode = 1;
    }
    for (const p of ownedProcesses()) {
        report.passed = false;
        process.exitCode = 1;
        report.forced_cleanup_pids = [...(report.forced_cleanup_pids || []), p.ProcessId];
        try {
            if (ownedProcesses().some(q => q.ProcessId === p.ProcessId))
                cp.execFileSync('taskkill.exe', ['/PID', String(p.ProcessId), '/T', '/F'], { windowsHide: true });
        }
        catch (e) {
            report.cleanup_error = e.message;
        }
    }
    report.processes_after_cleanup = ownedProcesses();
    report.finished = new Date().toISOString();
    record('finished');
});
