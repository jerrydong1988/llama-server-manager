# Llama Server Manager Product UI Refactor Plan

## North Star

Rebuild the frontend presentation layer so the app feels like a cohesive desktop product, using the original dashboard mockup as the product target.

The goal is not to polish the current screens one by one. The goal is to establish a product shell, a strict design system, and page templates that make every feature feel like part of the same tool.

## Product Principles

- The app should feel like a local professional desktop management console, not a web admin page.
- Dense operational information is preferred over decorative layout.
- Tables, status rows, compact panels, and inspectors are the primary surfaces.
- High-frequency actions must be visible and aligned.
- Low-frequency or destructive actions should be grouped or demoted.
- Long paths, model names, commands, and logs must never break layout.
- Every page should inherit the same shell, spacing, colors, controls, and status language.

## Preserve vs Rebuild

Preserve by default:

- Tauri/Rust command capabilities.
- Zustand store data model and runtime event wiring.
- Download persistence, resume, pause, cancel, and queue behavior.
- Instance start/stop, engine scan, model scan, cluster, metrics, logs, and config persistence.

Rebuild aggressively:

- `App.tsx` application shell.
- Page-level layout and visual structure.
- Shared UI primitives.
- Dashboard, Instances, Engines, Models, Downloads, Config, Cluster, Performance, and Logs presentation.
- Action columns, detail panels, toolbars, and path rendering.

Modify backend/business logic when needed:

- Add summary APIs if frontend currently derives too much display state.
- Add missing metadata for product-quality tables and inspectors.
- Fix desktop product shell issues such as garbled window title, tray menu labels, and startup visual glitches.

## Target Information Architecture

```text
AppShell
  WindowTitleBar
  Sidebar
  TopContextBar
  MainWorkspace
    PageHeader
    PageToolbar
    PageContent
    InspectorPanel optional
  BottomStatusBar
```

## Target Navigation

Primary:

- Dashboard
- Instances
- Models
- Downloads
- Engines
- Config

Secondary:

- Cluster
- Performance
- Logs
- Guide/About

## Shared UI System

Core shell components:

- `AppShell`
- `Sidebar`
- `TopContextBar`
- `BottomStatusBar`
- `PageFrame`
- `PageHeader`
- `PageToolbar`

Core product components:

- `DataTable`
- `DataTableActionCell`
- `InspectorPanel`
- `DetailField`
- `PathText`
- `ActionGroup`
- `IconButton`
- `SegmentedControl`
- `StatusBadge`
- `MetricStrip`
- `ResourceMeter`
- `EmptyPanel`
- `CommandBar`

## Component Rules

### Paths

All filesystem paths must render through `PathText`.

Required behavior:

- Never overflow parent containers.
- Prefer middle truncation for one-line compact contexts.
- Allow multi-line wrapping in detail views only when explicitly requested.
- Provide full path in `title`.
- Reserve room for copy/open actions when present.

### Actions

All table action cells must use `ActionGroup`.

Required behavior:

- One primary action maximum.
- Secondary actions use icon buttons with stable dimensions.
- Destructive actions are visually separated.
- Reorder/move actions are demoted.
- No uncontrolled `flex-wrap` action piles in table rows.

### Tables

All high-density management pages should use `DataTable`.

Required behavior:

- Fixed header rhythm.
- Stable row height.
- Column definitions define min/max width.
- Long text cells use `min-w-0` and truncation helpers.
- Row hover and selected states are consistent.

### Inspectors

Right-side details must use `InspectorPanel` and `DetailField`.

Required behavior:

- Key/value rows cannot overflow.
- Long values use `PathText` or truncated text.
- Primary item identity appears at top.
- Action buttons are full-width or grouped consistently.

## Implementation Phases

### Phase 1: Product Shell

Scope:

- `src/App.tsx`
- `src/index.css`
- `src/components/ui.tsx`
- `src/components/shell/*`

Deliverables:

- Shell close to the original mockup: left nav, top context bar, main workspace, bottom status bar.
- Global layout controls page padding and max width.
- App-level status chips and bottom metrics are consistent across pages.
- Existing pages still mount inside the new shell.

Exit criteria:

- App boots to Dashboard.
- No horizontal overflow at 1366x768.
- Sidebar, top bar, and bottom bar are visually stable.
- `tsc`, `cargo check`, and Vite build pass.

### Phase 2: Dashboard Product Sample

Scope:

