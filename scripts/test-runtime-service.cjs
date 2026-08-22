const childProcess = require('node:child_process')
const crypto = require('node:crypto')
const esbuild = require('esbuild')
const fs = require('node:fs')
const http = require('node:http')
const Module = require('node:module')
const net = require('node:net')
const os = require('node:os')
const path = require('node:path')

const sleep = (milliseconds) => new Promise(resolve => setTimeout(resolve, milliseconds))
let cachedWindowsUserSid

function loadDefaultInstanceConfig() {
  const sourcePath = path.resolve(__dirname, '..', 'src', 'store', 'defaults.ts')
  const compiled = esbuild.transformSync(fs.readFileSync(sourcePath, 'utf8'), {
    format: 'cjs',
    loader: 'ts',
    target: 'node20',
  }).code
  const loaded = new Module(sourcePath, module)
  loaded.filename = sourcePath
  loaded.paths = Module._nodeModulePaths(path.dirname(sourcePath))
  loaded._compile(compiled, sourcePath)
  return loaded.exports.defaultInstanceConfig
}

const defaultInstanceConfig = loadDefaultInstanceConfig()

function loadRustConfigShape() {
  const source = fs.readFileSync(
    path.resolve(__dirname, '..', 'src-tauri', 'src', 'models.rs'),
    'utf8',
  )
  const start = source.indexOf('pub struct InstanceConfig')
  const end = source.indexOf('impl Default for InstanceConfig', start)
  if (start === -1 || end === -1) throw new Error('InstanceConfig source definition was not found')
  const definition = source.slice(start, end)
  const fieldOrder = [...definition.matchAll(/pub\s+([a-zA-Z0-9_]+)\s*:/g)]
    .map(match => match[1])
  const floatFields = new Set(
    [...definition.matchAll(/pub\s+([a-zA-Z0-9_]+)\s*:\s*f(?:32|64)/g)]
      .map(match => match[1]),
  )
  return { fieldOrder, floatFields }
}

const RUST_CONFIG_SHAPE = loadRustConfigShape()
function stablePathHash(value) {
  let hash = 0xcbf29ce484222325n
  return updateFingerprintHash(hash, Buffer.from(value, 'utf8')).toString(16).padStart(16, '0')
}

function updateFingerprintHash(hash, bytes) {
  for (const byte of bytes) {
    hash ^= BigInt(byte)
    hash = BigInt.asUintN(64, hash * 0x100000001b3n)
  }
  return hash
}

function artifactIdentity(kind, artifactPath) {
  const fileSize = fs.statSync(artifactPath, { bigint: true }).size
  const digest = crypto.createHash('sha256')
  digest.update(Buffer.from('llama-server-manager:full-artifact:v1\0'))
  digest.update(Buffer.from(kind))
  const sizeBytes = Buffer.alloc(8)
  sizeBytes.writeBigUInt64LE(fileSize)
  digest.update(sizeBytes)
  digest.update(fs.readFileSync(artifactPath))
  return `urn:lsm:${kind}:v1:sha256:${digest.digest('hex')}`
}

