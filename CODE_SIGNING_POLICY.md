# Code Signing Policy

Free code signing provided by SignPath.io, certificate by SignPath Foundation.

## Scope

This policy applies to Windows installers published by the Llama Server Manager project. Signed artifacts must be built from the public source repository by the project's GitHub Actions workflow. Locally supplied or externally rebuilt binaries are not eligible for project signing.

## Roles

- Committer and reviewer: [jerrydong1988](https://github.com/jerrydong1988)
- Signing approver: [jerrydong1988](https://github.com/jerrydong1988)

The project is currently maintained by one individual. GitHub and SignPath accounts used for these roles must have multi-factor authentication enabled.

## Release Process

1. Release source is identified by a `v*` Git tag in the public repository.
2. GitHub Actions installs locked dependencies and runs the release checks, frontend build, Rust tests, and warning-free Clippy checks.
3. GitHub-hosted runners build the Windows MSI and NSIS installers from that tagged source.
4. The workflow uploads the unsigned installers as a GitHub Actions artifact and submits that artifact to SignPath.
5. The signing approver reviews and approves the request in SignPath.
6. Only the signed artifact returned by SignPath is attached to the GitHub Release when SignPath is configured.

When SignPath is not configured, the workflow may publish installers with an `-unsigned` filename suffix. Those artifacts are not represented as SignPath-signed releases.

## Security Reports

Report suspected signing misuse, compromised releases, or security issues through the repository's [GitHub Issues](https://github.com/jerrydong1988/llama-server-manager/issues). Do not include credentials or sensitive machine information in a public report.

See the project [Privacy Policy](PRIVACY.md) for network and data-handling behavior.
