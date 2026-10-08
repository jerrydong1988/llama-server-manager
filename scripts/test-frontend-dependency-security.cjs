const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const { createRequire } = require('node:module')
const { spawnSync } = require('node:child_process')

const root = path.resolve(__dirname, '..')
const braces = require('braces')
const complexityError = error => error instanceof SyntaxError && error.code === 'ERR_BRACES_COMPLEXITY'

function adversarial(entry) {
  const direct = entry.startsWith('lib/')
  const method = entry.replace('lib/', '')
  const invoke = direct ? require(`braces/lib/${method}`) : braces[method]
  if (!direct) {
    for (const input of [
      '{'.repeat(4998) + 'a,b' + '}'.repeat(4998),
      '('.repeat(4998) + 'x' + ')'.repeat(4998),
      '{('.repeat(1000) + 'a,b' + ')}'.repeat(1000),
      '{'.repeat(1000) + 'x',
    ]) {
      assert.throws(() => invoke(input, { maxLength: 65536, maxDepth: Infinity }), complexityError)
    }
  }
  if (method === 'parse') return

  let deep = { type: 'text', value: 'x' }
  for (let i = 0; i < 10000; i += 1) deep = { type: 'root', nodes: [deep] }
  const cycle = { type: 'root', nodes: [] }
  cycle.nodes.push(cycle)
  const shared = { type: 'text', value: 'x' }
  const wide = { type: 'root', nodes: Array(65536).fill(shared) }
  let arrayValue = 'x'
  for (let i = 0; i < 10000; i += 1) arrayValue = [arrayValue]
  const nestedArray = { type: 'root', nodes: [{ type: 'text', value: arrayValue }] }
  for (const ast of [deep, cycle, wide, nestedArray]) {
    assert.throws(() => invoke(ast), complexityError)
  }
  // Caller-provided ancestry and stale queues must not become traversal input.
  const ast = braces.parse('x{a,b}')
  const foreign = { type: 'other', queue: [arrayValue] }
  foreign.parent = foreign
  for (const node of ast.nodes) node.parent = foreign
  if (method === 'expand') assert.deepEqual(invoke(ast), ['xa', 'xb'])
  else assert.equal(typeof invoke(ast), 'string')
}

async function main() {
  const installed = fs.realpathSync(require.resolve('braces'))
  assert.equal(installed, fs.realpathSync(path.join(root, 'vendor/braces/index.js')))
  const fork = require('braces/package.json')
  assert.equal(fork.name, '@llama-server-manager/braces')
  assert.equal(fork.version, '3.0.3-lsm.1')
  assert.equal(fork.scripts, undefined, 'the local fork must have no lifecycle scripts')
  for (const consumer of ['micromatch', 'chokidar']) {
    const consumerRequire = createRequire(require.resolve(`${consumer}/package.json`))
    assert.equal(fs.realpathSync(consumerRequire.resolve('braces')), installed, consumer)
  }
  const globRequire = createRequire(require.resolve('fast-glob/package.json'))
  const micromatchRequire = createRequire(globRequire.resolve('micromatch/package.json'))
  assert.equal(fs.realpathSync(micromatchRequire.resolve('braces')), installed)
  const lock = require('../package-lock.json')
  for (const [location, metadata] of Object.entries(lock.packages)) {
    if (location.endsWith('node_modules/braces')) {
      assert.equal(metadata.link, true, location)
      assert.equal(metadata.resolved, 'vendor/braces', location)
    }
    if (location.endsWith('node_modules/brace-expansion')) assert.equal(metadata.version, '5.0.12')
    if (location.endsWith('node_modules/source-map-js')) assert.equal(metadata.version, '1.2.2')
    if (location.endsWith('node_modules/postcss-selector-parser')) assert.equal(metadata.version, '7.1.6')
  }

  for (const entry of ['parse', 'compile', 'expand', 'stringify', 'lib/compile', 'lib/expand', 'lib/stringify']) {
    const result = spawnSync(process.execPath, ['--max-old-space-size=128', __filename, '--adversarial', entry], {
      cwd: root, encoding: 'utf8', timeout: 10000,
    })
    assert.equal(result.error, undefined, `${entry}: ${result.error}`)
    assert.equal(result.status, 0, `${entry}: ${result.stderr}`)
  }
  const malicious = '{'.repeat(4998) + 'a,b' + '}'.repeat(4998)
  assert.throws(() => require('micromatch').braces(malicious, { expand: true }), complexityError)
  assert.throws(() => require('fast-glob').generateTasks([malicious]), complexityError)

  const controls = [
    ['src/**/*.{js,ts,jsx,tsx}', {}, ['src/**/*.js', 'src/**/*.ts', 'src/**/*.jsx', 'src/**/*.tsx']],
    ['a{b,{c,d}}e', {}, ['abe', 'ace', 'ade']],
    ['v{1..5..2}', {}, ['v1', 'v3', 'v5']],
    ['{a..c}', {}, ['a', 'b', 'c']],
    ['{,a,a}', { noempty: true, nodupes: true }, ['a']],
    ['${a,b}', {}, ['${a,b}']],
    ['{a,b', {}, ['{a,b']],
    ['a\\{b,c\\}', {}, ['a{b,c}']],
    ['a\\{b,c\\}', { keepEscaping: true }, ['a\\{b,c\\}']],
    ['{"a,b",c}', {}, ['a,b', 'c']],
    ['(a{b,c})', {}, ['(ab)', '(ac)']],
  ]
  for (const [input, options, expected] of controls) assert.deepEqual(braces.expand(input, options), expected, input)
  assert.throws(() => braces.expand('{1..1001}'), /range limit/)
  assert.equal(braces.expand('{1..1001}', { rangeLimit: false }).length, 1001)
  assert.equal(braces.compile('a{b,c}'), 'a(b|c)')
  assert.equal(braces.stringify(braces.parse('a{b,c}')), 'a{b,c}')
  const shared = { type: 'text', value: 'x' }
  assert.equal(braces.stringify({ type: 'root', nodes: [shared, shared] }), 'xx')
  assert.equal(braces.expand('{'.repeat(127) + 'x' + '}'.repeat(127)).length, 1)
  assert.throws(() => braces.expand('{'.repeat(129) + 'x' + '}'.repeat(129)), complexityError)
  assert.ok(require('fast-glob').sync(['index.html', 'src/**/*.{js,ts,jsx,tsx}'], { cwd: root }).length > 0)

  // Exercise selector-parser 7 through its real Tailwind and postcss-nested callers.
  const postcss = require('postcss')
  const css = await postcss([require('tailwindcss')({
    content: [{ raw: 'dark:bg-black group-hover:text-white peer-checked:block [&>a]:underline hover:!mt-2' }],
    darkMode: 'class', corePlugins: { preflight: false },
  })]).process('@tailwind utilities;', { from: undefined })
  for (const fragment of ['.dark', '.group:hover', '.peer:checked', '>a', 'margin-top: 0.5rem !important']) {
    assert.ok(css.css.includes(fragment), fragment)
  }
  const nested = await postcss([require('postcss-nested')]).process('.a { & + & { color: red } .b, .c { display: block } }', { from: undefined })
  assert.match(nested.css, /\.a\s*\+\s*\.a/)
  assert.match(nested.css, /\.a \.b, \.a \.c/)
  console.log('Frontend dependency security regression passed: bounded inputs, installed consumers, globs and CSS controls.')
}

if (process.argv[2] === '--adversarial') adversarial(process.argv[3])
else main().catch(error => { console.error(error); process.exitCode = 1 })
