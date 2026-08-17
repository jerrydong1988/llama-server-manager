# Repository Workflow

These rules apply to every change in this repository.

## Protected default branch

- Never commit or push directly to `master`.
- Never force-push or delete `master`.
- Start each change from an up-to-date `master` on a scoped `codex/<description>` branch.
- Keep unrelated changes out of the branch and pull request.

## Required pull request flow

1. Run the checks relevant to the change. For release-affecting code, run `npm run check:release`; run `npm run build` when production output may be affected.
2. Push the topic branch and open a pull request targeting `master`.
3. Do not merge until the branch is up to date, all review conversations are resolved, and these required GitHub Actions checks pass:
   - `quality`
   - `build-windows`
   - `build-macos`
   - `build-linux`
   - `build-linux-arm64`
4. Merge through GitHub. Do not bypass or disable the ruleset for routine work.

## Product roadmap governance

- `docs/PRODUCT_ROADMAP.md` is the source of truth for product direction, current phase, phase order, and phase exit gates.
- Read the roadmap and the active phase tracking issue before scoping product work.
- Every product feature pull request must name its phase, tracking issue, workstream or exit criterion, explicit non-goals, and validation evidence.
- Work on the current phase only. Record later-phase ideas in the relevant tracker or a linked issue instead of implementing them opportunistically.
- Keep defects, security fixes, compatibility work, releases, and maintenance unblocked, but explain their roadmap relationship in the pull request.
- Change phase scope, order, status, or exit gates only through a dedicated roadmap pull request with rationale and dependency impact.
- Transition phases only after the current tracker contains exit evidence, required checks pass, and a dedicated roadmap pull request marks the next phase current.
- A Codex Goal is one bounded execution package. It must reference the roadmap and tracker, define proof and a stopping condition, and must not automatically start the next package.

## Releases

- Create release commits through the same pull request flow.
- Create version tags and GitHub Releases only from the merged `master` commit.
- Do not tag or publish while required CI or the upstream compatibility gate is failing.