function engineBundleIdentity(executable) {
  const canonical = fs.realpathSync.native(executable)
  const primarySize = fs.statSync(canonical, { bigint: true }).size
  const primaryArtifactId = artifactIdentity('engine', canonical)
  if (process.platform !== 'win32') {
    return { artifactId: primaryArtifactId, fileSize: primarySize }
  }

  const root = path.dirname(canonical)
  const primaryName = path.basename(canonical).toLowerCase()
  const candidates = []
  const directories = [{ directory: root, depth: 0 }]
  while (directories.length) {
    const { directory, depth } = directories.pop()
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const memberPath = path.join(directory, entry.name)
      if (entry.isSymbolicLink()) {
        throw new Error(`engine bundle cannot contain links or reparse points: ${memberPath}`)
      }
      if (entry.isDirectory()) {
        if (depth >= 8) throw new Error('engine bundle exceeds 8 directory levels')
        directories.push({ directory: memberPath, depth: depth + 1 })
        continue
      }
      if (!entry.isFile()
        || path.extname(entry.name).toLowerCase() !== '.dll'
        || entry.name.toLowerCase() === primaryName) continue
      candidates.push({
        relativePath: path.relative(root, memberPath).replaceAll('\\', '/').toLowerCase(),
        memberPath,
      })
      if (candidates.length > 512) throw new Error('engine bundle exceeds 512 DLL members')
    }
  }
  candidates.sort((left, right) => Buffer.compare(
    Buffer.from(left.relativePath, 'utf8'),
    Buffer.from(right.relativePath, 'utf8'),
  ))
  if (candidates.some((candidate, index) => (
    index > 0 && candidate.relativePath === candidates[index - 1].relativePath
  ))) {
    throw new Error('engine bundle contains duplicate case-insensitive DLL paths')
  }

  const digest = crypto.createHash('sha256')
  digest.update(Buffer.from('llama-server-manager:engine-bundle:v1\0'))
  digest.update(Buffer.from(primaryArtifactId, 'utf8'))
  let fileSize = primarySize
  for (const candidate of candidates) {
    const memberSize = fs.statSync(candidate.memberPath, { bigint: true }).size
    fileSize += memberSize
    const relativeLength = Buffer.alloc(8)
    relativeLength.writeBigUInt64LE(BigInt(Buffer.byteLength(candidate.relativePath, 'utf8')))
    digest.update(relativeLength)
    digest.update(Buffer.from(candidate.relativePath, 'utf8'))
    const memberSizeBytes = Buffer.alloc(8)
    memberSizeBytes.writeBigUInt64LE(memberSize)
    digest.update(memberSizeBytes)
    digest.update(Buffer.from(artifactIdentity('engine', candidate.memberPath), 'utf8'))
  }
  return {
    artifactId: `urn:lsm:engine:v1:sha256:${digest.digest('hex')}`,
    fileSize,
  }
}

function rustConfigJson(config) {
  const canonical = { ...config, id: '', name: '' }
  const keys = Object.keys(canonical)
  const missing = RUST_CONFIG_SHAPE.fieldOrder.filter(key => !Object.hasOwn(canonical, key))
  const extra = keys.filter(key => !RUST_CONFIG_SHAPE.fieldOrder.includes(key))
  if (missing.length || extra.length) {
    throw new Error(`runtime smoke config shape drifted (missing: ${missing.join(', ') || 'none'}; extra: ${extra.join(', ') || 'none'})`)
  }
  const entries = RUST_CONFIG_SHAPE.fieldOrder.map((key) => {
    const value = canonical[key]
    let encoded = JSON.stringify(value)
    if (RUST_CONFIG_SHAPE.floatFields.has(key) && Number.isInteger(value)) encoded = `${value}.0`
    return `${JSON.stringify(key)}:${encoded}`
  })
  return `{${entries.join(',')}}`
}

function deploymentIdentity(spec) {
  const engineArtifactId = engineBundleIdentity(spec.command[0]).artifactId
  const modelArtifactId = artifactIdentity('model', spec.config.model_path)
  const configurationHash = crypto
    .createHash('sha256')
    .update(rustConfigJson(spec.config), 'utf8')
    .digest('hex')
  const configRevisionId = 'runtime-smoke-revision-v1'
  const configurationId = `urn:lsm:configuration:v1:sha256:${configurationHash}`
  const qualificationEvidenceId = 'urn:lsm:qualification:v2:sha256:runtime-smoke'
  const material = {
    schemaVersion: 1,
    engineArtifactId,
    modelArtifactId,
    configRevisionId,
    configurationId,
    qualificationEvidenceId,
  }
  const deploymentId = `urn:lsm:deployment:v1:sha256:${crypto
    .createHash('sha256')
    .update(JSON.stringify(material), 'utf8')
    .digest('hex')}`
  return { ...material, auxiliaryArtifacts: [], deploymentId }
}

