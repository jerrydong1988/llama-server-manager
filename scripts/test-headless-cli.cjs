const childProcess = require('node:child_process')
const crypto = require('node:crypto')
const fs = require('node:fs')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')

const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
const progress = step => console.log(`[headless-cli] ${step}`)

function executablePath() {
  return path.resolve(
    __dirname,
    '..',
    'src-tauri',
    'target',
    'debug',
    process.platform === 'win32' ? 'lsm.exe' : 'lsm',
  )
}

function invoke(executable, dataDir, arguments, expectedExit = 0) {
  const result = childProcess.spawnSync(
    executable,
    ['--output', 'json', '--data-dir', dataDir, ...arguments],
    { encoding: 'utf8', windowsHide: true },
  )
  if (result.status !== expectedExit) {
    throw new Error(
      `lsm ${arguments.join(' ')} exited ${result.status}, expected ${expectedExit}`
      + `\nspawn error: ${result.error?.message ?? '<none>'}`
      + `\n${result.stdout ?? ''}\n${result.stderr ?? ''}`,
    )
  }
  const output = result.stdout.trim()
  const payload = JSON.parse(output)
  if (payload.schemaVersion !== 1) throw new Error('CLI schema version drifted')
  if (
    output.includes('fixture-api-secret')
    || output.includes('control-token')
    || /"(?:apiKey|api_key|token)"\s*:/i.test(output)
  ) {
    throw new Error(`CLI output exposed a credential-shaped field: ${output}`)
  }
  return payload
}

function stablePathHash(value) {
  let hash = 0xcbf29ce484222325n
  for (const byte of Buffer.from(value, 'utf8')) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return hash.toString(16).padStart(16, '0')
}

function runtimeEndpoint(dataDir, token) {
  const suffix = crypto.createHash('sha256').update(token, 'utf8').digest('hex').slice(0, 32)
  if (process.platform === 'win32') {
    return `\\\\.\\pipe\\llama-server-manager-runtime-${suffix}`
  }
  const preferred = path.join(dataDir, 'runtime', `control-${suffix}.sock`)
  if (Buffer.byteLength(preferred, 'utf8') <= 90) return preferred
  const fallback = path.join(
    os.tmpdir(),
    `llama-server-manager-${stablePathHash(dataDir)}`,
    `control-${suffix}.sock`,
  )
  return Buffer.byteLength(fallback, 'utf8') <= 90
    ? fallback
    : path.join('/tmp', `llama-server-manager-${stablePathHash(dataDir)}`, `control-${suffix}.sock`)
}

async function runtimeRequest(endpoint, token, command, requestId) {
  const body = Buffer.from(JSON.stringify({
    protocol_version: 1,
    request_id: requestId,
    token,
    command,
  }))
  const frame = Buffer.allocUnsafe(body.length + 4)
  frame.writeUInt32LE(body.length, 0)
  body.copy(frame, 4)
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(endpoint)
    const chunks = []
    socket.setTimeout(5_000, () => socket.destroy(new Error('runtime request timed out')))
    socket.once('connect', () => socket.write(frame))
    socket.on('data', chunk => chunks.push(chunk))
    socket.once('error', reject)
    socket.once('end', () => {
      const response = Buffer.concat(chunks)
      if (response.length < 4) return reject(new Error('runtime response was truncated'))
      const length = response.readUInt32LE(0)
      if (response.length < length + 4) return reject(new Error('runtime response was incomplete'))
      resolve(JSON.parse(response.subarray(4, length + 4).toString('utf8')))
    })
  })
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
  } else {
    try {
      process.kill(pid, 'SIGKILL')
    } catch {
      // The process may exit between the liveness check and signal.
    }
  }
}

