# User Guide Refresh Design

## Status

- Approved direction: complete alignment across `GUIDE.md`, the in-app guide, the interactive tour, and the README guide entry.
- Target release baseline: `v2.9.22`.
- Primary audiences: first-time users setting up a local `llama-server`, existing users adopting newer routing and monitoring features, and release reviewers validating supported workflows.

## Problem Statement

The current documentation no longer matches the application surface. `GUIDE.md` still identifies itself as `v2.9.15`, describes eight tabs, and omits newer pages and workflows. The README uses unlabeled remote screenshots, while the in-app guide imports `GUIDE.md` but has no offline image set. The interactive tour covers only part of the current navigation.

The release needs one coherent, bilingual guide that is accurate in GitHub, readable offline inside the packaged desktop app, and visually grounded in the actual `v2.9.22` interface.

## Goals

1. Make `GUIDE.md` the single complete content source for repository and in-app documentation.
2. Align all instructions with the current navigation, labels, controls, persistence behavior, and runtime workflows.
3. Add current, real application screenshots and annotated workflow images that ship with the application.
4. Expand the interactive tour to cover every operational page that benefits from guided discovery.
5. Keep the README concise while giving users a reliable quick start and a clear route to the full guide.
6. Add automated checks that detect missing assets, stale version references, broken guide anchors, and incomplete tour coverage before release.

## Non-Goals

- No general frontend redesign.
- No new product functionality unrelated to documentation or guided discovery.
- No external documentation generator, hosted documentation service, or new test framework.
- No fabricated runtime results, fake benchmark numbers, or mock UI screenshots.
- No screenshots containing private API keys, local usernames, private paths, host addresses, or unrelated desktop content.

## Chosen Approach

Use `GUIDE.md` as the canonical full guide. Store screenshots under Vite's public asset tree so they are copied into the packaged frontend. Markdown uses repository-relative paths that GitHub can render, and the in-app renderer translates those paths to the corresponding runtime public URLs before calling `marked`.

The README remains a curated overview rather than a second full manual. Interactive tour definitions move into a focused data module so navigation coverage and selector uniqueness can be checked without parsing the full `GuidePage` component.

This approach avoids the drift of separately maintained manuals without introducing a document-generation system immediately before release.

## Documentation Architecture

### Canonical Guide

`GUIDE.md` will contain the full Chinese and English instructions in paired sections. Its top-level order follows the user's actual workflow:

1. Install and first launch.
2. Interface map and recommended setup path.
3. Dashboard and system health.
4. Model repository.
5. Download manager.
6. Engine manager.
7. Instance manager.
8. Parameter configuration and validation.
9. Cluster workers.
10. Instance routing and the OpenAI-compatible endpoint.
11. Performance monitoring and analysis.
12. Monitoring wall.
13. Server logs and troubleshooting.
14. In-app guide, command center, themes, language, tray behavior, auto-start, updates, and backup recovery.
15. FAQ and release-validation checklist.

Each operational section must include:

- What the page is for.
- Prerequisites.
- A numbered primary workflow.
- Important controls and status meanings.
- Failure or recovery guidance where applicable.
- A current screenshot with a descriptive bilingual caption.

The version shown in the repository guide is `v2.9.22`. The in-app renderer continues replacing semantic version text with the version imported from `package.json`, preventing a later package-only version bump from showing stale in-app text.

### README

`README.md` will provide:

- A short bilingual product summary.
- A five-step quick-start path.
- A labeled screenshot gallery using selected local guide assets.
- A concise current feature map including dashboard, downloads, instances, cluster, routing, performance, monitoring wall, and logs.
- Direct links to `GUIDE.md`, releases, and source-build instructions.

The existing anonymous remote attachment gallery will be replaced. README will not duplicate the guide's detailed control tables or troubleshooting material.

### In-App Guide

`GuidePage` continues rendering `GUIDE.md`, preserving its table of contents and setup checklist. It gains a small asset-path normalization step:

- Repository Markdown path: `public/docs/guide/<asset>.png`.
- Runtime path after normalization: `/docs/guide/<asset>.png`.

The renderer will retain sanitization. Image attributes needed for accessibility and stable rendering will be explicitly allowed, while unsafe protocols and event attributes remain rejected. Broken images must remain visually identifiable rather than silently collapsing the guide layout.

Internal heading links scroll within the guide. External HTTPS links open through the supported Tauri shell integration instead of navigating the application webview away from the product.

## Screenshot System

### Asset Location and Format

Committed assets live in `public/docs/guide/`. They are copied by Vite to `dist/docs/guide/` and therefore ship in every desktop package.

Screenshots use PNG for predictable GitHub and WebView rendering. Capture viewport is 1440 by 900 in the current dark theme unless a page requires a taller view. Images are losslessly optimized or reduced to a practical size while preserving readable text.

### Required Real Screenshots

1. `01-dashboard.png`
2. `02-model-repository.png`
3. `03-download-manager.png`
4. `04-engine-manager.png`
5. `05-instance-manager.png`
6. `06-configuration.png`
7. `07-cluster-manager.png`
8. `08-instance-routing.png`
9. `09-performance.png`
10. `10-monitoring-wall.png`
11. `11-server-logs.png`
12. `12-in-app-guide.png`

