# Working in this repository

Notes for Claude sessions on this fork (`DanielJoyce/brokkrsculpt`, a fork
of `MakerViking/brokkrsculpt`). The design, the state of each stage and
what was measured are in [`docs/STUDIO-PLAN.md`](docs/STUDIO-PLAN.md):
read its "Where it stands" table first.

## Standing permission from the maintainer

Given by the maintainer (DanielJoyce) in conversation, first on 2026-11-23
and again on 2026-11-24, in these words: *"You have full remit to merge
PRs and continue."* Earlier: *"You have my permission to drive this to
completion. Create and merge prs yourself. You are the primary owner of
this code base right now."*

What that covers, as it has been used:

- Opening pull requests into **`feature/studio`** and merging them, without
  asking first, once the gate below has passed.
- Carrying on to the next stage of the plan without waiting for a reply.

What it does not cover, and still wants asking:

- Anything on `main`, or on the upstream repository.
- Force pushes, history rewrites, deleting branches or files that are not
  this work's own.
- Removing a feature the maintainer has said to keep. Filament painting
  and its recipes stay (said on 2026-11-23).

## How the work goes in

1. Commit on the session's branch.
2. Open a pull request into `feature/studio`.
3. Merge it with a merge commit (not squash, not rebase).
4. Fast-forward the session branch to `feature/studio` and push it.

One stage of the plan a pull request. Keep `docs/STUDIO-PLAN.md` and the
README current in the same pull request as the code.

## The gate before a merge

- `cargo fmt --all -- --check`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`
- `cargo test -p brokkr-core` (the dev profile, where contracts and debug
  assertions are live), and `cargo test --release -p brokkr-core --lib`
- `cargo test -p brokkr-gpu`
- `cargo test -p brokkr-app` (about four and a half minutes)

Two `update::apply` tests in the application fail when the tests are run
as root, which a cloud container does. They are not this work's.

The release build of the core's tests has now and then spent ten minutes
or more in one LLVM pass. Give release runs a time limit.

## How it is tested

Decided by the maintainer on 2026-11-23: **no mutation testing.**

- Contracts, with the `contracts` crate (`debug_requires`,
  `debug_ensures`), on what a function needs and promises.
- Property tests, with `proptest`, where a rule holds for any input.
  Regression seeds under `proptest-regressions/` are committed.
- Integration tests that drive the application by its messages and
  pointer events.
- Unit tests on worked fixtures where they say something.

Then look at it on a screen. Several faults in the plan were found only
that way. `docs/DRIVING-THE-APP.md` says how; in a container, Xvfb and
`xdotool` move the pointer and the wheel but pass no keys.

## Performance work

Tracked in issue 62 of this fork and written up in
[`docs/PERFORMANCE.md`](docs/PERFORMANCE.md): the instruments, the machine
the numbers are from, and a before and an after for each change. A change
made for speed quotes both, from `what_a_stroke_costs` or a bench, and says
when the build machine could not measure it (it has no graphics card).

## House style

- No em-dashes in prose, comments, documents or commit messages.
- An `SPDX-License-Identifier: AGPL-3.0-only` header on every new source
  file.
- Add, do not modify (decision D6 of the plan): new behaviour goes in a
  module of its own, with the fewest lines that will do in upstream's
  files, and a comment at each of those lines saying where to look.
  Set aside for performance work by the maintainer on 2026-11-24 (*"The
  code is ours, so do what needs to be done"*): a change that makes
  upstream's code faster is made in upstream's file. New features still
  go in modules of their own.
- A number is either measured, and says with what, or is arithmetic, and
  says so. Do not write "measured" for something worked out.
- Commit messages are long and say what was tried and dropped, what a
  test or a screen found, and what is not done.
- Say what is not done. Each stage in the plan ends with it.
