# Maintained braces 3.0.3 patch

This private local package retains the upstream MIT license and the braces 3 API.
It is installed as `braces` using the root npm override; it is not an official
upstream release. No installation scripts or runtime patch application are used.

Source: the official npm `braces@3.0.3` tarball, verified with SHA-512:
`yQbXgO/OSZVD2IsiLlro+7Hf6Q18EJrKSEsdoMzKePKXct3gvD8oLcOQdIzGupr5Fj+EDe8gO/lxc1BzfMpxvA==`.
Only package metadata, this README, `lib/complexity.js`, and the parser/three
walker modules differ from that distribution.

## Security scope

[GHSA-vfj7-8cjw-p6xm](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm)
and [upstream issue 70](https://github.com/micromatch/braces/issues/70) describe
unbounded recursive AST processing. No patched npm release exists as of
2026-10-08. Tailwind 3 is retained to preserve Chrome 106 and macOS 13 support;
Tailwind 4 raises the minimum browser requirements and changes CSS semantics.

The parser bounds brace **and parenthesis** nesting at 128 containers. Each
compile/expand/stringify entry, including direct `lib/` imports, first checks
the AST iteratively: at most 128 child edges deep and 65,536 node visits,
including repeated visits to shared children. Terminal nodes count as an edge,
so 127 parsed containers leave room for their text nodes. Child cycles and
array-valued node values are rejected. Ordinary parent/prev backlinks are allowed.
Expansion derives queues from child traversal, never from external parent
metadata. This also prevents foreign parent queues from bypassing the guard.

Violations throw `SyntaxError` with `code: ERR_BRACES_COMPLEXITY`; options
cannot disable these bounds. Existing maxLength/rangeLimit behavior is retained.
This repair does not establish bounds on Cartesian expansion size, generated
regex execution, or arbitrary JavaScript getters/proxies/callbacks.

Regression coverage lives in `scripts/test-frontend-dependency-security.cjs`
and is part of the release gate. It verifies the actual packages resolved by
Tailwind's consumers, malicious structures, and ordinary glob/CSS behavior.
Remove this override when a compatible official release closes the same paths
and passes those checks. npm advisory services cannot audit local package
contents: the committed source, license and executed regressions are therefore
required evidence alongside the unchanged npm audit and dependency-review gates.
