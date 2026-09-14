## Decision: Build every target on its own native GitHub-hosted runner, and ship Linux as static musl

CI gains a six-way `build` matrix — `{linux, darwin, windows} x {x86_64,
aarch64}` — with each target compiled, linted and tested on a runner of
that same architecture and OS. No cross-compilation tooling is adopted.
Linux targets are `*-unknown-linux-musl`, not `-gnu`.

This resolves **OQ-4** in `docs/specs/2026-09-08-rust-port-spec.md`.

## Context: OQ-4 blocked M6, and hardware verification needed a Linux binary today

OQ-4 asks which of `cross` (Docker), `cargo-zigbuild`, or per-target
rustup toolchains to use for Windows and Linux builds from macOS, and
calls cross-compilation "the largest practical regression versus Go".
Go cross-compiles with two environment variables; Rust does not.

The immediate trigger was smaller. M3 task 6 (real-hardware
verification) needs `udapcfg` on a Linux host — `nas1`, TrueNAS Scale,
kernel 6.18.42, glibc 2.41 — to answer OQ-2 as an unprivileged user.
The first attempt at that was an ad-hoc `docker run --platform
linux/amd64` cross-build on the dev machine, producing an unreproducible
binary from an unrecorded toolchain. That is the wrong artifact to base
a fidelity claim on.

Two facts reframed the question. GitHub-hosted runners now cover every
target this project ships — `ubuntu-24.04-arm`, `macos-15-intel`,
`windows-11-arm` all exist and are free and unlimited on public
repositories. And the checklist records that only `aarch64-apple-darwin`
is installed on the dev host, "so even a cross `cargo check` cannot
run": the Windows `#[cfg]` arm in `udap::transport::udp` has never been
compiled by anything, anywhere.

## Alternatives considered:

- **`cargo-zigbuild`** — the spec's own recommended first try, on the
  strength of needing no Docker. Builds many targets from one Linux
  runner and can pin a target glibc. Passed over because it produces
  binaries it cannot execute: nothing on a Linux runner can run the
  Windows or macOS output, so the untested `#[cfg]` arms stay untested.
  It also reaches `x86_64-pc-windows-msvc` only awkwardly, preferring
  the `-gnu` ABI, and macOS targets need an SDK whose redistribution is
  legally murky.
- **`cross`** — Docker-based, well-established for Linux targets. Covers
  neither Darwin nor MSVC, so it would have to be combined with native
  macOS and Windows runners anyway. Adds a container runtime to CI for a
  subset of what native runners already do.
- **Per-target rustup toolchains on one runner** — the third option OQ-4
  names. Same execution problem as zigbuild, plus a linker to source per
  target.
- **glibc rather than musl for Linux** — simpler, no `musl-tools`, no
  `rustup target add`. Rejected on fidelity: go-udap builds
  `CGO_ENABLED=0`, so every Linux binary it ships is static. A
  glibc-dynamic Rust build would be *less* portable than the tool being
  ported, and would silently bind each artifact to the runner image's
  glibc version.

## Reasoning:

Native runners do not answer OQ-4 so much as dissolve it. The question
presumes one host must produce every target; GitHub supplies a host per
target at no cost, so the hard part stops existing. The "largest
practical regression versus Go" turns out to be a regression only for
local builds, which nothing in the project requires.

The decisive advantage is coverage of the platform-specific arms. The
Windows body of `bind_to_interface` in `udap::transport::udp` is now
compiled and linted for the first time — previously nothing anywhere
built it, so a syntax error or a renamed error variant in that arm would
have reached a release unnoticed.

Measured on the first green run: 116 tests run, 116 passed, 0 skipped on
both `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` — the same
count as the dev host, so the suite genuinely executed rather than
`--no-tests=pass` masking an empty run.

Be precise about what that is not. Compiling an arm is not running it.
No test calls `UdpTransport::bind_on_interface`; the only test naming
`--bind-interface`
(`udap-cli/tests/e2e_discover.rs::unknown_bind_interface_is_a_usage_error`)
passes an unresolvable name and fails CLI-side lookup before any
transport is built. **M3 task 6 step 7 therefore stays open**: nobody
has yet observed Windows surfacing the unsupported-interface message at
runtime. A regression that returned the wrong error variant would still
pass this matrix.

Closing step 7 needs a test that actually invokes `bind_on_interface`
and asserts the arm's behaviour per platform. That is deliberately not
in this change, because on Linux such a test asserts `SO_BINDTOIFINDEX`
succeeds for an unprivileged user — which is the still-open **OQ-2**,
and would make CI the thing that answers it. Worth doing; worth
deciding on its own terms.

musl follows from the fidelity contract rather than from preference.
The port's standing rule is same wire bytes, same CLI surface, same exit
codes; shipping a dynamically-linked Linux binary where go-udap ships a
static one is a user-visible divergence in where the tool will run. It
also happens to suit the verification target, which mounts `/tmp` and its
boot-pool `/home` and `/mnt` datasets `noexec` — one fewer variable when
the binary depends on no system libraries at all. (Its `$HOME` is a
separate dataset that does permit execution, which the first pass at this
got wrong by reading the parent mount rather than the target path.)

## Trade-offs accepted:

- **Six jobs where there was one.** Wall-clock CI grows, though less
  than feared: the settled matrix runs 91-209 s per leg, against roughly
  60 s for the single job it replaces, and the legs run concurrently.

  Getting there needed `MISE_DISABLE_TOOLS`, not `install_args`.
  `install_args` narrows only the *eager* install; mise's shims then
  install any still-missing tool the first time cargo runs, so the work
  relocates into the Clippy step instead of disappearing. That is not
  merely slow — cargo-mutants pulls a dependency with no ARM64 Windows
  support, so it failed the `aarch64-pc-windows-msvc` leg outright while
  building a tool that leg never invokes.
- **Newer runner labels carry less track record.** `windows-11-arm` and
  `macos-15-intel` are less battle-tested than `ubuntu-latest`. If
  `windows-11-arm` proves flaky the matrix entry can be dropped without
  touching the design.
- **mise on Windows works; `rustup` on Windows does not.** The first run
  of this matrix settled the open worry and found a different one.
  `jdx/mise-action` completed on both Windows runners, so the `cargo:`
  backend is fine there. What hung was `rustup target add <host triple>`
  — no output, killed at 30 minutes, on `windows-latest` and
  `windows-11-arm` alike, while the same step was instant on Linux and
  macOS. The step is now restricted to the musl legs, which are the only
  ones needing a target the host toolchain lacks. Do not reintroduce an
  unconditional `rustup target add`.
- **No local cross-building.** A developer on macOS still cannot produce
  a Linux binary without reaching for Docker or zig themselves. CI is
  the only supported path to a foreign-target artifact, deliberately.
- **`check` no longer tests the gnu target.** Clippy and the test suite
  moved into the matrix, where Linux means musl. The gnu target is now
  built by nobody. Accepted because musl is what ships; revisit if a
  libc-sensitive defect ever appears.
- **OQ-3 is untouched.** This produces workflow artifacts, not releases.
  Archives, SBOMs, man pages, completions and the Homebrew cask still
  need an answer at M6, and cargo-dist may still be that answer — it can
  consume this matrix rather than replace it.

## Supersedes: nothing. Amends OQ-4 in `docs/specs/2026-09-08-rust-port-spec.md`, which remains the place the resolution is recorded for the spec's own readers.
