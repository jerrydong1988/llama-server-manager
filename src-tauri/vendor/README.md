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
