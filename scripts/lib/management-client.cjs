// Real CLI and stdio MCP transports for opt-in desktop/engine validation.
const cp = require('node:child_process')
const readline = require('node:readline')
const assert = require('node:assert/strict')
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))

function managementClient({ cli, app, data, timeout = 240000 }) {
  const prefix = ['--app', app, '--data-dir', data]
  async function call(op, args = {}, { mutate = false, allowFailure = false } = {}) {
    const response = await new Promise((resolve, reject) => {
      cp.execFile(cli, [...prefix, ...op.split('.'), '--args', JSON.stringify(args), ...(mutate ? ['--wait', '--timeout', String(Math.ceil(timeout / 1000))] : [])],
        { windowsHide: true, timeout: timeout + 5000, maxBuffer: 8 * 1024 * 1024 }, (error, stdout, stderr) => {
          try { resolve(JSON.parse(stdout || stderr)) } catch { reject(new Error(`${op}: ${error?.message || stderr || stdout}`)) }
        })
    })
    if (!allowFailure) assert.equal(response.ok, true, `${op}: ${JSON.stringify(response)}`)
    if (allowFailure) return response
    return mutate ? response.data.result : response.data
  }
  async function mcp(permissions = ['--allow-write', '--allow-delete']) {
    const child = cp.spawn(cli, [...prefix, 'mcp', ...permissions], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] })
    const pending = new Map()
    let sequence = 0, diagnostics = ''
    child.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk).slice(-4096) })
    const rejectPending = error => {
      for (const entry of pending.values()) { clearTimeout(entry.timer); entry.reject(error) }
      pending.clear()
    }
    child.on('error', rejectPending)
    child.on('exit', code => rejectPending(new Error(`MCP exited ${code}: ${diagnostics}`)))
    const lines = readline.createInterface({ input: child.stdout })
    lines.on('line', line => {
      try {
        const response = JSON.parse(line)
        const entry = pending.get(response.id)
        if (entry) { pending.delete(response.id); clearTimeout(entry.timer); entry.resolve(response) }
      } catch (error) { rejectPending(error) }
    })
    const request = (method, params) => new Promise((resolve, reject) => {
      const id = ++sequence
      const timer = setTimeout(() => { pending.delete(id); reject(new Error(`MCP ${method} timed out`)) }, timeout)
      pending.set(id, { resolve, reject, timer })
      child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, ...(params ? { params } : {}) }) + '\n')
    })
    await request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'lsm-cross-validation', version: '1' } })
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n')
    const tool = async (op, args = {}, allowFailure = false) => {
      const response = await request('tools/call', { name: op.replaceAll('.', '_'), arguments: args })
      if (allowFailure) return response
      assert.equal(response.error, undefined, JSON.stringify(response))
      assert.equal(response.result?.structuredContent?.ok, true, JSON.stringify(response))
      return response.result.structuredContent.data
    }
    return {
      request, tool,
      mutate: async (op, args = {}, allowFailure = false) => {
        const task = await tool(op, args)
        assert.ok(task.task_id, JSON.stringify(task))
        const deadline = Date.now() + timeout
        for (;;) {
          const result = await tool('tasks.get', { id: task.task_id })
          if (!['queued', 'running'].includes(result.status)) {
            if (!allowFailure) assert.equal(result.status, 'succeeded', JSON.stringify(result))
            return allowFailure ? result : result.result
          }
          if (Date.now() >= deadline) throw new Error(`MCP task ${task.task_id} did not complete`)
          await sleep(100)
        }
      },
      close: async () => {
        child.stdin.end()
        await Promise.race([new Promise(resolve => child.once('exit', resolve)), sleep(3000)])
        if (child.exitCode === null) child.kill()
        lines.close()
      },
    }
  }
  return { call, mutate: (op, args) => call(op, args, { mutate: true }), mcp }
}
module.exports = { managementClient, sleep }
