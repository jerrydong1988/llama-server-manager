// Local Windows manual-test artifact. Never installs, tags or publishes a release.
const fs = require('node:fs')
const path = require('node:path')
const cp = require('node:child_process')
const crypto = require('node:crypto')
const assert = require('node:assert/strict')
const args = Object.fromEntries(Array.from({ length: (process.argv.length - 2) / 2 }, (_, i) => [process.argv[2 + i * 2].slice(2), process.argv[3 + i * 2]]))
assert.equal(process.platform, 'win32')
for (const key of ['output', 'launcher']) assert.ok(args[key] && path.isAbsolute(args[key]), `--${key} must be absolute`)
assert.ok(!fs.existsSync(args.output), 'Use a new package directory')
const source = JSON.parse(fs.readFileSync(args.launcher, 'utf8'))
const gufoVersion = cp.execFileSync(source.launcher.executable, ['--version'], { encoding: 'utf8', windowsHide: true, timeout: 15000 }).trim()
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
const revision = cp.execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
const output = args.output
fs.mkdirSync(path.join(output, 'bin'), { recursive: true })
const managerHashes = {}
for (const name of ['llama-server-manager.exe', 'lsm.exe']) {
  const target = path.join(output, 'bin', name)
  fs.copyFileSync(path.resolve('src-tauri/target/release', name), target)
  managerHashes[name] = hash(target)
}
fs.cpSync(path.dirname(source.launcher.executable), path.join(output, 'gufo'), { recursive: true })
// Preserve the launcher script's Unicode text in Windows PowerShell 5.1.
fs.writeFileSync(path.join(output, 'Launch.ps1'), '\ufeff' + fs.readFileSync('scripts/gufo-package-launcher.ps1', 'utf8'))
for (const action of ['Start', 'Stop', 'Status']) fs.writeFileSync(path.join(output, `${action}.cmd`), `@echo off\r\npowershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0Launch.ps1" -Action ${action}\r\nif errorlevel 1 pause\r\n${action === 'Status' ? 'pause\r\n' : ''}`)
const profiles = source.presets.map((preset, index) => {
  const s = preset.settings
  return { id: `gufo-trial-${index}`, config: { engine_kind: 'gufo', name: `${preset.name} [Gufo test]`,
    model_path: s.model, mmproj_path: s.mmproj, alias: s.served_model_name, host: '127.0.0.1', port: 18581 + index,
    ctx_size: 262144, ctx_size_auto: false, n_predict: s.max_tokens,
    temp: s.temperature, top_k: s.top_k, top_p: s.top_p, min_p: s.min_p, seed: s.seed,
    repeat_penalty: s.repeat_penalty, repeat_last_n: s.repeat_last_n, presence_penalty: s.presence_penalty, frequency_penalty: s.frequency_penalty,
    gufo: { model_family: s.model_family, speculative: s.speculative, mtp_model: s.mtp_model, dflash_model: s.dflash_model,
      draft_tokens: s.draft_tokens, sessions: 1, think: s.think, reasoning_effort: s.reasoning_effort, preserve_thinking: s.preserve_thinking,
      cache_ram_mode: 'auto', cache_ram_gib: 4, cache_disk: true, cache_disk_gib: s.cache_disk_gib, startup_timeout_seconds: 600 } } }
})
fs.writeFileSync(path.join(output, 'profiles.json'), JSON.stringify(profiles, null, 2))
fs.writeFileSync(path.join(output, 'BUILD.json'), JSON.stringify({ package_id: path.basename(output), version: require('../package.json').version,
  revision, dirty: Boolean(cp.execFileSync('git', ['status', '--porcelain'], { encoding: 'utf8' }).trim()), built_at: new Date().toISOString(),
  formal_release: false, platform: 'windows-x86_64', gufo_version: gufoVersion,
  gufo_sha256: hash(source.launcher.executable), manager_sha256: managerHashes }, null, 2))
fs.copyFileSync('docs/GUFO_MANUAL_TEST.md', path.join(output, 'README.md'))
fs.copyFileSync('docs/GUFO_PHASE1.md', path.join(output, 'INTEGRATION.md'))
fs.copyFileSync('LICENSE', path.join(output, 'MANAGER-LICENSE.txt'))
function files(directory) { return fs.readdirSync(directory, { withFileTypes: true }).flatMap(e => e.isDirectory() ? files(path.join(directory,e.name)) : [path.join(directory,e.name)]) }
fs.writeFileSync(path.join(output, 'SHA256SUMS.txt'), files(output).sort().map(file => `${hash(file)}  ${path.relative(output,file)}`).join('\r\n') + '\r\n')
console.log(JSON.stringify({ directory: output, files: files(output).length, revision }))
