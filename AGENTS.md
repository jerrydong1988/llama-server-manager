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

## Releases

- Create release commits through the same pull request flow.
- Create version tags and GitHub Releases only from the merged `master` commit.
- Do not tag or publish while required CI or the upstream compatibility gate is failing.
