const assert = require('node:assert/strict')
const fs = require('node:fs')
const path = require('node:path')
const root = path.resolve(__dirname, '..')
const read = name => fs.readFileSync(path.join(root, name), 'utf8')
const cargo = read('src-tauri/Cargo.toml')
const lock = read('src-tauri/Cargo.lock')
for (const crate of ['glib-0.18.5', 'glib-macros-0.18.5', 'gtk3-macros-0.18.2', 'urlpattern-0.3.0']) {
  assert.ok(cargo.includes(`path = "vendor/${crate}"`), `${crate} must remain selected`)
}
const glib = read('src-tauri/vendor/glib-0.18.5/src/variant_iter.rs')
assert.match(glib, /let mut p: \*mut libc::c_char = std::ptr::null_mut\(\)/)
assert.match(glib, /g_variant_get_child\([\s\S]*?&mut p,/)
const url = read('src-tauri/vendor/urlpattern-0.3.0/src/tokenizer.rs')
assert.match(url, /props::IdStart/)
assert.match(url, /props::IdContinue/)
for (const name of ['proc-macro-error', 'proc-macro-error2', 'unic-char-range', 'unic-char-property', 'unic-common', 'unic-ucd-version', 'unic-ucd-ident']) {
  assert.ok(!lock.includes(`name = "${name}"`), `${name} must not return to the dependency graph`)
}
console.log('Security dependency backport checks passed.')
