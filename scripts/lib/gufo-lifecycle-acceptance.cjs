const fs = require('node:fs')
const path = require('node:path')
const net = require('node:net')
const assert = require('node:assert/strict')
const { sleep } = require('./management-client.cjs')

async function gufoLifecycleAcceptance({ client, mcp, id, port, model, data, record, owned, crash, authKey = '' }) {
  async function ready() {
    for (let i = 0; i < 660; i++) {
      const health = await mcp.tool('instances.health', { id })
      if (health.running?.pid) owned.add(health.running.pid)
      if (health.health === 'ok') return health.running.pid
      assert.ok(health.running, `Engine exited during restart: ${JSON.stringify(health)}`)
      assert.notEqual(health.health, 'fail'); await sleep(1000)
    }
    throw new Error('Gufo restart timed out')
  }
  async function complete() {
    const response = await fetch(`http://127.0.0.1:${port}/v1/chat/completions`, { method: 'POST',
      headers: { 'content-type': 'application/json', ...(authKey ? { authorization: `Bearer ${authKey}` } : {}) }, signal: AbortSignal.timeout(180000),
      body: JSON.stringify({ model, temperature: 0, seed: 42, reasoning_effort: 'none', max_tokens: 32,
        messages: [{ role: 'user', content: `${'A red fox lives near a green forest and visits the river each morning. '.repeat(150)}\nAnswer only this arithmetic question: 6 times 7?` }] }) })
    const body = await response.json(); assert.equal(response.status, 200, JSON.stringify(body))
    assert.match(body.choices[0].message.content, /42/)
    return body
  }
  const initialPid = (await mcp.tool('instances.health', { id })).running.pid
  const cold = await complete()
  await client.mutate('instances.stop', { id })
  const cache = path.join(data, 'gufo', id, 'snapshots')
  const files = fs.readdirSync(cache, { recursive: true }).filter(name => fs.statSync(path.join(cache, name)).isFile())
  assert.ok(files.length > 0, 'Gufo did not publish disk snapshots during stop')
  await client.mutate('instances.start', { id })
  const restartedPid = await ready()
  assert.notEqual(restartedPid, initialPid)
  const warm = await complete()
  assert.ok(warm.usage.prompt_tokens_details?.cached_tokens > 0, JSON.stringify(warm.usage))
  assert.equal(warm.choices[0].message.content, cold.choices[0].message.content)
  record('gufo_disk_cache_process_restart', { id, initialPid, restartedPid, cacheFiles: files.length, coldUsage: cold.usage, warmUsage: warm.usage })
  // Exercise window attachment separately from the inference process lifetime.
  await client.mutate('service.show'); await sleep(1500)
  await client.mutate('service.hide'); await client.mutate('service.show'); await sleep(1000)
  assert.equal((await mcp.tool('instances.health', { id })).running.pid, restartedPid)
  await client.mutate('service.hide')
  record('gufo_window_reopen_preserves_process', { id, pid: restartedPid })
  if (crash) {
    // This PID was obtained from our isolated management host, never by process name.
    process.kill(restartedPid)
    for (let i = 0; ; i++) {
      if (!(await mcp.tool('instances.health', { id })).running) break
      assert.ok(i < 30, 'Manager did not observe the owned engine exit'); await sleep(1000)
    }
    await client.mutate('instances.start', { id }); const recoveredPid = await ready()
    await complete()
    record('gufo_abnormal_exit_and_explicit_recovery', { id, recoveredPid })
  }
  await client.mutate('instances.stop', { id })
  const blocker = net.createServer(socket => socket.destroy())
  await new Promise((resolve, reject) => { blocker.once('error', reject); blocker.listen(port, '127.0.0.1', resolve) })
  try {
    const result = await client.call('instances.start', { id }, { mutate: true, allowFailure: true })
    if (result.ok && result.data?.status === 'succeeded') {
      for (let i = 0; i < 30 && (await mcp.tool('instances.health', { id })).running; i++) await sleep(1000)
    }
    assert.equal((await mcp.tool('instances.health', { id })).running, null)
    assert.ok(blocker.listening, 'port owner must remain alive')
    record('gufo_port_conflict_preserves_owner', { id, result })
  } finally { await new Promise(resolve => blocker.close(resolve)) }
  await client.mutate('instances.start', { id })
  await sleep(500)
  await client.mutate('instances.stop', { id })
  assert.equal((await mcp.tool('instances.health', { id })).running, null)
  record('gufo_stop_during_loading', { id })
}
module.exports = { gufoLifecycleAcceptance }