function deploymentRevision(spec, identity, proxyConfig = {}) {
  const deploymentMaterial = { schemaVersion: 1, instanceId: spec.instance_id }
  const deploymentId = `urn:lsm:managed-deployment:v1:sha256:${crypto
    .createHash('sha256')
    .update(JSON.stringify(deploymentMaterial), 'utf8')
    .digest('hex')}`
  const deploymentIdentityMaterial = {
    schemaVersion: identity.schemaVersion,
    deploymentId: identity.deploymentId,
    engineArtifactId: identity.engineArtifactId,
    modelArtifactId: identity.modelArtifactId,
    auxiliaryArtifacts: identity.auxiliaryArtifacts,
    configRevisionId: identity.configRevisionId,
    configurationId: identity.configurationId,
    qualificationEvidenceId: identity.qualificationEvidenceId,
  }
  const runtimePolicy = {
    autoStart: Boolean(spec.config.auto_start),
    restartPolicy: spec.config.restart_policy?.toLowerCase() === 'on-failure'
      ? 'on-failure'
      : 'never',
  }
  const routing = {
    proxyEnabled: Boolean(proxyConfig.enabled),
    defaultTarget: proxyConfig.default_instance_id === spec.instance_id,
    routingStrategy: String(proxyConfig.routing_strategy ?? 'priorityFailover').trim(),
    routes: (proxyConfig.routes ?? [])
      .filter(route => route.target_instance_id === spec.instance_id)
      .map(route => ({
        id: String(route.id ?? '').trim(),
        enabled: route.enabled ?? true,
        modelAlias: String(route.model_alias ?? '').trim(),
        priority: route.priority ?? 0,
        weight: route.weight ?? 1,
        maxConcurrentRequests: route.max_concurrent_requests ?? 0,
      }))
      .sort((left, right) => (
        left.id.localeCompare(right.id)
        || Number(left.enabled) - Number(right.enabled)
        || left.modelAlias.localeCompare(right.modelAlias)
        || left.priority - right.priority
        || left.weight - right.weight
        || left.maxConcurrentRequests - right.maxConcurrentRequests
      )),
  }
  const material = {
    schemaVersion: 1,
    deploymentId,
    deploymentIdentity: deploymentIdentityMaterial,
    runtimePolicy,
    routing,
  }
  const id = `urn:lsm:deployment-revision:v1:sha256:${crypto
    .createHash('sha256')
    .update(JSON.stringify(material), 'utf8')
    .digest('hex')}`
  const createdAt = 1
  const integrity = `sha256:${crypto
    .createHash('sha256')
    .update(JSON.stringify({ id, createdAt, material }), 'utf8')
    .digest('hex')}`
  return { ...material, id, createdAt, integrity }
}

function withLaunchIdentity(spec, proxyConfig) {
  const identity = deploymentIdentity(spec)
  return {
    ...spec,
    ...engineQualificationBinding(spec.command),
    deployment_identity: identity,
    deployment_revision: deploymentRevision(spec, identity, proxyConfig),
  }
}

function fingerprintPathIdentity(value) {
  if (process.platform !== 'win32') return value.trim()

  let normalized = value.trim().replaceAll('\\', '/')
  const lower = normalized.toLowerCase()
  if (lower.startsWith('//?/unc/')) {
    normalized = `//${normalized.slice(8)}`
  } else if (lower.startsWith('//?/')) {
    normalized = normalized.slice(4)
  }

  const isUnc = normalized.startsWith('//')
  const isDriveRooted = normalized.length >= 3
    && normalized[1] === ':' && normalized[2] === '/'
  const prefix = isUnc ? '//' : isDriveRooted ? normalized.slice(0, 3) : ''
  const body = isUnc ? normalized.slice(2) : isDriveRooted ? normalized.slice(3) : normalized
  const protectedSegments = isUnc ? 2 : 0
  const segments = []
  for (const segment of body.split('/')) {
    if (!segment || segment === '.') continue
    if (segment === '..') {
      if (segments.length > protectedSegments && segments.at(-1) !== '..') {
        segments.pop()
      } else if (!prefix) {
        segments.push(segment)
      }
      continue
    }
    segments.push(segment)
  }
  const joined = segments.join('/')
  return (joined ? `${prefix}${joined}` : prefix || '.').toLowerCase()
}

