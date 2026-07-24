const childProcess = require('node:child_process')
const crypto = require('node:crypto')
const fs = require('node:fs')
const http = require('node:http')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')

const sleep = (milliseconds) => new Promise(resolve => setTimeout(resolve, milliseconds))

function stablePathHash(value) {
  let hash = 0xcbf29ce484222325n
  for (const byte of Buffer.from(value, 'utf8')) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return hash.toString(16).padStart(16, '0')
}

function endpointSuffix(token) {
  return crypto.createHash('sha256').update(token, 'utf8').digest('hex').slice(0, 32)
}

function unixSocketPathFits(endpoint) {
  return Buffer.byteLength(endpoint, 'utf8') <= 90
}

function runtimeEndpoint(dataDir, token) {
  const suffix = endpointSuffix(token)
  if (process.platform === 'win32') {
    return `\\\\.\\pipe\\llama-server-manager-runtime-${suffix}`
  }
  const preferred = path.join(dataDir, 'runtime', `control-${suffix}.sock`)
  const dataHash = stablePathHash(dataDir)
  if (unixSocketPathFits(preferred)) return preferred
  const fallback = path.join(
    os.tmpdir(),
    `llama-server-manager-${dataHash}`,
    `control-${suffix}.sock`,
  )
  return unixSocketPathFits(fallback)
    ? fallback
    : path.join('/tmp', `llama-server-manager-${dataHash}`, `control-${suffix}.sock`)
}

function debugExecutable() {
  const executable = process.platform === 'win32'
    ? 'llama-server-manager.exe'
    : 'llama-server-manager'
  return path.resolve(__dirname, '..', 'src-tauri', 'target', 'debug', executable)
}

async function readToken(dataDir) {
  const tokenPath = path.join(dataDir, 'runtime', 'control-token')
  for (let attempt = 0; attempt < 160; attempt += 1) {
    try {
      return fs.readFileSync(tokenPath, 'utf8').trim()
    } catch {
      await sleep(50)
    }
  }
  throw new Error(`runtime control token was not created at ${tokenPath}`)
}

async function request(endpoint, token, command, requestId) {
  const body = Buffer.from(JSON.stringify({
    protocol_version: 1,
    request_id: requestId,
    token,
    command,
  }))
  const frame = Buffer.allocUnsafe(body.length + 4)
  frame.writeUInt32LE(body.length, 0)
  body.copy(frame, 4)

  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      return await new Promise((resolve, reject) => {
        const socket = net.createConnection(endpoint)
        const chunks = []
        socket.once('connect', () => socket.write(frame))
        socket.on('data', chunk => chunks.push(chunk))
        socket.once('error', reject)
        socket.once('end', () => {
          const response = Buffer.concat(chunks)
          if (response.length < 4) {
            reject(new Error('runtime service returned a truncated response'))
            return
          }
          const length = response.readUInt32LE(0)
          if (response.length < length + 4) {
            reject(new Error('runtime service returned an incomplete response frame'))
            return
          }
          resolve(JSON.parse(response.subarray(4, length + 4).toString('utf8')))
        })
      })
    } catch (error) {
      if (attempt === 79) throw error
      await sleep(50)
    }
  }
  throw new Error('runtime request retry loop exhausted')
}

async function waitForExit(child, timeoutMs) {
  if (child.exitCode !== null) return child.exitCode
  return Promise.race([
    new Promise(resolve => child.once('exit', code => resolve(code))),
    sleep(timeoutMs).then(() => null),
  ])
}

function spawnRuntime(executable, dataDir) {
  const child = childProcess.spawn(
    executable,
    ['--runtime-service', '--runtime-data-dir', dataDir],
    {
      stdio: ['ignore', 'ignore', 'pipe'],
      windowsHide: true,
      env: { ...process.env, LSM_RUNTIME_TEST_LOGIN_REGISTERED: '1' },
    },
  )
  const stderr = []
  child.stderr.on('data', chunk => stderr.push(chunk))
  child.runtimeStderr = () => Buffer.concat(stderr).toString('utf8').trim()
  return child
}

