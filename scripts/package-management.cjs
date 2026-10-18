// Exploration artifacts only. Does not tag, release, sign, or modify updater channels.
const fs = require('node:fs')
const path = require('node:path')
const cp = require('node:child_process')
const crypto = require('node:crypto')
const root = path.resolve(__dirname, '..')
const output = path.join(root, '.temp', 'agent-management-packages')
const platform = `${process.platform}-${process.arch}`
const revision = cp.execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
const name = `llama-server-manager-agent-${platform}-${revision.slice(0, 8)}`
const staging = fs.mkdtempSync(path.join(require('node:os').tmpdir(), 'lsm-management-package-'))
const directory = path.join(staging, name)
fs.mkdirSync(directory); fs.mkdirSync(output, { recursive: true })
const extension = process.platform === 'win32' ? '.exe' : ''
try {
  for (const binary of ['llama-server-manager', 'lsm']) {
    fs.copyFileSync(path.join(root, 'src-tauri', 'target', 'release', binary + extension), path.join(directory, binary + extension))
    if (process.platform !== 'win32') fs.chmodSync(path.join(directory, binary), 0o755)
  }
  for (const doc of ['agent-management-validation.md', 'agent-management-exploration.md']) {
    fs.copyFileSync(path.join(root, 'docs', doc), path.join(directory, doc))
  }
  fs.writeFileSync(path.join(directory, 'BUILD.json'), JSON.stringify({ exploration: true, version: require('../package.json').version, revision, platform, built_at: new Date().toISOString(), formal_release: false }, null, 2) + '\n')
  const checksum = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')
  fs.writeFileSync(path.join(directory, 'SHA256SUMS.txt'), fs.readdirSync(directory).sort().map(file => `${checksum(path.join(directory, file))}  ${file}`).join('\n') + '\n')
  const archive = path.join(output, name + (process.platform === 'win32' ? '.zip' : '.tar.gz'))
  if (process.platform === 'win32') {
    const quote = value => "'" + value.replaceAll("'", "''") + "'"
    cp.execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', `Compress-Archive -LiteralPath ${quote(directory)} -DestinationPath ${quote(archive)} -Force`], { windowsHide: true, stdio: 'inherit' })
  } else cp.execFileSync('tar', ['-czf', archive, '-C', staging, name], { stdio: 'inherit' })
  fs.writeFileSync(archive + '.sha256', `${checksum(archive)}  ${path.basename(archive)}\n`)
  console.log(`${archive}\nSHA256 ${checksum(archive)}`)
} finally {
  const temporaryRoot = path.resolve(require('node:os').tmpdir()) + path.sep
  if (!path.resolve(staging).startsWith(temporaryRoot + 'lsm-management-package-')) throw new Error('unsafe staging cleanup path')
  fs.rmSync(staging, { recursive: true, force: true })
}
