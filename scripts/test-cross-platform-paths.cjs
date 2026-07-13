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

const { isPathWithinRoot, pathJoin } = loadTypeScriptModule('src/utils/path.ts')
const { formatHostPort, parseHostPort, httpUrl } = loadTypeScriptModule('src/utils/network.ts')

assert.equal(pathJoin('/home/jerry/models', 'org/model', 'file.gguf'), '/home/jerry/models/org/model/file.gguf')
assert.equal(pathJoin('//server/share/models', 'org/model', 'file.gguf'), '//server/share/models/org/model/file.gguf')
assert.equal(pathJoin('C:\\models', 'org/model', 'file.gguf'), 'C:/models/org/model/file.gguf')
assert.equal(pathJoin('models', 'org/model', 'file.gguf'), 'models/org/model/file.gguf')
assert.equal(pathJoin('/', 'models'), '/models')
assert.equal(isPathWithinRoot('/models/A/file.gguf', '/models/A'), true)
assert.equal(isPathWithinRoot('/models/a/file.gguf', '/models/A'), false)
assert.equal(isPathWithinRoot('/models/AB/file.gguf', '/models/A'), false)

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