async function main() {
  const executable = executablePath()
  if (!fs.existsSync(executable) || fs.statSync(executable).size === 0) {
    throw new Error(`headless CLI binary is missing: ${executable}; run cargo build first`)
  }
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'lsm-cli-smoke-'))
  let servicePid = 0
  try {
    const help = invoke(executable, dataDir, ['help'])
    const version = invoke(executable, dataDir, ['version'])
    if (!help.data.help.includes('instance start') || version.data.schemaVersion !== 1) {
      throw new Error('help or version contract failed')
    }
    if (fs.existsSync(path.join(dataDir, 'runtime', 'control-token'))) {
      throw new Error('help/version unexpectedly started the local runtime')
    }
    progress('offline help and version verified')

    const seeded = invoke(executable, dataDir, ['__test-seed-fixture'])
    progress('fixture seeded')
    if (seeded.data.instanceId !== 'cli-fixture') throw new Error('fixture seed failed')

    const status = invoke(executable, dataDir, ['status'])
    progress('runtime available')
    servicePid = status.data.service.pid
    if (!Number.isInteger(servicePid) || servicePid <= 0) {
      throw new Error('status did not report a live runtime PID')
    }
    const started = invoke(executable, dataDir, ['instance', 'start', 'cli-fixture'])
    progress('instance started')
    if (started.data.state !== 'running' || started.data.pid <= 0) {
      throw new Error(`instance start contract failed: ${JSON.stringify(started)}`)
    }
    const listed = invoke(executable, dataDir, ['instance', 'list'])
    progress('instance listed')
    const ids = listed.data.instances.map(instance => instance.id)
    if (JSON.stringify(ids) !== JSON.stringify([...ids].sort())) {
      throw new Error('instance list ordering is not deterministic')
    }
    if (!listed.data.instances.some(instance => (
      instance.id === 'cli-fixture' && instance.state === 'running'
    ))) {
      const logPath = path.join(dataDir, 'configs', 'logs', 'cli-fixture.log')
      const log = fs.existsSync(logPath) ? fs.readFileSync(logPath, 'utf8') : '<missing>'
      const service = invoke(executable, dataDir, ['status'])
      throw new Error(
        `instance list did not expose the running fixture: ${JSON.stringify(listed)}\n`
        + `runtime status: ${JSON.stringify(service)}\nfixture log: ${log}`,
      )
    }
    invoke(executable, dataDir, ['instance', 'status', 'secret-fixture'])
    progress('configured API key redaction verified')
    invoke(executable, dataDir, ['instance', 'status', 'missing'], 3)
    invoke(executable, dataDir, ['unknown'], 2)
    progress('error contracts verified')

    const proxyStarted = invoke(executable, dataDir, ['proxy', 'start'])
    progress('proxy started')
    if (!proxyStarted.data.running) throw new Error('proxy start did not persist runtime state')
    const proxyStatus = invoke(executable, dataDir, ['proxy', 'status'])
    if (!proxyStatus.data.running) throw new Error('proxy status lost the running state')
    const proxyStopped = invoke(executable, dataDir, ['proxy', 'stop'])
    progress('proxy stopped')
    if (proxyStopped.data.running) throw new Error('proxy stop did not persist runtime state')

    const stopped = invoke(executable, dataDir, ['instance', 'stop', 'cli-fixture'])
    if (stopped.data.state !== 'stopped') throw new Error('instance stop contract failed')
    const stoppedAgain = invoke(executable, dataDir, ['instance', 'stop', 'cli-fixture'])
    progress('instance stop is idempotent')
    if (stoppedAgain.data.state !== 'stopped') throw new Error('instance stop is not idempotent')

    const token = fs.readFileSync(path.join(dataDir, 'runtime', 'control-token'), 'utf8').trim()
    const response = await runtimeRequest(
      runtimeEndpoint(dataDir, token),
      token,
      { command: 'shutdown', payload: { stop_instances: true } },
      'cli-smoke-shutdown',
    )
    if (response.error || response.reply?.result !== 'ack') {
      throw new Error(`runtime shutdown failed: ${JSON.stringify(response)}`)
    }
    progress('runtime shutdown acknowledged')
    for (let attempt = 0; attempt < 80 && pidIsAlive(servicePid); attempt += 1) {
      await sleep(50)
    }
    if (pidIsAlive(servicePid)) throw new Error('runtime did not exit after CLI smoke test')
    console.log('Headless CLI cross-process lifecycle passed.')
  } finally {
    if (servicePid > 0) terminatePid(servicePid)
    const resolved = fs.realpathSync.native(dataDir)
    const tempRoot = fs.realpathSync.native(os.tmpdir())
    if (!resolved.startsWith(`${tempRoot}${path.sep}`)) {
      throw new Error(`refusing to remove unexpected CLI smoke path: ${resolved}`)
    }
    fs.rmSync(resolved, { recursive: true, force: true })
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
