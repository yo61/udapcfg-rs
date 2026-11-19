## Decision: Remove the `aarch64-apple-darwin` leg from the CI build matrix

The `build` matrix goes from six targets to five. The other five targets
keep their native runners, and so does the rest of the 2026-09-13
decision.

## Context: GitHub could not allocate a `macos-latest` runner

On PR #65 (a `jdx/mise-action` bump), `build (aarch64-apple-darwin)`
failed three times in a row, on 2026-11-18 and 2026-11-19. Each attempt
sat queued for 15 minutes before GitHub cancelled it with "The job was not
acquired by Runner of type hosted even after multiple attempts". GitHub's
annotation said: "Due to capacity constraints, jobs targeting macOS arm64
runners may experience longer queue times." None of the attempts ran a
step, and every other leg passed each time.

`timeout-minutes` cannot help here. It limits how long a job runs once it
has a runner, not how long it waits for one. The 15-minute acquisition
limit is GitHub's, and the workflow has no setting for it.

## Alternatives considered:

- **Keep re-running until a runner is free.** No workflow change, but each
  failed attempt costs 15 minutes, and how long the capacity shortage lasts
  is outside the project's control.
- **`continue-on-error` on the leg.** It keeps the coverage whenever a
  runner turns up, but a leg that may fail without consequence is easily
  ignored, including when it fails for a real reason.
- **Build `aarch64-apple-darwin` from `macos-15-intel`.** That would
  produce a binary nothing in CI can run, which is the reason the
  2026-09-13 decision rejected cross-compilation.
- **Drop all three aarch64 legs.** Not needed: the Linux and Windows arm64
  runners were allocated without trouble.

## Reasoning:

Every `#[cfg]` arm in the code (`udap::transport::udp`) is selected by
`target_os` or `unix`, never by `target_arch`. The `x86_64-apple-darwin`
leg therefore still compiles and lints every macOS arm, so the
"Cross-platform code" criterion in `quality/criteria.md` still holds.
`aarch64-apple-darwin` is the dev host's native triple, so it is built,
linted and tested locally whenever anyone works on the project.

## Trade-offs accepted:

- **CI no longer runs the test suite on Apple Silicon.** A defect that
  shows up only on arm64 macOS is caught locally or not at all. The code
  currently has no arch-specific paths, so this is a bet that it stays
  that way.
- **No CI artifact for `aarch64-apple-darwin`.** The workflow uploads
  nothing for that triple. This matters at M6 (OQ-3, release packaging),
  which will have to restore an Apple Silicon build somehow, whether by a
  native runner, a larger paid runner or a universal binary.
- **Revisit when runner capacity recovers.** Restoring the leg is the
  matrix entry it lost.

## Supersedes: `decisions/2026-09-13-native-runners-over-cross-compilation.md`, in part

That decision's six-way matrix becomes five-way. Its reasoning about
native runners, musl and the Windows `rustup` hang stands unchanged.
