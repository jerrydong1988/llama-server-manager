const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const esbuild = require('esbuild')

function loadTypeScriptModule(relativePath) {
  const absolutePath = path.join(process.cwd(), relativePath)
  const source = fs.readFileSync(absolutePath, 'utf8')
  const { code } = esbuild.transformSync(source, {
    format: 'cjs',
    loader: 'ts',
    target: 'node20',
  })
  const loaded = { exports: {} }
  const evaluate = new Function('module', 'exports', 'require', '__filename', '__dirname', code)
  evaluate(loaded, loaded.exports, require, absolutePath, path.dirname(absolutePath))
  return loaded.exports
}

const {
  dedupePaths,
  formatPathForDisplay,
  isPathWithinRoot,
  normalizePath,
  pathComparisonKey,
  pathDirname,
  pathJoin,
  pathsEqual,
} = loadTypeScriptModule('src/utils/path.ts')
const { formatHostPort, parseHostPort, httpUrl } = loadTypeScriptModule('src/utils/network.ts')

assert.equal(pathJoin('/home/jerry/models', 'org/model', 'file.gguf'), '/home/jerry/models/org/model/file.gguf')
assert.equal(pathJoin('//server/share/models', 'org/model', 'file.gguf'), '//server/share/models/org/model/file.gguf')
assert.equal(pathJoin('C:\\models', 'org/model', 'file.gguf'), 'C:/models/org/model/file.gguf')
assert.equal(pathJoin('\\\\?\\C:\\models', 'org/model', 'file.gguf'), 'C:/models/org/model/file.gguf')
assert.equal(pathJoin('\\\\?\\UNC\\server\\share\\models', 'org/model'), '//server/share/models/org/model')
assert.equal(pathJoin('models', 'org/model', 'file.gguf'), 'models/org/model/file.gguf')
assert.equal(pathJoin('/', 'models'), '/models')
assert.equal(normalizePath('\\\\?\\C:\\Models\\.\\Qwen\\..\\Llama\\'), 'C:/Models/Llama')
assert.equal(normalizePath('\\\\?\\UNC\\Server\\Share\\Models'), '//Server/Share/Models')
assert.equal(formatPathForDisplay('\\\\?\\c:\\Models\\.\\Qwen\\'), 'C:\\Models\\Qwen')
assert.equal(formatPathForDisplay('c:/Models/Qwen'), 'C:\\Models\\Qwen')
assert.equal(formatPathForDisplay('\\\\?\\UNC\\Server\\Share\\Models'), '\\\\Server\\Share\\Models')
assert.equal(formatPathForDisplay('\\\\Server\\Share\\Models\\'), '\\\\Server\\Share\\Models')
assert.equal(formatPathForDisplay('\\\\.\\PhysicalDrive0'), '\\\\.\\PhysicalDrive0')
assert.equal(formatPathForDisplay('\\\\?\\Volume{abc}\\Models'), '\\\\?\\Volume{abc}\\Models')
assert.equal(formatPathForDisplay('/opt/models/Qwen/'), '/opt/models/Qwen')
assert.equal(formatPathForDisplay('/opt/models/Qwen\\literal'), '/opt/models/Qwen\\literal')
assert.equal(formatPathForDisplay('weights/model.gguf'), 'weights/model.gguf')
assert.equal(formatPathForDisplay('http://127.0.0.1:8080/v1/models'), 'http://127.0.0.1:8080/v1/models')
assert.equal(pathComparisonKey('  \\\\?\\C:\\Models\\Llama  '), 'c:/models/llama')
assert.equal(pathsEqual('C:\\Models', '\\\\?\\c:\\models\\'), true)
assert.equal(pathsEqual('\\\\Server\\Share\\Models', '\\\\?\\UNC\\server\\share\\models'), true)
assert.equal(pathsEqual('/models/A', '/models/a'), false)
assert.deepEqual(
  dedupePaths(['C:\\Models', '\\\\?\\c:\\models\\', '\\\\Server\\Share', '\\\\?\\UNC\\server\\share']),
  ['C:\\Models', '\\\\Server\\Share'],
)
assert.equal(pathDirname('C:\\Models\\file.gguf'), 'C:/Models')
assert.equal(pathDirname('C:\\Models'), 'C:/')
assert.equal(pathDirname('/models/file.gguf'), '/models')
assert.equal(isPathWithinRoot('/models/A/file.gguf', '/models/A'), true)
assert.equal(isPathWithinRoot('/models/a/file.gguf', '/models/A'), false)
assert.equal(isPathWithinRoot('/models/AB/file.gguf', '/models/A'), false)
assert.equal(isPathWithinRoot('\\\\?\\C:\\Models\\Qwen\\file.gguf', 'c:/models'), true)
assert.equal(isPathWithinRoot('C:\\Models-Old\\file.gguf', '\\\\?\\c:\\models'), false)
assert.equal(isPathWithinRoot('\\\\?\\UNC\\SERVER\\Share\\Models\\file.gguf', '\\\\server\\share\\models'), true)
assert.equal(isPathWithinRoot('C:\\Models', ''), false)
assert.equal(isPathWithinRoot('./weights/model.gguf', '.'), true)
assert.equal(isPathWithinRoot('../outside/model.gguf', '.'), false)

assert.equal(formatHostPort('127.0.0.1', 50052), '127.0.0.1:50052')
assert.equal(formatHostPort('worker.local', 50052), 'worker.local:50052')
assert.equal(formatHostPort('::1', 50052), '[::1]:50052')
assert.equal(formatHostPort('[::1]', 50052), '[::1]:50052')
assert.deepEqual(parseHostPort('[::1]:50052', 80), { host: '::1', port: 50052 })
assert.deepEqual(parseHostPort('::1', 50052), { host: '::1', port: 50052 })
assert.deepEqual(parseHostPort('worker.local:50053', 50052), { host: 'worker.local', port: 50053 })
assert.deepEqual(parseHostPort('worker.local', 50052), { host: 'worker.local', port: 50052 })
assert.equal(httpUrl('::1', 8080), 'http://[::1]:8080')

console.log('cross-platform path regression passed')
