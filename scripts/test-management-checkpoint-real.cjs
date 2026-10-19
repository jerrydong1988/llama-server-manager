// Real-engine checkpoint round trip through CLI/MCP and the managed router.
// --bin DIR --root DIR --engine FILE --model FILE [--gpu-layers 0] [--id checkpoint-real]
const fs = require('node:fs')
const path = require('node:path')
const assert = require('node:assert/strict')
const { managementClient, sleep } = require('./lib/management-client.cjs')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + 2 * i].slice(2), process.argv[3 + 2 * i]]))
for (const key of ['bin', 'root', 'engine', 'model']) assert.ok(args[key] && path.isAbsolute(args[key]), `--${key} must be absolute`)
const extension = process.platform === 'win32' ? '.exe' : ''
const data = path.join(args.root, 'data'), id = args.id || 'checkpoint-real'
const client = managementClient({ cli: path.join(args.bin, `lsm${extension}`), app: path.join(args.bin, `llama-server-manager${extension}`), data, timeout: 600000 })
const reportPath = path.join(args.root, `${id}.json`)
const report = { started_at: new Date().toISOString(), model: args.model, engine: args.engine, checks: [] }
const record = (name, details = {}) => { report.checks.push({ name, ...details }); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2)); console.log(JSON.stringify({ name, ...details })) }
let mcp
async function ready() {
  const deadline = Date.now() + 180000
  while (Date.now() < deadline) {
    const s = await mcp.tool('checkpoints.status', { id })
    if (s?.routable) return s
    await sleep(500)
  }
  throw new Error('Checkpoint readiness did not arrive')
}
const prompt = 'Once upon a time, there was a little girl named Lily. She lived near a beautiful forest. Every morning she walked with her dog and looked for flowers. '.repeat(args.id === 'checkpoint-tiny' ? 3 : 15)
async function completion() {
  const response = await fetch('http://127.0.0.1:18185/v1/completions', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ model: 'checkpoint-test', prompt, max_tokens: 16, temperature: 0, seed: 42, cache_prompt: true, id_slot: 0 }), signal: AbortSignal.timeout(180000) })
  const body = await response.json()
  assert.equal(response.status, 200, JSON.stringify(body))
  assert.ok(body.usage?.prompt_tokens > 64)
  return body
}
async function start() { await client.mutate('instances.start', { id }); return ready() }
async function stop() { await mcp.mutate('instances.stop', { id }); return mcp.tool('checkpoints.status', { id }) }
function walk(dir) { return fs.existsSync(dir) ? fs.readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory() ? walk(path.join(dir,e.name)) : [path.join(dir,e.name)]) : [] }
async function main() {
  await client.call('service.start')
  for (const [purpose, directory] of [['model', path.dirname(args.model)], ['engine', path.dirname(args.engine)]]) {
    await client.mutate('paths.authorize', { purpose, path: directory })
    await client.mutate('directories.add', { purpose, path: directory, revision: (await client.call('service.status')).revision })
  }
  await client.mutate('models.refresh')
  await client.mutate('engines.refresh')
  const engine = (await client.call('engines.list')).engines.find(e => path.basename(e.exe).toLowerCase() === path.basename(args.engine).toLowerCase())
  assert.ok(engine); await client.mutate('engines.probe', { id: engine.id })
  const config = { name: `Real KV - ${path.basename(args.model)}`, engine_id: engine.id, model_path: args.model, host: '127.0.0.1', port: 18184,
    launch_mode: 'managed', ctx_size: 2048, ctx_size_auto: false, gpu_layers: Number(args['gpu-layers'] || 0), gpu_layers_auto: false,
    fit_mode: 'off', warmup: false, parallel: 1, cache_prompt: true, cache_idle_slots: true, cache_ram: 256, ctx_checkpoints: 64,
    slots_enabled: true, metrics: true, props: true, mmproj_auto: false,
    kv_checkpoint: { enabled: true, auto_save: true, auto_restore: true, storage_limit_gib: 1, minimum_prompt_tokens: 64 } }
  const current = await client.call('instances.get', { id }, { allowFailure: true })
  await client.mutate(current.ok ? 'instances.update' : 'instances.create', { id, revision: (await client.call('service.status')).revision, [current.ok ? 'patch' : 'config']: config })
  mcp = await client.mcp()
  const eligibility = await mcp.tool('checkpoints.eligibility', { id })
  record('eligibility', eligibility)
  assert.equal(eligibility.eligible, true, JSON.stringify(eligibility))
  await mcp.mutate('checkpoints.clear', { id })
  await client.mutate('routes.update', { revision: (await client.call('routes.get')).revision, patch: { host: '127.0.0.1', port: 18185, default_instance_id: id, strict_model_routing: true, health_check_interval_ms: 1000, routes: [{ id: 'checkpoint-route', enabled: true, model_alias: 'checkpoint-test', target_instance_id: id, priority: 0, weight: 1, max_concurrent_requests: 1 }] } })
  const cold = await start()
  await mcp.mutate('routes.start')
  const first = await completion()
  const firstPid = (await client.call('instances.health', { id })).running.pid
  record('cold-inference', { status: cold, usage: first.usage, output: first.choices[0].text, pid: firstPid })
  const forbidden = await mcp.mutate('checkpoints.clear', { id }, true)
  assert.equal(forbidden.status, 'failed')
  const saved = await stop()
  assert.equal(saved.last_operation, 'save'); assert.equal(saved.last_outcome, 'success', JSON.stringify(saved)); assert.ok(saved.bytes > 0)
  record('controlled-stop-saves-real-slot-state', saved)
  const restored = await start()
  assert.equal(restored.last_operation, 'restore'); assert.equal(restored.last_outcome, 'success', JSON.stringify(restored))
  const secondPid = (await client.call('instances.health', { id })).running.pid
  assert.notEqual(firstPid, secondPid)
  const second = await completion()
  assert.equal(second.choices[0].text, first.choices[0].text, 'restored deterministic completion must match cold completion')
  assert.ok(second.usage?.prompt_tokens_details?.cached_tokens > 0, JSON.stringify(second.usage))
  let observed
  for (let i = 0; i < 30; i++) {
    observed = await mcp.tool('checkpoints.status', { id })
    if (observed.reuse_observation?.cached_tokens > 0) break
    await sleep(500)
  }
  // Tiny requests can finish between monitor samples. The response is the exact
  // first-request evidence; a missing sample must remain unavailable, not zero.
  if (observed.reuse_observation) assert.ok(observed.reuse_observation.cached_tokens > 0, JSON.stringify(observed))
  record('cross-process-restore-and-first-request-cache-reuse', { status: observed, usage: second.usage, output: second.choices[0].text, old_pid: firstPid, new_pid: secondPid })
  // Preserve a rollback copy, then corrupt only this test instance's saved payload.
  const savedAgain = await stop()
  const checkpointRoot = path.join(data, 'kv-checkpoints', id)
  const payloads = walk(checkpointRoot).filter(file => file.endsWith('.bin') && file.includes(savedAgain.generation_id))
  assert.equal(payloads.length, 1, JSON.stringify(walk(checkpointRoot)))
  const payload = payloads[0]
  fs.copyFileSync(payload, path.join(args.root, `${id}-payload-rollback.bin`))
  const handle = fs.openSync(payload, 'r+')
  try { const byte = Buffer.alloc(1); fs.readSync(handle, byte, 0, 1, 0); byte[0] ^= 0xff; fs.writeSync(handle, byte, 0, 1, 0); fs.fsyncSync(handle) } finally { fs.closeSync(handle) }
  const rejected = await start()
  assert.notEqual(rejected.generation_id, savedAgain.generation_id, 'corrupt generation must never be restored')
  await completion()
  record('corrupt-newest-checkpoint-rejected-and-older-valid-generation-used', rejected)
  await stop()
  const rollback = path.join(args.root, `${id}-payloads-rollback`)
  fs.mkdirSync(rollback, { recursive: true })
  for (const candidate of walk(checkpointRoot).filter(file => file.endsWith('.bin'))) {
    const backup = path.join(rollback, path.relative(checkpointRoot, candidate))
    fs.mkdirSync(path.dirname(backup), { recursive: true }); fs.copyFileSync(candidate, backup)
    const handle = fs.openSync(candidate, 'r+')
    try { fs.writeSync(handle, Buffer.from('INVALID!'), 0, 8, 0); fs.fsyncSync(handle) } finally { fs.closeSync(handle) }
  }
  const coldFallback = await start()
  assert.notEqual(coldFallback.last_outcome, 'success', JSON.stringify(coldFallback))
  assert.equal(coldFallback.routable, true)
  await completion()
  record('no-valid-generation-falls-back-to-real-cold-inference', coldFallback)
  await stop()
  const revision = (await client.call('instances.get', { id })).revision
  await client.mutate('instances.update', { id, revision, patch: { ctx_size: 1024 } })
  const incompatible = await start()
  assert.notEqual(incompatible.last_outcome, 'success')
  record('configuration-change-does-not-reuse-incompatible-checkpoint', incompatible)
  await stop()
  await mcp.mutate('checkpoints.clear', { id })
  assert.equal((await mcp.tool('checkpoints.status', { id })).last_operation, 'clear')
  await client.mutate('routes.stop')
  report.status = 'passed'
}
main().catch(error => { report.status = 'failed'; report.error = error.stack; console.error(error); process.exitCode = 1 })
  .finally(async () => {
    try { await client.mutate('instances.stop', { id }); await client.mutate('routes.stop') } catch {}
    if (mcp) await mcp.close()
    report.finished_at = new Date().toISOString(); fs.writeFileSync(reportPath, JSON.stringify(report, null, 2))
  })
