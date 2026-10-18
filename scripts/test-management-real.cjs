// Opt-in local hardware test. Reads existing model/engine files, uses disposable configuration.
const fs = require('node:fs')
const path = require('node:path')
const os = require('node:os')
const cp = require('node:child_process')
const net = require('node:net')
const assert = require('node:assert/strict')
const engine = process.env.LSM_REAL_ENGINE
const model = process.env.LSM_REAL_MODEL
if (!engine || !model || !fs.existsSync(engine) || !fs.existsSync(model)) throw new Error('Set LSM_REAL_ENGINE and LSM_REAL_MODEL to existing absolute paths')
const profile = process.env.LSM_TEST_PROFILE || 'debug'
const extension = process.platform === 'win32' ? '.exe' : ''
const bin = path.resolve(__dirname, '../src-tauri/target', profile)
const data = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-management-real-'))
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
let hostPid
async function call(op, args = {}, wait = false) {
  const output = await new Promise((resolve, reject) => cp.execFile(path.join(bin, `lsm${extension}`), ['--app', path.join(bin, `llama-server-manager${extension}`), '--data-dir', data, ...op.split('.'), '--args', JSON.stringify(args), ...(wait ? ['--wait', '--timeout', '180'] : [])], { windowsHide: true, timeout: 210000, maxBuffer: 8 * 1024 * 1024 }, (error, stdout, stderr) => error ? reject(new Error(`${op}: ${stdout || stderr || error.message}`)) : resolve(stdout)))
  const response = JSON.parse(output)
  assert.equal(response.ok, true)
  return wait ? response.data.result : response.data
}
async function run() {
  hostPid = (await call('service.start')).pid
  for (const [purpose, directory] of [['engine', path.dirname(engine)], ['model', path.dirname(model)]]) {
    await call('paths.authorize', { purpose, path: directory }, true)
    await call('directories.add', { purpose, path: directory, revision: (await call('service.status')).revision }, true)
  }
  await call('engines.refresh', {}, true)
  const inventory = await call('engines.list')
  const chosen = inventory.engines.find(item => path.basename(item.exe).toLowerCase() === path.basename(engine).toLowerCase())
  assert.ok(chosen)
  const probe = await call('engines.probe', { id: chosen.id }, true)
  console.log(JSON.stringify({ stage: 'engine-probed', engine: path.basename(engine), capability_result: !!probe }))
  const server = net.createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  await call('instances.create', { id: 'real-validation', revision: (await call('service.status')).revision, config: { name: 'Real CLI validation', engine_id: chosen.id, model_path: model, launch_mode: 'manual', port, host: '127.0.0.1', manual_command: `"${engine}" --model "${model}" --host 127.0.0.1 --ctx-size 2048 --n-gpu-layers 99 --fit off --no-warmup --metrics --slots` } }, true)
  await call('instances.start', { id: 'real-validation' }, true)
  const started = Date.now()
  for (;;) {
    const response = await fetch(`http://127.0.0.1:${port}/health`, { signal: AbortSignal.timeout(3000) }).catch(() => null)
    if (response?.ok) break
    if (Date.now() - started > 150000) throw new Error('real engine did not become healthy')
    await sleep(1000)
  }
  const response = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ model: 'test', messages: [{ role: 'user', content: 'Reply with the single word OK.' }], max_tokens: 16, temperature: 0, chat_template_kwargs: { enable_thinking: false } }), signal: AbortSignal.timeout(120000) })
  const completion = await response.json()
  assert.equal(response.ok, true, JSON.stringify(completion))
  assert.ok(completion.choices?.[0]?.message?.content?.trim(), JSON.stringify(completion))
  assert.ok(completion.usage?.completion_tokens > 0)
  const health = await call('instances.health', { id: 'real-validation' })
  assert.ok(health.running?.pid > 0)
  console.log(JSON.stringify({ stage: 'real-inference-passed', model: path.basename(model), content: completion.choices[0].message.content, usage: completion.usage, elapsed_ms: Date.now() - started, manual_fallback_port: port, runtime_pid: health.running.pid }))
  await call('instances.stop', { id: 'real-validation' }, true)
}
run().catch(error => { console.error(error); process.exitCode = 1 }).finally(async () => {
  try { await call('service.stop', {}, true); await sleep(3500) } catch (error) { console.error(error.message) }
  if (hostPid) { try { process.kill(hostPid) } catch {} }
  if (process.exitCode || process.env.LSM_KEEP_TEST_DATA === '1') console.log(`Retained real test data: ${data}`)
  else {
    assert.ok(path.resolve(data).startsWith(path.resolve(os.tmpdir()) + path.sep + 'lsm-management-real-'))
    fs.rmSync(data, { recursive: true, force: true, maxRetries: 20, retryDelay: 300 })
  }
})
