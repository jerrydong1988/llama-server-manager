const childProcess = require('node:child_process')
const fs = require('node:fs')
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
  // Recent macOS GitHub runners mount the system temporary directory noexec.
  // The seeded engine is executable, so keep that isolated fixture inside the
  // checkout on macOS and use the system temporary directory elsewhere.
  const smokeRoot = process.platform === 'darwin'
    ? path.resolve(__dirname, '..')
    : os.tmpdir()
  const dataDir = fs.mkdtempSync(path.join(smokeRoot, 'lsm-cli-smoke-'))
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

    const currentServicePid = Number.parseInt(
      fs.readFileSync(path.join(dataDir, 'runtime', 'runtime-service.pid'), 'utf8').trim(),
      10,
    )
    if (!Number.isInteger(currentServicePid) || currentServicePid <= 0) {
      throw new Error('runtime service PID file was invalid after the final CLI stop')
    }
    servicePid = currentServicePid
    const idleExitDeadline = Date.now() + 40_000
    while (pidIsAlive(servicePid) && Date.now() < idleExitDeadline) {
      await sleep(50)
    }
    if (pidIsAlive(servicePid)) {
      const serviceLogPath = path.join(dataDir, 'runtime', 'runtime-service.log')
      const serviceLog = fs.existsSync(serviceLogPath)
        ? fs.readFileSync(serviceLogPath, 'utf8')
        : '<missing>'
      throw new Error(
        `idle runtime PID ${servicePid} did not exit after the final CLI stop\n`
        + `runtime service log: ${serviceLog}`,
      )
    }
    progress('idle runtime exited')
    console.log('Headless CLI cross-process lifecycle passed.')
  } finally {
    if (servicePid > 0) terminatePid(servicePid)
    const resolved = fs.realpathSync.native(dataDir)
    const resolvedSmokeRoot = fs.realpathSync.native(smokeRoot)
    if (path.dirname(resolved) !== resolvedSmokeRoot
      || !path.basename(resolved).startsWith('lsm-cli-smoke-')) {
      throw new Error(`refusing to remove unexpected CLI smoke path: ${resolved}`)
    }
    if (process.platform === 'win32') {
      childProcess.execFileSync('icacls', [resolved, '/inheritance:e', '/T', '/C'], {
        stdio: 'ignore',
        windowsHide: true,
      })
      childProcess.execFileSync('icacls', [resolved, '/reset', '/T', '/C'], {
        stdio: 'ignore',
        windowsHide: true,
      })
    }
    fs.rmSync(resolved, { recursive: true, force: true })
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
