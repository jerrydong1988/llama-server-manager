# Security backports for the Tauri 2 / GTK3 ABI

These four packages are copied from the immutable crates.io releases named by
their directory. Their original source, tests and licenses are retained. Cargo's
`[patch.crates-io]` selects them; application APIs and package versions are unchanged.

- `glib 0.18.5`: backport gtk-rs/gtk-rs-core PR 1343, making the C out-pointer
  mutable in `VariantStrIter::impl_get`. This fixes RUSTSEC-2024-0429 without mixing
  incompatible GLib major versions with GTK3. https://github.com/gtk-rs/gtk-rs-core/pull/1343
- `glib-macros 0.18.5` and `gtk3-macros 0.18.2`: replace the unmaintained
  `proc-macro-error` dependency with direct `syn::Error` propagation and compile-error
  tokens. The successor `proc-macro-error2` is also unmaintained (RUSTSEC-2026-0173)
  and is deliberately absent. Public macro signatures and successful expansions
  are preserved; invalid inputs remain compiler errors without panic-based control flow.
- `urlpattern 0.3.0`: backport the upstream switch from UNIC identifier properties
  to ICU4X `IdStart` / `IdContinue`. This removes the five unmaintained UNIC crates
  and updates Unicode identifier coverage, without changing URLPattern's public API.
  https://github.com/denoland/rust-urlpattern/blob/main/src/tokenizer.rs

`scripts/check-vendor-patches.cjs` guards the selected local packages, patch content
and absence of the retired dependencies from the application lockfile. The ten
GTK3 unmaintained advisories were withdrawn on 2026-08-14 and removed from the
allowlist during the 2026-10-16 review. The dated policy retains only the patched
GLib advisory while Tauri still requires the GLib 0.18 ABI.
Do not describe this backport as an upstream fixed release. Remove these patches
when a supported Tauri release supplies the equivalent fixes. Linux CI must build
the patched GTK/GLib chain and run its iterator regression in an optimized build.

## Review on 2026-11-15

The RustSec snapshot is `4ef2df8e806a65e2f1ecc14f56f51249062b23d6`.
[RUSTSEC-2024-0429](https://github.com/RustSec/advisory-db/blob/4ef2df8e806a65e2f1ecc14f56f51249062b23d6/crates/glib/RUSTSEC-2024-0429.md)
still names GLib >=0.20 as the upstream fix. The local `impl_get` contains both
changes from [gtk-rs/gtk-rs-core#1343](https://github.com/gtk-rs/gtk-rs-core/pull/1343):
`let mut p` and the `&mut p` C out-argument. Cargo selects this local 0.18.5 package
through `[patch.crates-io]`; the locked Tauri 2.11.5/GTK3 chain still uses GLib 0.18.

The Linux regression exercises `next`, `next_back`, `nth`, `nth_back` and `last`.
`[profile.dev.package.glib] opt-level = 2` keeps the affected dependency optimized
during tests. Both [x64](https://github.com/jerrydong1988/llama-server-manager/actions/runs/43675490434/job/132700372194)
and [ARM64](https://github.com/jerrydong1988/llama-server-manager/actions/runs/43675490434/job/132700372230)
passed this regression on `e8de427`. The release PR runs it again on both targets.
This verifies the source backport and its consumer, not an upstream 0.18 fixed release.
Cargo audit does not substitute for this source/runtime evidence for a path-patched crate.

The refreshed lockfile also fixes the rustix and Tokio soundness advisories with
1.1.6 and 1.53.3, respectively, and replaces yanked async-executor 1.14.0 with 1.14.1.
The audit reports no vulnerabilities, unsoundness warnings or yanked packages in
the registry dependency graph. Three maintenance advisories remain visible for
zbus_names, zvariant and zvariant_derive (RUSTSEC-2026-0371/0370/0372); these are
owned by the locked zbus 5/Tauri plugin chain, and their upstream replacement is
the incompatible zbus 6 API. They are not added to the ignore list. The sole GLib
exception remains dated and must be reviewed again by 2026-12-15.