Each image must come from the current development build. A page may show a genuine empty or inactive state when the required hardware or running service is unavailable; no values will be invented.

### Required Annotated Workflow Images

1. `flow-01-first-run.png`: model directory, engine directory, and instance creation sequence.
2. `flow-02-start-and-diagnose.png`: start, health state, performance view, and logs.
3. `flow-03-route-requests.png`: running instances, model aliases, proxy start, and unified endpoint.

Annotations use numbered callouts, short bilingual labels, solid high-contrast borders, and arrows that do not cover control labels. The base layer remains an actual application screenshot or a clear montage of actual application screenshots.

### Privacy and Accuracy Rules

- Replace or cover local usernames, absolute private paths, API keys, tokens, public IP addresses, and SSH credentials.
- Use neutral example names such as `Qwen3-8B`, `Local CUDA`, and `chat-main` where text must be obscured.
- Do not alter button positions, status colors, page composition, or runtime values beyond privacy redaction.
- Verify every screenshot against the final committed UI after all guide-related code changes.

## Interactive Tour

Tour definitions move from `GuidePage.tsx` to `src/components/guide/guideTour.ts`. Each step declares:

- Stable step identifier.
- Target tab identifier.
- `data-guide` selector.
- Chinese title and description.
- English title and description.

The tour covers dashboard, model repository, downloads, engine manager, instance manager, configuration, cluster manager, instance routing, performance, monitoring wall, and logs. The guide page itself remains the launch and return point rather than a tour target.

Missing selectors, route changes, and timeout failures are handled gracefully: the tour skips an unavailable step, records the failure in the console for diagnostics, and returns the user to the guide when the tour ends or is closed. Starting the tour must not create, modify, start, or stop user resources.

New `data-guide` markers are limited to stable page-level or primary-action controls. They must not affect layout, accessibility, or existing behavior.

## Automated Validation

A repository script will validate documentation without adding a test framework. It will fail when any of the following is true:

- `GUIDE.md` has a version other than `v2.9.22` at release baseline.
- A local image referenced by `GUIDE.md` or `README.md` does not exist.
- An image path escapes `public/docs/guide/`.
- Required guide sections or screenshot names are missing.
- A table-of-contents anchor has no corresponding heading.
- A tour tab is absent from the application navigation.
- A tour selector is missing from the source tree or duplicated as a primary tour target.
- A required operational page has neither a tour step nor an explicit documented exclusion.

The script will be exposed as `npm run check:guide` and added to the GitHub Actions build workflow before packaging.

## Manual and Visual Validation

1. Run the documentation validator and existing encoding checks.
2. Run TypeScript compilation and the production frontend build.
3. Confirm `dist/docs/guide/` contains every committed image.
4. Launch the Windows Tauri development build.
5. Open the in-app guide with network access disabled and verify every image renders.
6. Check the table of contents, internal anchors, and external links.
7. Run the complete interactive tour and verify each target is visible and correctly framed.
8. Inspect guide layout at desktop and compact window sizes for clipping, overflow, and unreadable captions.
9. Compare every screenshot with the final application page to detect stale labels or composition.
10. Run the full existing Rust and frontend verification suite before commit and CI.

## File Responsibilities

- `GUIDE.md`: canonical bilingual manual.
- `README.md`: concise overview, quick start, and guide entry.
- `public/docs/guide/*.png`: shared GitHub and offline application images.
- `src/components/GuidePage.tsx`: guide rendering, asset URL handling, links, checklist, and tour orchestration.
- `src/components/guide/guideTour.ts`: bilingual tour step data and page coverage.
- Operational page components: stable `data-guide` targets only where missing.
- `scripts/check-guide.cjs`: static documentation, asset, anchor, and tour checks.
- `package.json`: `check:guide` command.
- `.github/workflows/build.yml`: pre-package guide validation.

## Error Handling

- Invalid or unsafe Markdown links remain sanitized and inert.
- A missing image shows a visible fallback boundary and useful alt text in the in-app guide.
- An unavailable tour target does not trap the user on an unrelated page.
- Screenshot capture failures do not permit placeholder images into the commit; the affected page remains incomplete until a real capture is available.
- The guide checker reports exact file paths and missing identifiers so CI failures are actionable.

## Acceptance Criteria

- Repository and in-app guide content accurately describe all current operational pages in `v2.9.22`.
- README, GUIDE, navigation, and interactive tour use consistent page names and workflow order.
- All 12 real screenshots and 3 annotated workflow images are committed, privacy-reviewed, and readable.
- Every image renders both on GitHub-compatible Markdown paths and offline in the packaged application.
- Interactive tour reaches all 11 operational targets without changing user data.
- `npm run check:guide`, encoding checks, TypeScript compilation, frontend build, Rust tests, and existing project checks pass.
- The Windows desktop guide receives a final manual visual review at normal and compact window sizes.
- The working tree contains no temporary capture files, scratch assets, or untracked documentation artifacts at handoff.
