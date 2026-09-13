# Quality Criteria — udapcfg-rs

Testable checks evaluated before marking a task complete. `blocking`
criteria must pass; `warning` criteria are surfaced but do not stop the
work. See the global CLAUDE.md "Quality Gate" section for how these are
promoted, pruned, and maintained.

This project is a **faithful port**, so most criteria here are about
matching go-udap rather than about being good in the abstract. Where the
Go has a wart we carry it forward; "this is better" is not a reason to
diverge, and a divergence that is genuinely wanted goes in the spec's
accepted-deltas table first.

---

## Category: Fidelity to go-udap

## Criteria:

    - Any change touching user-visible output is compared against
      go-udap on the same input, byte for byte, modulo the program-name
      exception the spec lists. Reading both implementations and
      reasoning about them does not count -- run both binaries.
    - The comparison uses a released go-udap binary or a build from a
      recorded commit, and the version is written down in the PR. "I
      compared against go-udap" without a version is not reproducible.
    - A deliberate divergence is recorded in the spec's accepted-deltas
      table in the same change that introduces it, never afterwards.
    - Exit codes are asserted, not assumed: 0 success, 2 for failures,
      and 1 only for an unusable `--bind-interface` name.

## Severity: blocking

## Source: spec "Fidelity contract"; issue #13

## Last triggered: 2026-09-13 (#13 -- `interfaces` row order diverged on
Linux and had never been diffed against go-udap on a host with enough
interfaces to show it)

---

## Category: Output determinism

## Criteria:

    - The same command against the same environment produces identical
      bytes on repeated runs. Any output built by iterating a collection
      has a defined order; `HashMap`/`HashSet` iteration never reaches
      stdout, directly or through a dependency's public API.
    - A dependency that returns a `Vec` is not assumed to have a stable
      order. Where order is user-visible, either the crate documents the
      guarantee or we impose one.
    - Commands whose output is a list are run at least twice on a host
      with enough entries to expose disorder before the behaviour is
      called verified. Two interfaces cannot distinguish "ordered" from
      "coincidence".

## Severity: blocking

## Source: issue #13 (netdev's Linux netlink backend collects through a
`HashMap`; Rust re-seeds `RandomState` per process, so `udapcfg
interfaces` printed a different order every run)

## Last triggered: 2026-09-13 (#13)

---

## Category: Claims match what was actually verified

## Criteria:

    - "Compiled", "linted", "run" and "verified against hardware" are
      four different claims. A doc, commit message or PR body asserts
      only the one that happened.
    - Before writing that a code path is exercised, grep for its call
      sites and confirm a test reaches it. A test that fails earlier in
      the call chain does not exercise what it is named after.
    - A checklist item is marked closed only when its own stated
      acceptance condition was observed, not when something adjacent
      passed.

## Severity: blocking

## Source: PR #11 review (the matrix was described as exercising the
Windows `#[cfg]` arm; nothing calls `bind_on_interface`, and the one
`--bind-interface` test fails at CLI-side name lookup first)

## Last triggered: 2026-09-13 (PR #11)

---

## Category: Cross-platform code

## Criteria:

    - Every `#[cfg]` arm compiles and is linted on a runner of that
      platform. An arm no CI job builds is an arm that will break.
    - A platform-specific socket option is verified to do what its name
      suggests on that platform, not assumed equivalent to the option it
      replaces. `IP_BOUND_IF` and `SO_BINDTOIFINDEX` differ: the first
      pins egress only, the second filters ingress too.
    - A platform that cannot support a feature fails with an explicit
      "not supported" error, never a silent no-op.
    - Test counts are read from the runner's output, not inferred from a
      green tick. `--no-tests=pass` makes an empty run look like success.

## Severity: blocking

## Source: spec accepted-deltas (Linux `SO_BINDTOIFINDEX`); M3 task 6
steps 5-7; decision 2026-09-13-native-runners-over-cross-compilation

## Last triggered: 2026-09-13 (until PR #11 merged, every CI job was
`runs-on: ubuntu-latest` and the Windows arm of `bind_to_interface` was
compiled by nothing. The six-target matrix now compiles and lints it,
and reports 116 tests run / 116 passed / 0 skipped on both Windows
targets. The arm is still never *invoked* -- see "Claims match what was
actually verified" -- so M3 task 6 step 7 remains open)

---

## Category: Test quality

## Criteria:

    - Tests assert behaviour -- output, exit code, wire bytes -- not
      internal structure a refactor would change.
    - Every error branch the code handles has a test that triggers it.
    - A test that reads the real machine asserts invariants, not a fixed
      list, and states what it does when the machine offers nothing to
      test.
    - New parser, encoder or decoder work gets a proptest round-trip.
    - Before claiming a test covers a branch, break the branch and watch
      the test fail.

## Severity: blocking

## Source: global CLAUDE.md Testing standards; spec success criteria 1-3

## Last triggered: never

---

## Category: Wire protocol

## Criteria:

    - UDAP fields are big-endian; TLV encode/decode round-trips.
    - New wire behaviour is checked against a go-udap capture fixture or
      an explicit citation, never derived by guessing at hardware.
    - Discovery sends always target `255.255.255.255`. The spec records a
      go-udap spike where a directed broadcast meant pre-DHCP devices
      never replied -- do not re-derive this on hardware.
    - Device-supplied bytes are preserved byte-exact; nothing that came
      off the wire is round-tripped through `String`.

## Severity: blocking

## Source: spec "Fidelity contract"; issue #3

## Last triggered: 2026-09-09 (#3 -- non-UTF-8 device values were lossy)

---

## Category: CI and toolchain changes

## Criteria:

    - A workflow change is verified by a real run on every platform it
      touches, before the claim that it works. `actionlint` and `zizmor`
      passing say nothing about runtime.
    - A step added "just in case" is justified or removed. An
      unconditional `rustup target add` was neither, and deadlocked both
      Windows runners.
    - Every job has an explicit `timeout-minutes`. GitHub's default is
      six hours.
    - Actions are SHA-pinned with a version comment, and the pinned
      version is looked up at the time of writing, never recalled.
    - A tool-installation mechanism is confirmed to have the effect
      intended, not the effect its name suggests.

## Severity: blocking

## Source: PR #11 (three red runs: a hung `rustup target add`;
`install_args` narrowing only the eager install while mise's shims
installed the rest anyway; `cargo-mutants` failing to build on ARM64
Windows in a leg that never invokes it)

## Last triggered: 2026-09-13 (PR #11)

---

## Category: Supply chain

## Criteria:

    - `cargo deny check` passes: advisories, licences, bans.
    - Dependency versions are pinned exactly in `mise.toml` and
      `Cargo.toml`; no floating ranges.
    - A new dependency is justified in the PR against the spec's
      dependency table, including why the standard library or an
      existing dependency will not do.
    - Default features are off unless a specific feature is needed, and
      the reason is recorded next to it.

## Severity: warning

## Source: global CLAUDE.md "Justify new dependencies"; spec dependency
table (the `netdev` and `futures-util` entries model the expected
rationale)

## Last triggered: never

---

## Category: Zero warnings

## Criteria:

    - `cargo clippy --all-targets --all-features` is clean under
      `-D warnings` on every target in the matrix.
    - An unavoidable lint is suppressed inline with a justification
      comment. `allow_attributes` is denied, so a bare `#[allow]` will
      not compile.
    - `cargo fmt --all --check` is clean.

## Severity: blocking

## Source: global CLAUDE.md "Zero warnings policy"; `Cargo.toml`
`[workspace.lints.clippy]`

## Last triggered: never