function executableFingerprint(executable) {
  const canonical = fs.realpathSync.native(executable)
  const metadata = fs.statSync(canonical, { bigint: true })
  if (!metadata.isFile()) throw new Error(`qualification executable is not a file: ${canonical}`)
  const identity = engineBundleIdentity(canonical)

  const normalizedPath = fingerprintPathIdentity(canonical)
  const digest = crypto.createHash('sha256')
  digest.update(Buffer.from('llama-server-manager:engine-fingerprint:v3\0'))
  digest.update(Buffer.from(normalizedPath, 'utf8'))
  const sizeBytes = Buffer.alloc(8)
  sizeBytes.writeBigUInt64LE(identity.fileSize)
  digest.update(sizeBytes)
  const modifiedBytes = Buffer.alloc(16)
  modifiedBytes.writeBigUInt64LE(BigInt.asUintN(64, metadata.mtimeNs), 0)
  modifiedBytes.writeBigUInt64LE(metadata.mtimeNs >> 64n, 8)
  digest.update(modifiedBytes)
  digest.update(Buffer.from(identity.artifactId, 'utf8'))

  return `v3:${normalizedPath}:${identity.fileSize}:${metadata.mtimeNs}:${digest.digest('hex')}`
}

function engineQualificationBinding(command) {
  return {
    engine_qualification_fingerprint: executableFingerprint(command[0]),
    engine_qualification_profile_version: 1,
  }
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
    `lsm-${dataHash}-${suffix}`,
    `control-${suffix}.sock`,
  )
  return unixSocketPathFits(fallback)
    ? fallback
    : path.join('/tmp', `lsm-${suffix}`, `control-${suffix}.sock`)
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

function runtimeFrame(value) {
  const body = Buffer.from(JSON.stringify(value))
  const frame = Buffer.allocUnsafe(body.length + 4)
  frame.writeUInt32LE(body.length, 0)
  body.copy(frame, 4)
  return frame
}

function expectedServicePid(dataDir) {
  const value = Number.parseInt(
    fs.readFileSync(path.join(dataDir, 'runtime', 'runtime-service.pid'), 'utf8').trim(),
    10,
  )
  if (!Number.isInteger(value) || value <= 0 || value > 0xffffffff) {
    throw new Error('runtime service identity is invalid')
  }
  return value
}

function runtimeHandshakeProof(controlToken, nonce, servicePid) {
  const servicePidBytes = Buffer.alloc(4)
  servicePidBytes.writeUInt32LE(servicePid)
  return crypto
    .createHash('sha256')
    .update(Buffer.from('llama-server-manager:runtime-handshake:v1\0'))
    .update(Buffer.from(controlToken, 'utf8'))
    .update(Buffer.from([0]))
    .update(Buffer.from(nonce, 'utf8'))
    .update(Buffer.from([0]))
    .update(servicePidBytes)
    .digest('hex')
}

