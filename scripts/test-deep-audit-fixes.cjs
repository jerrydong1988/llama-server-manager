const fs = require('node:fs')
const path = require('node:path')
const Module = require('node:module')
const esbuild = require('esbuild')
const root = path.resolve(__dirname, '..')
const source = String.raw`
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { exportShellCommand, maskCommandArguments, maskStartupCommandSecrets } from './src/store/commandFormatting'
import { createInstanceSlice } from './src/store/instanceSlice'
import { createMonitoringSlice } from './src/store/monitoringSlice'
import { formatConfigValue } from './src/components/ConfigPage/configWorkspace'

async function run() {
  const literals = ['', 'a b', 'a"b', "a'b", '$env:PATH', '$(echo escaped)', 'x;echo escaped', 'a|b', String.fromCharCode(96) + 'echo escaped' + String.fromCharCode(96), 'line\nvalue', 'C:\\model dir\\']
  const argv = [process.execPath, '-e', 'process.stdout.write(JSON.stringify(process.argv.slice(1)))', ...literals]
  const shell = process.platform === 'win32' ? 'powershell' : 'posix'
  const text = exportShellCommand(argv, shell)
  const child = spawnSync(shell === 'powershell' ? 'pwsh' : '/bin/sh', shell === 'powershell' ? ['-NoProfile', '-NonInteractive', '-Command', text] : ['-c', text], { encoding: 'utf8', windowsHide: true })
  assert.equal(child.status, 0, child.stderr)
  assert.deepEqual(JSON.parse(child.stdout), literals, 'copied command preserves literal argv without interpolation')
  assert.deepEqual(maskCommandArguments(['x', '--hf-token', 'secret', '-hft=secret', '--api-key=secret', '--port', '1']), ['x', '--hf-token', '********', '-hft=********', '--api-key=********', '--port', '1'])
  assert.ok(!maskStartupCommandSecrets('x --hf-token "a secret" -hft=second --api-key third').includes('secret'))
  assert.equal(formatConfigValue('api_key', 'secret', { emptyValue: 'empty' }, {} as any), '********')
  assert.ok(!formatConfigValue('manual_command', 'x --hf-token secret', { emptyValue: 'empty' }, {} as any).includes('secret'))

  let state: any = { instances: [{ id: 'gone' }, { id: 'live' }], checkpointStatuses: { gone: {}, live: {} }, logs: { gone: [], live: [] }, recentLogs: [{ instanceId: 'gone', timestamp: 1 }], monitoringFramesByInstance: { gone: [] }, monitoringCurrentByInstance: { gone: {} }, runningTasksByInstance: { gone: [] }, lastCompletedTaskByInstance: { gone: {} }, instanceLifecycle: { gone: {} } }
  const set = (update: any) => { state = { ...state, ...(typeof update === 'function' ? update(state) : update) } }
  Object.assign(state, createInstanceSlice(set, () => state, []), createMonitoringSlice(set))
  const frame = { instanceId: 'gone', ts: 5, sessionStartedAt: 1 }
  state.ingestMonitoringFrame(frame)
  state.deleteInstance('gone')
  state.hydrateMonitoringFrames([frame])
  state.applyPerfUpdate({ instanceId: 'gone', tasks: [], lastCompleted: {} })
  await new Promise(resolve => setTimeout(resolve, 80))
  for (const name of ['checkpointStatuses', 'logs', 'monitoringFramesByInstance', 'monitoringCurrentByInstance', 'runningTasksByInstance', 'lastCompletedTaskByInstance', 'instanceLifecycle']) assert.ok(!('gone' in state[name]), name)
  state.addLogs([{ instanceId: 'gone', timestamp: 2, message: 'late' }, { instanceId: 'live', timestamp: 3, message: 'normal' }])
  assert.ok(!('gone' in state.logs))
  assert.equal(state.logs.live.length, 1)
  assert.ok(state.recentLogs.every((entry: any) => entry.instanceId === 'live'))
  console.log('Deep audit frontend and shell round-trip regressions passed.')
}
run().catch(error => { console.error(error); process.exitCode = 1 })
`
const bundle = esbuild.buildSync({ stdin: { contents: source, resolveDir: root, sourcefile: 'deep-audit-fixes.ts', loader: 'ts' }, bundle: true, platform: 'node', format: 'cjs', write: false, logLevel: 'silent' })
const test = new Module(__filename, module)
test.filename = __filename
test.paths = module.paths
test._compile(bundle.outputFiles[0].text, __filename)