- `src/components/Dashboard/Dashboard.tsx`
- optional new `src/components/Dashboard/*`

Deliverables:

- Dashboard matches mockup structure: resource strip, running/stopped tabs, instance table, compact summaries.
- CPU/GPU/Memory area has no dead empty space.
- Dashboard table uses product table/action primitives.

Exit criteria:

- First viewport feels like the original design target.
- Resource panels have balanced height.
- Instance rows align cleanly.

### Phase 3: Instances Product Table

Scope:

- `src/components/InstanceManager.tsx`
- optional new `src/components/instances/*`

Deliverables:

- Instances page uses table + optional inspector pattern.
- Operation area is no longer crowded.
- Primary and secondary actions are consistently grouped.
- Create/edit modals visually match the design system.

Exit criteria:

- Action column remains aligned with many instances.
- Long model names do not shift layout.
- Running and stopped rows are equally readable.

### Phase 4: Engines and Models

Scope:

- `src/components/EngineManager.tsx`
- `src/components/ModelRepo.tsx`

Deliverables:

- Shared directory + inventory + inspector pattern.
- Engine and model paths use `PathText`.
- No detail panel path overflow.
- Tables/lists share visual rhythm with Dashboard and Instances.

Exit criteria:

- The path overflow issue is eliminated.
- Engines and Models look like sibling pages.

### Phase 5: Downloads

Scope:

- `src/components/DownloadManager.tsx`
- store/backend only if strategy controls need real backend support.

Deliverables:

- Professional transfer manager with queue sections and strategy panel.
- Existing reliable resume behavior preserved.
- UI-only strategy controls are either clearly marked or backed by real commands.

Exit criteria:

- Pause/resume/exit recovery remains valid.
- Download rows are compact and aligned.

### Phase 6: Config, Cluster, Performance, Logs

Scope:

- `src/components/ConfigPage.tsx`
- `src/components/ClusterPage/ClusterPage.tsx`
- `src/components/PerformancePage/*`
- `src/components/LogsViewer.tsx`

Deliverables:

- Config: parameter directory + editor + inspector.
- Cluster: worker table + inspector.
- Performance: compact monitoring console.
- Logs: high-density console layout.

Exit criteria:

- Pages no longer look like independent prototypes.
- Empty states are compact and useful.

### Phase 7: Tauri Product Shell Cleanup

Scope:

- `src-tauri/src/main.rs`
- `src-tauri/tauri.conf.json`
- related config if needed.

Deliverables:

- Fix garbled window title, tray menu labels, and comments.
- Align window sizing and background color with product shell.
- Evaluate custom title bar only after frontend shell is stable.

Exit criteria:

- Native shell details no longer break product polish.

## Verification Matrix

Run after every implementation phase:

```powershell
node .\node_modules\typescript\bin\tsc --noEmit
cargo check --manifest-path src-tauri\Cargo.toml
npm run build -- --mode development
rg -n "\?\?\?|鈥|鈹|攣|銆|鍚|鎬|鐩|寮|瀹|浣|妯|杩|绯|鐘|鎼|涓|鍐|閰|宸|鏂|鍒|鏌|褰|鑼|淇|閿|璀|灏|骞|绔|妫|彛|閲|鍋" src src-tauri/src -g "*.ts" -g "*.tsx" -g "*.rs" -g "*.css"
```

Browser checks:

- 1366x768
- 1600x900
- 1920x1080
- 390x844

Required browser assertions:

- `document.documentElement.scrollWidth <= document.documentElement.clientWidth + 2`
- no obvious overlapping text
- no broken path overflow
- no uncontrolled table action wrapping
- no huge empty primary panels

## First Implementation Wave

Parallel work split:

1. Shell and design system
   - Owns `src/App.tsx`, `src/index.css`, `src/components/ui.tsx`, `src/components/shell/*`.
   - Builds the product frame and new primitives.

2. Dashboard sample
   - Owns `src/components/Dashboard/Dashboard.tsx` and optional Dashboard child components.
   - Rebuilds Dashboard around the new primitives.

3. Instances sample
   - Owns `src/components/InstanceManager.tsx` and optional `src/components/instances/*`.
   - Rebuilds instance table/actions using the new primitives.

4. Visual QA and product consistency
   - Read-only unless asked to patch small isolated issues.
   - Audits screenshots, overflow risks, class drift, and product mismatch.

Main integration owner:

- The main Codex thread integrates, resolves conflicts, runs verification, and decides whether the wave meets the product target.