async function request(endpoint, token, command, requestId) {
  const requestFrame = runtimeFrame({
    protocol_version: 1,
    request_id: requestId,
    token,
    command,
  })

  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const servicePid = expectedServicePid(endpoint.dataDir)
      const nonce = crypto.randomUUID()
      const handshakeFrame = runtimeFrame({ protocol_version: 1, nonce })
      return await new Promise((resolve, reject) => {
        const socket = net.createConnection(endpoint.address)
        let buffered = Buffer.alloc(0)
        let phase = 'handshake'
        let settled = false
        const fail = (error) => {
          if (settled) return
          settled = true
          socket.destroy()
          reject(error)
        }
        socket.setTimeout(10_000, () => fail(new Error('runtime service request timed out')))
        socket.once('connect', () => socket.write(handshakeFrame))
        socket.on('data', (chunk) => {
          buffered = Buffer.concat([buffered, chunk])
          while (!settled && buffered.length >= 4) {
            const length = buffered.readUInt32LE(0)
            if (length > 8 * 1024 * 1024) {
              fail(new Error(`runtime service returned an oversized frame: ${length}`))
              return
            }
            if (buffered.length < length + 4) return
            const payload = buffered.subarray(4, length + 4)
            buffered = buffered.subarray(length + 4)
            let response
            try {
              response = JSON.parse(payload.toString('utf8'))
            } catch (error) {
              fail(error)
              return
            }
            if (phase === 'handshake') {
              const expectedProof = runtimeHandshakeProof(endpoint.controlToken, nonce, servicePid)
              const actualProof = Buffer.from(String(response.proof ?? ''), 'utf8')
              const expectedProofBytes = Buffer.from(expectedProof, 'utf8')
              if (response.protocol_version !== 1
                || response.nonce !== nonce
                || response.service_pid !== servicePid
                || actualProof.length !== expectedProofBytes.length
                || !crypto.timingSafeEqual(actualProof, expectedProofBytes)) {
                fail(new Error('runtime server authentication failed before sending credentials'))
                return
              }
              phase = 'response'
              socket.write(requestFrame)
              continue
            }
            settled = true
            socket.end()
            resolve(response)
          }
        })
        socket.once('error', fail)
        socket.once('end', () => {
          if (!settled) fail(new Error('runtime service returned a truncated response'))
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

function testLaunchSpec(dataDir, backendPort, proxyConfig) {
  const modelPath = path.join(dataDir, 'models', 'runtime-smoke-model.gguf')
  fs.mkdirSync(path.dirname(modelPath), { recursive: true })
  fs.writeFileSync(modelPath, 'runtime-smoke-model')
  protectWindowsFixtureTree(path.dirname(modelPath))
  const apiKeyFile = fixtureApiKeyFile(dataDir)
  const command = process.platform === 'win32'
    ? [fixtureCommand(dataDir), '/D', '/S', '/C', 'ping -n 120 127.0.0.1 >NUL & rem', '--model', modelPath, '--api-key-file', apiKeyFile]
    : [fixtureCommand(dataDir), '-c', 'sleep 120', 'runtime-smoke', '--model', modelPath, '--api-key-file', apiKeyFile]
  return withLaunchIdentity({
    instance_id: 'runtime-smoke-instance',
    config: {
      ...defaultInstanceConfig(),
      id: 'runtime-smoke-instance',
      name: 'Runtime IPC smoke instance',
      alias: 'runtime-smoke-model',
      model_path: modelPath,
      host: '127.0.0.1',
      port: backendPort,
      api_key: '',
      api_key_file: apiKeyFile,
    },
    engine_backend: 'test',
    command,
    command_display: command.join(' '),
    workload: 'inference',
    working_directory: dataDir,
  }, proxyConfig)
}

function crashingLaunchSpec(dataDir, backendPort, proxyConfig) {
  const spec = testLaunchSpec(dataDir, backendPort, proxyConfig)
  const command = process.platform === 'win32'
    ? [fixtureCommand(dataDir), '/D', '/S', '/C', 'ping -n 2 127.0.0.1 >NUL & exit /B 1 & rem', '--model', spec.config.model_path, '--api-key-file', spec.config.api_key_file]
    : [fixtureCommand(dataDir), '-c', 'sleep 0.2; exit 1', '--model', spec.config.model_path, '--api-key-file', spec.config.api_key_file]
  return withLaunchIdentity({
    ...spec,
    command,
    command_display: command.join(' '),
  }, proxyConfig)
}

function recoverOnceLaunchSpec(dataDir, backendPort, proxyConfig) {
  const marker = 'runtime-recovery-once.marker'
  const spec = testLaunchSpec(dataDir, backendPort, proxyConfig)
  const command = process.platform === 'win32'
    ? [
        fixtureCommand(dataDir),
        '/D',
        '/S',
        '/C',
        `if exist ${marker} (ping -n 120 127.0.0.1 >NUL) else (type NUL > ${marker} & ping -n 2 127.0.0.1 >NUL & exit /B 1) & rem`,
        '--model',
        spec.config.model_path,
        '--api-key-file',
        spec.config.api_key_file,
      ]
    : [fixtureCommand(dataDir), '-c', `if [ -f ${marker} ]; then sleep 120; else : > ${marker}; sleep 0.2; exit 1; fi`, '--model', spec.config.model_path, '--api-key-file', spec.config.api_key_file]
  return withLaunchIdentity({
    ...spec,
    config: { ...spec.config, restart_policy: 'on-failure' },
    command,
    command_display: command.join(' '),
  }, proxyConfig)
}

function fixtureCommand(dataDir) {
  const directory = path.join(dataDir, 'engines', 'runtime-smoke')
  const destination = path.join(directory, process.platform === 'win32' ? 'cmd.exe' : 'sh')
  if (!fs.existsSync(destination)) {
    fs.mkdirSync(directory, { recursive: true })
    const source = process.platform === 'win32'
      ? process.env.ComSpec || 'C:\\Windows\\System32\\cmd.exe'
      : '/bin/sh'
    fs.copyFileSync(source, destination)
    if (process.platform !== 'win32') fs.chmodSync(destination, 0o700)
  }
  protectWindowsFixtureTree(path.join(dataDir, 'engines'))
  return destination
}

function fixtureApiKeyFile(dataDir) {
  const directory = path.join(dataDir, 'credentials')
  const destination = path.join(directory, 'runtime-smoke.api-key')
  fs.mkdirSync(directory, { recursive: true })
  fs.writeFileSync(
    destination,
    'lsm_runtime_smoke_instance_0123456789abcdefghijklmnopqrstuvwxyz\n',
    { encoding: 'utf8', mode: 0o600 },
  )
  if (process.platform !== 'win32') fs.chmodSync(destination, 0o600)
  protectWindowsFixtureTree(directory)
  return destination
}

function protectWindowsFixtureTree(directory) {
  if (process.platform !== 'win32') return
  if (!cachedWindowsUserSid) {
    const identity = childProcess.execFileSync('whoami', ['/user', '/fo', 'csv', '/nh'], {
      encoding: 'utf8',
      windowsHide: true,
    })
    cachedWindowsUserSid = identity.match(/"(S-[^"]+)"/)?.[1]
    if (!cachedWindowsUserSid) throw new Error('failed to resolve the Windows test user SID')
  }
  const directories = [directory]
  const files = []
  for (let index = 0; index < directories.length; index += 1) {
    for (const entry of fs.readdirSync(directories[index], { withFileTypes: true })) {
      const entryPath = path.join(directories[index], entry.name)
      if (entry.isDirectory()) directories.push(entryPath)
      else if (entry.isFile()) files.push(entryPath)
    }
  }
  for (const target of directories) {
    childProcess.execFileSync('icacls', [
      target,
      '/inheritance:r',
      '/grant:r',
      `*${cachedWindowsUserSid}:(OI)(CI)(F)`,
      '*S-1-5-18:(OI)(CI)(F)',
    ], { stdio: 'ignore', windowsHide: true })
  }
  for (const target of files) {
    childProcess.execFileSync('icacls', [
      target,
      '/inheritance:r',
      '/grant:r',
      `*${cachedWindowsUserSid}:(F)`,
      '*S-1-5-18:(F)',
    ], { stdio: 'ignore', windowsHide: true })
  }
}

async function waitForAutomaticRecovery(endpoint, token, originalPid) {
  let status
  for (let attempt = 0; attempt < 200; attempt += 1) {
    status = await request(
      endpoint,
      token,
      { command: 'get_status' },
      `automatic-recovery-status-${attempt}`,
    )
    const running = status.reply?.payload?.running?.['runtime-smoke-instance']
    const recovery = status.reply?.payload?.recovery?.['runtime-smoke-instance']
    if (running?.pid > 0 && running.pid !== originalPid
      && recovery?.phase === 'monitoring' && recovery.restart_attempts === 1) {
      return { running, recovery }
    }
    await sleep(50)
  }
  throw new Error(`runtime did not complete the scheduled recovery: ${JSON.stringify(status)}`)
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

async function removeTreeEventually(directory) {
  if (process.platform === 'win32' && fs.existsSync(directory) && cachedWindowsUserSid) {
    childProcess.execFileSync('icacls', [
      directory,
      '/inheritance:e',
      '/T',
      '/C',
    ], { stdio: 'ignore', windowsHide: true })
    childProcess.execFileSync('icacls', [directory, '/reset', '/T', '/C'], {
      stdio: 'ignore',
      windowsHide: true,
    })
  }
  let lastError
  for (let attempt = 0; attempt < 40; attempt += 1) {
    try {
      fs.rmSync(directory, { recursive: true, force: true })
      return
    } catch (error) {
      lastError = error
      await sleep(50)
    }
  }
  throw lastError
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
    const endpoint = {
      address: runtimeEndpoint(dataDir, token),
      controlToken: token,
      dataDir,
    }
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
      || !status.reply.payload?.capabilities?.includes('config_sync_ack_v1')
      || !status.reply.payload?.capabilities?.includes('deployment_revision_v1')
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

    const proxyConfig = {
      enabled: true,
      host: '127.0.0.1',
      port: proxyPort,
      api_keys: [{
        id: 'runtime-smoke-client',
        name: 'Runtime smoke client',
        key: `sha256:${crypto.createHash('sha256').update('runtime-smoke-proxy-key').digest('base64url')}`,
        enabled: true,
        strength_verified: true,
        scopes: ['inference', 'discovery'],
        requests_per_minute: 0,
      }],
      default_instance_id: 'runtime-smoke-instance',
      routes: [{
        id: 'runtime-smoke-route',
        enabled: true,
        model_alias: 'runtime-smoke-model',
        target_instance_id: 'runtime-smoke-instance',
        priority: 0,
      }],
      runtime_service_enabled: true,
    }
    const launchSpec = testLaunchSpec(dataDir, backendPort, proxyConfig)
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
    if (synced.reply?.result !== 'ack') {
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
      { command: 'start_instance', payload: { spec: crashingLaunchSpec(dataDir, backendPort, proxyConfig) } },
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
      { command: 'start_instance', payload: { spec: crashingLaunchSpec(dataDir, backendPort, proxyConfig) } },
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

    const recoveryMarker = path.join(dataDir, 'runtime-recovery-once.marker')
    fs.rmSync(recoveryMarker, { force: true })
    const recoveryLaunchSpec = recoverOnceLaunchSpec(dataDir, backendPort, proxyConfig)
    const recoverySynced = await request(
      endpoint,
      token,
      {
        command: 'sync_config',
        payload: {
          revision: initialRevision + 2,
          proxy_config: proxyConfig,
          instances: { [recoveryLaunchSpec.instance_id]: recoveryLaunchSpec.config },
        },
      },
      'sync-automatic-recovery-config',
    )
    if (recoverySynced.reply?.result !== 'ack') {
      throw new Error(`automatic recovery configuration sync failed: ${JSON.stringify(recoverySynced)}`)
    }
    const recoverOnce = await request(
      endpoint,
      token,
      { command: 'start_instance', payload: { spec: recoveryLaunchSpec } },
      'start-automatic-recovery',
    )
    if (recoverOnce.reply?.result !== 'instance' || recoverOnce.reply.payload?.pid <= 0) {
      throw new Error(`automatic recovery fixture did not start: ${JSON.stringify(recoverOnce)}`)
    }
    launchedPids.add(recoverOnce.reply.payload.pid)
    const recovered = await waitForAutomaticRecovery(endpoint, token, recoverOnce.reply.payload.pid)
    launchedPids.delete(recoverOnce.reply.payload.pid)
    launchedPids.add(recovered.running.pid)
    if (recovered.recovery.origin_failure?.kind !== 'unexpected_exit') {
      throw new Error(`automatic recovery lost its originating failure: ${JSON.stringify(recovered)}`)
    }
    const stopRecovered = await request(
      endpoint,
      token,
      { command: 'stop_instance', payload: { instance_id: launchSpec.instance_id } },
      'stop-automatically-recovered-instance',
    )
    if (stopRecovered.reply?.result !== 'ack') {
      throw new Error(`automatically recovered instance stop failed: ${JSON.stringify(stopRecovered)}`)
    }
    launchedPids.delete(recovered.running.pid)
    const stoppedRecoveryStatus = await request(
      endpoint,
      token,
      { command: 'get_status' },
      'status-after-recovery-stop',
    )
    if (stoppedRecoveryStatus.reply?.payload?.recovery?.[launchSpec.instance_id]) {
      throw new Error(`expected stop retained a recovery incident: ${JSON.stringify(stoppedRecoveryStatus)}`)
    }
    await request(endpoint, token, { command: 'clear_last_error' }, 'clear-recovery-error')

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
    await removeTreeEventually(dataDir)
  }
}

main().catch(error => {
  console.error(error)
  process.exitCode = 1
})