function testLaunchSpec(dataDir, backendPort) {
  const command = process.platform === 'win32'
    ? [process.env.ComSpec || 'C:\\Windows\\System32\\cmd.exe', '/D', '/S', '/C', 'ping -n 120 127.0.0.1 >NUL']
    : ['/bin/sleep', '120']
  return {
    instance_id: 'runtime-smoke-instance',
    config: {
      name: 'Runtime IPC smoke instance',
      alias: 'runtime-smoke-model',
      host: '127.0.0.1',
      port: backendPort,
    },
    engine_backend: 'test',
    command,
    command_display: command.join(' '),
    workload: 'inference',
    working_directory: dataDir,
  }
}

function crashingLaunchSpec(dataDir, backendPort) {
  const command = process.platform === 'win32'
    ? [process.env.ComSpec || 'C:\\Windows\\System32\\cmd.exe', '/D', '/S', '/C', 'ping -n 2 127.0.0.1 >NUL & exit /B 1']
    : ['/bin/sh', '-c', 'sleep 0.2; exit 1']
  return {
    ...testLaunchSpec(dataDir, backendPort),
    command,
    command_display: command.join(' '),
  }
}

async function waitForRuntimeError(endpoint, token, expectedError, requestPrefix) {
  let status
  for (let attempt = 0; attempt < 120; attempt += 1) {
    status = await request(
      endpoint,
      token,
      { command: 'get_status' },
      `${requestPrefix}-${attempt}`,
    )
    if (status.reply?.payload?.last_error === expectedError) return status
    await sleep(50)
  }
  throw new Error(`runtime did not report ${expectedError}: ${JSON.stringify(status)}`)
}

function listen(server, port = 0) {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', () => {
      server.removeListener('error', reject)
      resolve(server.address().port)
    })
  })
}

function closeServer(server) {
  if (!server.listening) return Promise.resolve()
  return new Promise(resolve => server.close(resolve))
}

async function reserveLoopbackPort() {
  const server = net.createServer()
  const port = await listen(server)
  await closeServer(server)
  return port
}

async function httpRequest(port, pathname, { method = 'GET', headers = {}, body = '' } = {}) {
  let lastError
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      return await new Promise((resolve, reject) => {
        const request = http.request({
          host: '127.0.0.1',
          port,
          path: pathname,
          method,
          headers,
        }, response => {
          const chunks = []
          response.on('data', chunk => chunks.push(chunk))
          response.once('end', () => resolve({
            status: response.statusCode,
            body: Buffer.concat(chunks).toString('utf8'),
          }))
        })
        request.setTimeout(2_000, () => request.destroy(new Error('HTTP request timed out')))
        request.once('error', reject)
        if (body) request.write(body)
        request.end()
      })
    } catch (error) {
      lastError = error
      await sleep(50)
    }
  }
  throw lastError || new Error('HTTP request retry loop exhausted')
}

function pidIsAlive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

function terminatePid(pid) {
  if (!pidIsAlive(pid)) return
  if (process.platform === 'win32') {
    childProcess.spawnSync('taskkill', ['/F', '/T', '/PID', String(pid)], {
      stdio: 'ignore',
      windowsHide: true,
    })
    return
  }
  try {
    process.kill(pid, 'SIGKILL')
  } catch {
    // The process may have exited between the liveness check and the signal.
  }
}

