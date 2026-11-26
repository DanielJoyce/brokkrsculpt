# Working in this repository

Notes for Claude sessions on HaniSculpt (`DanielJoyce/hanisculpt`). It
began as a fork of `MakerViking/brokkrsculpt`; the maintainer severed the
two on 2026-11-26. The design, the state of each stage and
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
- Merging `feature/studio` into **`main`**, by a pull request with a merge
  commit, once the gate has passed, and publishing to GitHub Pages by
  fast-forwarding the **`pages`** branch to it. Given by the maintainer on
  2026-11-26: *"You have my full permission to merge to main and publish
  on pages too."*

## Run every task to completion

Said by the maintainer on 2026-11-26: when given tasks, run each one to
completion instead of pausing or stopping. In practice:

- A request with several tasks in it is worked through in order, each one
  built, gated, merged and written up, and then the next, in the same
  session, without stopping between them to report or to ask whether to
  go on.
- Do not stop to ask a question that a sensible default answers. Pick the
  default, say which in the pull request, and carry on.
- Report once, at the end, with what was built, what was measured and
  what is not done.
- Stop early only for what the standing permission above does not cover,
  or for a refusal from a tool or the environment that cannot be worked
  round in a permitted way. Say which, and finish everything else first.

What it does not cover, and still wants asking:

- Anything on `main` other than merging `feature/studio` into it as above:
  no commits made on `main` itself, and no changes to its settings.
- Force pushes, history rewrites, deleting branches or files that are not
  this work's own.
- Removing a feature the maintainer has said to keep. (Filament painting
  was one, said on 2026-11-23; on 2026-11-25 the maintainer asked for it
  to be removed: *"Remove the filament painting, we don't need it."*)

## How the work goes in

1. Commit on the session's branch.
2. Open a pull request into `feature/studio`.
3. Merge it with a merge commit (not squash, not rebase).
4. Fast-forward the session branch to `feature/studio` and push it.
5. Open a pull request from `feature/studio` into `main` and merge it with
   a merge commit, then fast-forward `pages` to `main`
   (`git push origin origin/main:refs/heads/pages`), which builds and
   publishes the web version.

One stage of the plan a pull request. Keep `docs/STUDIO-PLAN.md` and the
README current in the same pull request as the code.

## The gate before a merge

- `cargo fmt --all -- --check`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --target wasm32-unknown-unknown`
  (the browser build; `rustup target add wasm32-unknown-unknown` once)
- `cargo test -p hani-core` (the dev profile, where contracts and debug
  assertions are live), and `cargo test --release -p hani-core --lib`
- `cargo test -p hani-gpu`
- `cargo test -p hani-app` (about four and a half minutes)

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

- **The interface is controls, not prose.** Said by the maintainer on
  2026-11-24, of a block that explained its own rows and said where its
  files are kept: *"Remove the extra text telling me where brushes or
  stored or anything like that in the UI. Keep ui clean. If we need this
  info then let's add a help/doc ticket."* No captions that explain a
  control, no paths, no "this is kept in". A label, a placeholder and a
  status line after an action are what a panel says. What a user may
  need to know beyond that goes in the documents, with a ticket.

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
