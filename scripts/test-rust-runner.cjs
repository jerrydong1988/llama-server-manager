const assert = require('node:assert/strict')
const path = require('node:path')
const { test } = require('node:test')
const { collectMacroArtifact, expectedManifests } = require('./test-rust.cjs')

test('Cargo artifact discovery accepts cached flat and per-crate build layouts', () => {
  for (const extension of ['dll', 'so', 'dylib']) {
    for (const directory of ['target/debug/deps', 'build cache/debug/build/glib-macros/hash/out']) {
      const library = path.resolve(directory, `libglib_macros-hash.${extension}`)
      const artifacts = {}
      collectMacroArtifact({
        reason: 'compiler-artifact', fresh: true,
        manifest_path: expectedManifests.glib_macros,
        target: { name: 'glib_macros', kind: ['proc-macro'] },
        filenames: [library + '.lib', library],
      }, artifacts)
      assert.deepEqual(artifacts, { glib_macros: library })
    }
  }
})

test('Macro discovery rejects other versions, crate kinds and non-library outputs', () => {
  const valid = {
    reason: 'compiler-artifact',
    manifest_path: expectedManifests.glib_macros,
    target: { name: 'glib_macros', kind: ['proc-macro'] },
    filenames: [path.resolve('macro.dll')],
  }
  for (const changes of [
    { reason: 'compiler-message' },
    { manifest_path: path.resolve('other/glib-macros/Cargo.toml') },
    { target: { name: 'glib_macros', kind: ['lib'] } },
    { target: { name: 'unexpected', kind: ['proc-macro'] } },
    { filenames: ['macro.dll.lib', 'macro.rmeta'] },
  ]) {
    const artifacts = {}
    collectMacroArtifact({ ...valid, ...changes }, artifacts)
    assert.deepEqual(artifacts, {})
  }
})
