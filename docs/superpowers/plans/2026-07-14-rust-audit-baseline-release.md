# Rust Audit Baseline and v2.9.26 Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove actionable CI dependency warnings, enforce an exact Tauri 2 advisory baseline, and publish verified v2.9.26 packages.

**Architecture:** Keep RustSec auditing enabled and pin its Node 24 implementation. Resolve the yanked package through the lockfile, represent only upstream-blocked advisories as explicit workflow inputs, and protect the policy with repository regression checks before release.

**Tech Stack:** GitHub Actions, Rust/Cargo, RustSec cargo-audit, Tauri 2, Node.js regression scripts.

## Global Constraints

- Do not disable informational RustSec warnings globally.
- Allow exactly the 17 advisory IDs listed in the approved design and no wildcard rules.
- Any new vulnerability or warning remains visible and blocks release review.
- Keep Windows, macOS, Linux x64 and Linux ARM64 packages.
- Publish version 2.9.26 only after PR and tag workflows both pass.

---

### Task 1: Lockfile and Audit Policy

**Files:**
- Modify: `src-tauri/Cargo.lock`
- Modify: `.github/workflows/build.yml`
- Modify: `scripts/check-cross-platform-release.cjs`

**Interfaces:**
- Consumes: current Cargo dependency graph and Build workflow.
- Produces: Node 24 RustSec audit with an exact advisory baseline and a non-yanked lockfile.

- [x] **Step 1: Add failing workflow policy assertions**

Require the Node 24 RustSec commit, every approved advisory ID, exactly 17 unique IDs, no `@v2.0.0`, no `ACTIONS_ALLOW_USE_UNSECURE_NODE_VERSION`, and no broad audit disable switch.

- [x] **Step 2: Run the release policy test and verify failure**

Run: `node scripts/check-cross-platform-release.cjs`

Expected: failure because the workflow still uses `rustsec/audit-check@v2.0.0` and has no exact baseline.

- [x] **Step 3: Update compatible Rust dependencies**

Run: `cargo update --manifest-path src-tauri/Cargo.toml`

Verify `spin 0.9.8` is absent and `spin 0.9.9` is present.

- [x] **Step 4: Pin the audit Action and exact advisory list**

Use `RustSec/audit-check@858dc40f52ca2b8570b7a997c1c4e35c6fc9a432` and its comma-separated `ignore` input. Document Tauri GTK3 and tauri-utils/urlpattern ownership next to the list.

- [x] **Step 5: Run focused and complete policy checks**

Run:

```bash
node scripts/check-cross-platform-release.cjs
npm run check:release
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
```

Expected: all checks pass with no compiler warnings.

### Task 2: Version and Release Documentation

**Files:**
- Modify: `package.json`
- Modify: `package-lock.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/tauri.conf.json`
- Modify: `GUIDE.md`

**Interfaces:**
- Consumes: verified dependency policy.
- Produces: internally consistent v2.9.26 metadata and user guide.

- [x] **Step 1: Update all version sources to 2.9.26**

Change only the application package versions; do not rewrite dependency package versions manually.

- [x] **Step 2: Run version and guide checks**

Run:

```bash
node scripts/test-release-tag-version.cjs
node scripts/check-guide.cjs
npm run check:encoding
```

Expected: all checks pass and GUIDE declares v2.9.26.

### Task 3: Pull Request Verification

**Files:**
- Modify only if CI exposes a defect.

**Interfaces:**
- Consumes: Tasks 1 and 2.
- Produces: clean cross-platform test artifacts and zero unexpected audit annotations.

- [x] **Step 1: Run final local release checks**

Run formatting, `npm run check:release`, `npm run build`, all Rust tests and strict Clippy.

- [ ] **Step 2: Commit and push the branch**

Commit dependency policy and v2.9.26 metadata, then update PR #2.

- [ ] **Step 3: Monitor PR CI to completion**

Require `quality`, `build-windows`, `build-macos`, `build-linux` and `build-linux-arm64` to succeed. Read annotations and prove the old Node 20 and 18-warning summaries are absent.

### Task 4: Formal Release

**Files:**
- Create locally only: release notes input for `gh release edit`; do not commit a scratch file.

**Interfaces:**
- Consumes: successful PR CI.
- Produces: merged master, v2.9.26 tag and published GitHub Release assets.

- [ ] **Step 1: Mark PR ready and merge to master**

Use a merge strategy that preserves the reviewed commits. Confirm master contains the exact tested head.

- [ ] **Step 2: Create and push v2.9.26 tag**

The tag must point to the merged release commit and pass the tag/version validation.

- [ ] **Step 3: Monitor the tag workflow**

Require all five jobs to pass and all four artifact groups to upload. Diagnose and fix any failure before continuing.

- [ ] **Step 4: Publish complete release notes**

Explain vector runtime monitoring, dependency audit changes, controlled upstream exceptions, validation results, signing fallback and package choices in Chinese.

- [ ] **Step 5: Verify release assets and repository hygiene**

Confirm the release is public, not draft or prerelease, assets are present, the working tree is clean, and no scratch files remain.
