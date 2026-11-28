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

- Opening pull requests into **`main`** and merging them, without asking
  first, once the gate below has passed. Given by the maintainer on
  2026-11-26: *"You have my full permission to merge to main and publish
  on pages too."*, and then *"Let's switch to main trunk development.
  Studio branch can be retired going forward."* Until then pull requests
  went into `feature/studio`, which is retired: nothing more goes into it.
- Publishing to GitHub Pages by fast-forwarding the **`pages`** branch to
  `main`.
- Carrying on to the next stage of the plan without waiting for a reply.

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

- Anything on `main` other than merging pull requests into it as above:
  no commits pushed to `main` itself, and no changes to its settings.
- Force pushes, history rewrites, deleting branches or files that are not
  this work's own.
- Removing a feature the maintainer has said to keep. (Filament painting
  was one, said on 2026-11-23; on 2026-11-25 the maintainer asked for it
  to be removed: *"Remove the filament painting, we don't need it."*)

## How the work goes in

1. Commit on the session's branch.
2. Open a pull request into `main`.
3. Merge it with a merge commit (not squash, not rebase).
4. Fast-forward the session branch to `main` and push it.
5. Fast-forward `pages` to `main`
   (`git push origin origin/main:refs/heads/pages`), which builds and
   publishes the web version.

A merge into `main` that changes code also publishes the rolling `beta`
release (`.github/workflows/release.yml`), which every installed copy is
offered as an update. One that changes only documents does not. So a
pull request into `main` is one finished piece of work, gated, never a
part of one.

One stage of the plan a pull request. Keep `docs/STUDIO-PLAN.md` and the
README current in the same pull request as the code.

## The gate before a merge

- `cargo fmt --all -- --check`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --target wasm32-unknown-unknown`
  (the browser build; `rustup target add wasm32-unknown-unknown` once)
- `cargo test -p hani-core` (the dev profile, where contracts and debug
  assertions are live)
- `cargo test -p hani-gpu`
- `cargo test -p hani-app`

Two `update::apply` tests in the application fail when the tests are run
as root, which a cloud container does. They are not this work's.

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

- **No extraneous text in the UI.** Said by the maintainer on 2026-11-28:
  explanations of what a field is or does go either in a tooltip or in
  Help. On screen, a control has its label, its value and its unit, and a
  panel has a heading and a status line after an action; nothing more. A
  sentence that explains, a hint under a field, a note of where something
  is kept: a tooltip (`crate::ui::tip`, or a number's `.tip`), or a page of
  `docs/help/`.

- **Every setting kept in a file is editable in the UI.** Said by the
  maintainer on 2026-11-28. Whatever the application writes to a settings
  file under the configuration or data directory has a control that
  changes it: in Preferences, or beside the tool it belongs to, or both.
  A new setting that is kept comes with its control in the same pull
  request, and one that is not kept says so in the pull request.

- No em-dashes in prose, comments, documents or commit messages.
- An `SPDX-License-Identifier: AGPL-3.0-only` header on every new source
  file.
- New features go in modules of their own. Decision D6 of the plan
  ("add, do not modify") also kept upstream's files from being
  restructured, for the sake of merging from upstream. The fork was
  severed on 2026-11-26, and the maintainer retired that half on
  2026-11-27: *"retire d6 and break up app."* The code is ours:
  restructure it where that makes it better, in a pull request that
  moves and does not change, and make a change for speed in whichever
  file it belongs.
- A number is either measured, and says with what, or is arithmetic, and
  says so. Do not write "measured" for something worked out.
- Commit messages are long and say what was tried and dropped, what a
  test or a screen found, and what is not done.
- Say what is not done. Each stage in the plan ends with it.