async function main() {
  const executable = debugExecutable()
  if (!fs.existsSync(executable)) {
    throw new Error(`debug runtime executable is missing: ${executable}; run cargo build first`)
  }
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-runtime-smoke-'))
  let forwardedRequests = 0
  const backend = http.createServer((request, response) => {
    const chunks = []
    request.on('data', chunk => chunks.push(chunk))
    request.once('end', () => {
      response.setHeader('content-type', 'application/json')
      if (request.url === '/health') {
        response.end('{"status":"ok"}')
      } else if (request.url === '/v1/models') {
        response.end('{"object":"list","data":[{"id":"runtime-smoke-model"}]}')
      } else if (request.url === '/metrics') {
        response.setHeader('content-type', 'text/plain')
        response.end('')
      } else if (request.url === '/slots') {
        response.end('[]')
      } else if (request.url === '/v1/chat/completions') {
        forwardedRequests += 1
        response.end('{"id":"runtime-smoke-response","model":"runtime-smoke-model","choices":[]}')
      } else {
        response.statusCode = 404
        response.end('{"error":"not found"}')
      }
    })
  })
  const backendPort = await listen(backend)
  const proxyPort = await reserveLoopbackPort()
  const proxyBlocker = net.createServer()
  const launchedPids = new Set()
  const contenders = Array.from({ length: 4 }, () => spawnRuntime(executable, dataDir))
  const serviceProcesses = new Set(contenders)
  let service = contenders[0]

  try {
    const token = await readToken(dataDir)
    const endpoint = runtimeEndpoint(dataDir, token)
    const unauthorized = await request(
      endpoint,
      'invalid-runtime-token-value',
      { command: 'ping' },
      'unauthorized',
    )
    if (unauthorized.error !== 'unauthorized') {
      throw new Error(`runtime authentication was not enforced: ${JSON.stringify(unauthorized)}`)
    }

    const status = await request(endpoint, token, { command: 'get_status' }, 'status')
    if (status.reply?.result !== 'status'
      || status.reply.payload?.protocol_version !== 1
      || status.reply.payload?.service_pid <= 0
      || !status.reply.payload?.capabilities?.includes('background_detach_v1')
      || !status.reply.payload?.capabilities?.includes('runtime_error_ack_v1')) {
      throw new Error(`runtime status is invalid: ${JSON.stringify(status)}`)
    }
    service = contenders.find(candidate => candidate.pid === status.reply.payload.service_pid)
    if (!service) {
      throw new Error('neither simultaneous runtime contender owns the control endpoint')
    }
    const duplicates = contenders.filter(candidate => candidate !== service)
    const duplicateExitCodes = await Promise.all(
      duplicates.map(candidate => waitForExit(candidate, 10_000)),
    )
    const failedDuplicateIndex = duplicateExitCodes.findIndex(code => code !== 0)
    if (failedDuplicateIndex !== -1) {
      const duplicate = duplicates[failedDuplicateIndex]
      throw new Error(`duplicate runtime contender exited with code ${duplicateExitCodes[failedDuplicateIndex]}: ${duplicate.runtimeStderr()}`)
    }

    const launchSpec = testLaunchSpec(dataDir, backendPort)
    const proxyConfig = {
      enabled: true,
      host: '127.0.0.1',
      port: proxyPort,
      public_api_key: 'runtime-smoke-proxy-key',
      default_instance_id: launchSpec.instance_id,
      routes: [{
        id: 'runtime-smoke-route',
        enabled: true,
        model_alias: 'runtime-smoke-model',
        target_instance_id: launchSpec.instance_id,
        priority: 0,
      }],
      runtime_service_enabled: true,
    }
    const initialRevision = Date.now()
    const synced = await request(
      endpoint,
      token,
      {
        command: 'sync_config',
        payload: {
          revision: initialRevision,
          proxy_config: proxyConfig,
          instances: { [launchSpec.instance_id]: launchSpec.config },
        },
      },
      'sync-config',
    )
    if (synced.reply?.result !== 'status') {
      throw new Error(`runtime configuration sync failed: ${JSON.stringify(synced)}`)
    }

    const started = await request(
      endpoint,
      token,
      { command: 'start_instance', payload: { spec: launchSpec } },
      'start-instance',
    )
    if (started.reply?.result !== 'instance' || started.reply.payload?.pid <= 0) {
      throw new Error(`runtime instance start failed: ${JSON.stringify(started)}`)
    }
    const firstInstancePid = started.reply.payload.pid
    launchedPids.add(firstInstancePid)

    const proxyStarted = await request(endpoint, token, { command: 'start_proxy' }, 'start-proxy')
    if (proxyStarted.reply?.result !== 'proxy_status' || proxyStarted.reply.payload?.running !== true) {
      throw new Error(`runtime routing start failed: ${JSON.stringify(proxyStarted)}`)
    }
    const unauthorizedProxy = await httpRequest(proxyPort, '/health')
    if (unauthorizedProxy.status !== 401) {
      throw new Error(`runtime routing authentication was not enforced: ${JSON.stringify(unauthorizedProxy)}`)
    }
    const routedBeforeUpgrade = await httpRequest(proxyPort, '/v1/chat/completions', {
      method: 'POST',
      headers: {
        authorization: 'Bearer runtime-smoke-proxy-key',
        'content-type': 'application/json',
      },
      body: JSON.stringify({ model: 'runtime-smoke-model', messages: [] }),
    })
    if (routedBeforeUpgrade.status !== 200 || !routedBeforeUpgrade.body.includes('runtime-smoke-response')) {
      throw new Error(`runtime did not forward the pre-upgrade request: ${JSON.stringify(routedBeforeUpgrade)}`)
    }

    const detached = await request(
      endpoint,
      token,
      {
        command: 'prepare_background_detach',
        payload: {
          revision: initialRevision + 1,
          proxy_config: proxyConfig,
          instances: { [launchSpec.instance_id]: launchSpec.config },
          expected_running: { [launchSpec.instance_id]: started.reply.payload },
        },
      },
      'prepare-background-detach',
    )
    if (detached.reply?.result !== 'status'
      || detached.reply.payload?.background_enabled !== true
      || detached.reply.payload?.registered_for_login !== true
      || detached.reply.payload?.proxy?.running !== true
      || detached.reply.payload?.running?.[launchSpec.instance_id]?.pid !== firstInstancePid) {
      throw new Error(`runtime background handoff verification failed: ${JSON.stringify(detached)}`)
    }

    // No GUI process sends a heartbeat in this test. Surviving a watchdog
    // interval proves that the verified detach flag, not the tray process,
    // owns the runtime lifetime.
    await sleep(21_500)
    const detachedStatus = await request(endpoint, token, { command: 'get_status' }, 'detached-status')
    if (!pidIsAlive(service.pid)
      || detachedStatus.reply?.payload?.running?.[launchSpec.instance_id]?.pid !== firstInstancePid
      || detachedStatus.reply?.payload?.proxy?.running !== true) {
      throw new Error(`runtime did not survive GUI heartbeat expiry: ${JSON.stringify(detachedStatus)}`)
    }
    const routedWhileDetached = await httpRequest(proxyPort, '/v1/chat/completions', {
      method: 'POST',
      headers: {
        authorization: 'Bearer runtime-smoke-proxy-key',
        'content-type': 'application/json',
      },
      body: JSON.stringify({ model: 'runtime-smoke-model', messages: [] }),
    })
    if (routedWhileDetached.status !== 200 || !routedWhileDetached.body.includes('runtime-smoke-response')) {
      throw new Error(`detached runtime stopped forwarding requests: ${JSON.stringify(routedWhileDetached)}`)
    }

    const upgradeShutdown = await request(
      endpoint,
      token,
      { command: 'shutdown', payload: { stop_instances: false } },
      'upgrade-shutdown',
    )
    if (upgradeShutdown.reply?.result !== 'ack') {
      throw new Error(`runtime upgrade shutdown failed: ${JSON.stringify(upgradeShutdown)}`)
    }
    const firstExitCode = await waitForExit(service, 10_000)
    if (firstExitCode !== 0) {
      throw new Error(`first runtime service exited with code ${firstExitCode}`)
    }
    if (pidIsAlive(firstInstancePid)) {
      throw new Error('runtime upgrade left the old supervised child process alive')
    }
    launchedPids.delete(firstInstancePid)

    await listen(proxyBlocker, proxyPort)
    const runtimeStatePath = path.join(dataDir, 'runtime', 'runtime-state.json')
    fs.writeFileSync(runtimeStatePath, '{corrupt-runtime-state', 'utf8')
    service = spawnRuntime(executable, dataDir)
    serviceProcesses.add(service)
    let restored = await request(endpoint, token, { command: 'get_status' }, 'restored-status')
    const restoredInstance = restored.reply?.payload?.running?.['runtime-smoke-instance']
    if (!restoredInstance?.pid || restoredInstance.pid === firstInstancePid) {
      throw new Error(`runtime did not restore the desired instance under fresh supervision: ${JSON.stringify(restored)}`)
    }
    JSON.parse(fs.readFileSync(runtimeStatePath, 'utf8'))
    launchedPids.add(restoredInstance.pid)
    if (restored.reply?.payload?.proxy?.running !== false) {
      throw new Error(`runtime ignored the occupied routing port during recovery: ${JSON.stringify(restored)}`)
    }
    await closeServer(proxyBlocker)
    for (let attempt = 0; attempt < 80; attempt += 1) {
      restored = await request(
        endpoint,
        token,
        { command: 'get_status' },
        `restored-proxy-status-${attempt}`,
      )
      if (restored.reply?.payload?.proxy?.running === true) break
      await sleep(100)
    }
    if (restored.reply?.payload?.proxy?.running !== true) {
      throw new Error(`runtime did not retry routing after the occupied port was released: ${JSON.stringify(restored)}`)
    }
    if (restored.reply?.payload?.last_error) {
      throw new Error(`runtime retained a stale routing recovery error: ${JSON.stringify(restored)}`)
    }
    const routedAfterUpgrade = await httpRequest(proxyPort, '/v1/chat/completions', {
      method: 'POST',
      headers: {
        authorization: 'Bearer runtime-smoke-proxy-key',
        'content-type': 'application/json',
      },
      body: JSON.stringify({ model: 'runtime-smoke-model', messages: [] }),
    })
    if (routedAfterUpgrade.status !== 200 || !routedAfterUpgrade.body.includes('runtime-smoke-response')) {
      throw new Error(`runtime did not forward the post-upgrade request: ${JSON.stringify(routedAfterUpgrade)}`)
    }
    if (forwardedRequests !== 3) {
      throw new Error(`runtime routed an unexpected number of requests: ${forwardedRequests}`)
    }
    const stopped = await request(
      endpoint,
      token,
      { command: 'stop_instance', payload: { instance_id: 'runtime-smoke-instance' } },
      'stop-instance',
    )
    if (stopped.reply?.result !== 'ack') {
      throw new Error(`runtime instance stop failed: ${JSON.stringify(stopped)}`)
    }
    launchedPids.delete(restoredInstance.pid)

    const expectedExitError = `instance ${launchSpec.instance_id} exited unexpectedly (code 1)`
    const firstCrash = await request(
      endpoint,
      token,
      { command: 'start_instance', payload: { spec: crashingLaunchSpec(dataDir, backendPort) } },
      'start-first-crash',
    )
    if (firstCrash.reply?.result !== 'instance' || firstCrash.reply.payload?.pid <= 0) {
      throw new Error(`runtime crash fixture did not start: ${JSON.stringify(firstCrash)}`)
    }
    launchedPids.add(firstCrash.reply.payload.pid)
    await waitForRuntimeError(endpoint, token, expectedExitError, 'first-crash-status')
    launchedPids.delete(firstCrash.reply.payload.pid)

    const restarted = await request(
      endpoint,
      token,
      { command: 'start_instance', payload: { spec: launchSpec } },
      'restart-after-crash',
    )
    if (restarted.reply?.result !== 'instance' || restarted.reply.payload?.pid <= 0) {
      throw new Error(`runtime instance retry failed: ${JSON.stringify(restarted)}`)
    }
    launchedPids.add(restarted.reply.payload.pid)
    const restartedStatus = await request(
      endpoint,
      token,
      { command: 'get_status' },
      'status-after-successful-retry',
    )
    if (restartedStatus.reply?.payload?.last_error) {
      throw new Error(`runtime retained a stale instance exit error after retry: ${JSON.stringify(restartedStatus)}`)
    }
    const restopped = await request(
      endpoint,
      token,
      { command: 'stop_instance', payload: { instance_id: launchSpec.instance_id } },
      'stop-retried-instance',
    )
    if (restopped.reply?.result !== 'ack') {
      throw new Error(`retried runtime instance stop failed: ${JSON.stringify(restopped)}`)
    }
    launchedPids.delete(restarted.reply.payload.pid)

    const secondCrash = await request(
      endpoint,
      token,
      { command: 'start_instance', payload: { spec: crashingLaunchSpec(dataDir, backendPort) } },
      'start-second-crash',
    )
    if (secondCrash.reply?.result !== 'instance' || secondCrash.reply.payload?.pid <= 0) {
      throw new Error(`second runtime crash fixture did not start: ${JSON.stringify(secondCrash)}`)
    }
    launchedPids.add(secondCrash.reply.payload.pid)
    await waitForRuntimeError(endpoint, token, expectedExitError, 'second-crash-status')
    launchedPids.delete(secondCrash.reply.payload.pid)

    const cleared = await request(
      endpoint,
      token,
      { command: 'clear_last_error' },
      'clear-last-error',
    )
    if (cleared.reply?.result !== 'ack') {
      throw new Error(`runtime error acknowledgement failed: ${JSON.stringify(cleared)}`)
    }
    const clearedStatus = await request(
      endpoint,
      token,
      { command: 'get_status' },
      'status-after-error-clear',
    )
    if (clearedStatus.reply?.payload?.last_error) {
      throw new Error(`runtime retained an acknowledged error: ${JSON.stringify(clearedStatus)}`)
    }

    const enabled = await request(
      endpoint,
      token,
      { command: 'set_background_enabled', payload: { enabled: true } },
      'enable',
    )
    if (enabled.reply?.payload?.background_enabled !== true) {
      throw new Error('runtime did not persist background enablement')
    }
    const disabled = await request(
      endpoint,
      token,
      { command: 'set_background_enabled', payload: { enabled: false } },
      'disable',
    )
    if (disabled.reply?.payload?.background_enabled !== false) {
      throw new Error('runtime did not persist background disablement')
    }

    const shutdown = await request(
      endpoint,
      token,
      { command: 'shutdown', payload: { stop_instances: true } },
      'shutdown',
    )
    if (shutdown.reply?.result !== 'ack') {
      throw new Error(`runtime shutdown failed: ${JSON.stringify(shutdown)}`)
    }
    const exitCode = await waitForExit(service, 8_000)
    if (exitCode === null) throw new Error('runtime service did not exit after shutdown')
    if (exitCode !== 0) throw new Error(`runtime service exited with code ${exitCode}`)
    console.log(`Runtime service IPC smoke test passed (PID ${status.reply.payload.service_pid}).`)
  } finally {
    for (const process of serviceProcesses) {
      if (process.exitCode === null) {
        process.kill()
        await waitForExit(process, 2_000)
      }
    }
    for (const pid of launchedPids) terminatePid(pid)
    await closeServer(proxyBlocker)
    await closeServer(backend)
    fs.rmSync(dataDir, { recursive: true, force: true })
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
