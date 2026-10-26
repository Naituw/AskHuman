# Build cache and incremental install investigation (2026-08)

## Outcome

The repository had reached roughly 15–20 GiB while a normal warm `install.sh` took about
205 seconds. The two symptoms had the same underlying amplifier: every install rebuilt `dist/`,
which invalidated Tauri's embedded frontend resources and forced the large Rust binary crate
through an almost-cold release rebuild.

The implemented development path now:

- fingerprints frontend inputs and preserves `dist/` when they are unchanged;
- builds local installs with the `local-install` profile (`opt-level=0`, 64 codegen units);
- keeps the production `release` profile unchanged (`opt-level="z"`, 16 codegen units);
- skips copy/sign when both the built source binary and installed binary match saved hashes;
- keeps one Swift/Clang module cache in `target/swift-module-cache`, shared by every profile (until
  2026-10 each build kept its own inside Cargo's `OUT_DIR`; see the 2026-10 follow-up);
- uses Cargo-coordinated per-profile budgets instead of deleting incremental directories directly;
- uses line-table debug information by default, with `full-debug` available explicitly.

## Before: measured state

The main worktree was the only live project worktree with build output. Its target breakdown was:

| Directory | Size | Main contents |
| --- | ---: | --- |
| `target/debug` | 11 GiB | tests, Clippy, dev incremental state |
| `target/debug/deps` | 9.5 GiB | dependency artifacts and Rust codegen objects |
| `target/release` | 2.3 GiB | local install/release dependencies and incremental state |
| Entire repository | about 15 GiB | almost entirely `src-tauri/target` |

`debug/deps` contained 15,395 `AskHuman-*.rcgu.o` files across seven recent build hashes,
with a logical size of about 10.7 GiB. The old 14-day `cargo-sweep` window allowed active
development to create many generations before any became eligible. The custom incremental
pruner deleted raw directories without Cargo's target lock; many objects were hard-linked from
`deps`, so removing only the incremental path often did not release the blocks.

## Compile benchmarks

Environment: Apple Silicon macOS, Rust/Cargo 1.94.1, Node 22.14, pnpm 10.13.1. Cold targets reused
already-downloaded crates but no compiled artifacts.

### Original path

| Scenario | Wall time |
| --- | ---: |
| Frontend dependency check | 0.423 s |
| Frontend production build | 4.38 s |
| Production release, cold target | 215.39 s |
| Cargo no-op without rewriting `dist` | 0.88–1.29 s |
| One real small Rust edit, without rebuilding frontend | 30.34 s |
| Unchanged frontend rebuilt first, then Cargo | 200.67 s |
| Estimated old warm install compile path | about 205.5 s plus copy/sign/cleanup |

A cold release target was 1.9 GiB. One small Rust edit grew it to 2.2 GiB; release incremental
state grew from 480 MiB to 806 MiB, showing why repeated forced rebuilds were both slow and large.

### Profile experiment

Each incremental measurement changed the same leaf help string after a cold build.

| Profile override | Cold build | Small edit | Incremental growth | Binary |
| --- | ---: | ---: | ---: | ---: |
| `z`, 16 CGU (original) | 215.39 s | 30.34 s | about 326 MiB | about 14 MiB |
| `z`, 64 CGU | 210.79 s | 20.70 s | about 357 MiB | 14.3 MiB |
| opt 2, 64 CGU | 226.26 s | 32.39 s | about 332 MiB | 20.5 MiB |
| opt 1, 64 CGU | 200.56 s | 22.86 s | about 359 MiB | 20.4 MiB |
| opt 1, 128 CGU | 208.03 s | 26.96 s | about 354 MiB | 20.5 MiB |
| opt 0, 64 CGU | 122.08 s | 11.26 s | about 333 MiB | 38.1 MiB |

For opt 0 / 64 CGU, rebuilding only the local package after retaining dependencies took 36.53
seconds. Ten `--version` launches took 1.57 seconds versus 0.73 seconds for the production
binary, an approximately 84 ms per-process development penalty. This trade-off applies only to
locally installed development binaries; release publishing remains fully optimized.

### Implemented end-to-end path

| Scenario | Wall time |
| --- | ---: |
| Stable no-change `install.sh` | 2.26 s |
| One-line Rust edit, full install | 11.62 s / 12.77 s (forward/revert) |
| One-line frontend edit, full install | 19.22 s / 16.51 s (forward/revert) |

These numbers include pnpm's dependency check, frontend fingerprint/build as applicable, Cargo,
copy, formal local signing, and cache checks.

## Cache policy

After every successful copy, the installer checks actual profile directory sizes:

| Profile | Main checkout | Linked worktree (since 2026-10) |
| --- | ---: | ---: |
| `local-install` | 4 GiB | 3 GiB |
| `dev` / `test` (`target/debug`) | 6 GiB | 5 GiB |
| `full-debug` | 6 GiB | 5 GiB |
| production `release` | 4 GiB | 3 GiB |

When a profile exceeds its budget, `cargo clean -p humaninloop --profile <name>` removes only this
workspace package first, retaining third-party dependencies. If the remaining dependencies alone
still exceed the budget, the installer cleans that whole profile. Cargo owns the target lock for
both operations. Optional `cargo-sweep --time 7` separately retires unused dependency hashes;
budget enforcement does not depend on cargo-sweep being installed.

The first real install after this change reclaimed 5.64 GiB through cargo-sweep and reduced the
debug profile from 7,534 MiB to 3,135 MiB through package-only cleanup. After rebuilding the new
debug/test profile and running the full test suite, target remained around 11 GiB across active
debug, local-install, and legacy release caches. A one-time package-only cleanup of the superseded
release install artifacts then reclaimed another 1.9 GiB while retaining release dependencies,
leaving `target` at 8.9 GiB and the whole repository at about 10 GiB.

## 2026-10 follow-up: dependency features, shared Swift cache, worktree budgets

Trigger: `target/` again reached many GiB per checkout and builds felt slow for a small project.
Everything below was measured on 2026-10-25 (Apple Silicon, rustc 1.94.1) in single runs; wall time
moves by about ±25% with machine load, so user CPU is the steadier number.

**Feature unification kept ~300 AppKit bindings alive.** The `objc2-*` framework crates have one
feature per header and enable all of them by default (about 300 for `objc2-app-kit`). Cargo unifies
features across the graph, so one dependency that leaves the defaults on undoes everyone else's
trimming, and `drag`'s published manifest did exactly that. `src-tauri/Cargo.toml` now turns the
defaults off and lists the bindings it uses, and `drag` is vendored in `src-tauri/vendor/drag`
(`[patch.crates-io]`) with the defaults off plus the two bindings its sources use. Unit features
afterwards: app-kit 49, foundation 49, core-foundation 24, web-kit 29, core-graphics 4. The unit count
barely moves (375 → 369); what shrinks is the generated binding code each objc2 unit compiles. A cold
`local-install` build went from 205.6 s to 143.5 s wall and from 418 s to 378 s user CPU on a quiet
machine, and `target/local-install` from 2.24 to 1.95 GiB. (A later cold run under load took 182 s
wall, 392 s user CPU.)

**One Swift module cache instead of one per build.** `build.rs` kept the Clang/Swift module cache in
each build script's `OUT_DIR`: about 243 MiB each, two under `debug` and one under `local-install` per
checkout, each rebuilt cold (about 25 s). It now lives once in `target/swift-module-cache`, shared by
every profile. The first real install after the change removed three legacy copies and `debug`
shrank by 487 MB. The cache embeds its absolute path, so it cannot be moved or cloned with a `target/`;
it is capped at 768 MiB and rebuilt when it outgrows that.

**`RUSTC_BOOTSTRAP` silently invalidates the tree.** The build scripts of `proc-macro2`, `anyhow` and
`thiserror` emit `rerun-if-env-changed=RUSTC_BOOTSTRAP`. Exporting it in a shell (it is needed for
`-Z unstable-options --unit-graph` diagnostics) dirties about 116 units — the tauri/wry stack and this
crate — and unsetting it does so again. The mechanism is reproducible and a plausible cause of an
unattributed cold rebuild on 2026-10-23 (a `cargo sweep --time 7` dry run showed the cleanup was not
responsible), but no log existed to prove it; that gap is why `install.sh` now unsets the variable
for its own build, warns, and logs cache-affecting events.

**Worktrees multiply all of the above.** Each linked worktree has its own `target/`. Their budgets are
now 3 / 5 / 5 / 3 GiB (main checkout unchanged at 4 / 6 / 6 / 4), and `install.sh` records budget
cleanups, sweeps, compiler/SDK changes and the `RUSTC_BOOTSTRAP` guard in
`target/askhuman-cache.log` (newest 200 lines), so a cold build can be explained afterwards. The
optional `scripts/seed-worktree-target.sh` seeds a new worktree with APFS clones of the main
checkout's `local-install` and `debug` profiles: 6 s plus a 1 m 20 s first build (77 s user CPU)
against 3 m 02 s (392 s user CPU) from an empty `target/`, with no measurable physical disk use for
the clone. It must clean every path package afterwards (mtime freshness would otherwise reuse the
source checkout's artifacts), skips the non-relocatable Swift cache, and refuses to run across
volumes because `cp -c` silently degrades to a full copy there.

Verified on the real tree: the first install after these changes compiled the affected crates in
1 m 38 s; a second install reused everything (0.88 s build, copy and signing skipped) and, with
`debug` at 6,818 MB against its 6,144 MB budget, cleaned only this package's artifacts (to 3,237 MB).

Considered but not part of this round: splitting the crate into a workspace, and reclaiming other
checkouts' stale build directories (`.build`, `demo/*/target`, an older worktree's `target/debug`).

## When to revisit architecture

The Rust source grew from 35,683 lines on 2026-06-19 to 109,348 lines on 2026-08-09 while remaining
one binary crate. Module files do not create Cargo compilation boundaries. If small local-install
edits again exceed roughly 15 seconds despite an unchanged frontend, the next investigation should
measure extracting stable subsystems or Tauri resource context into separate crates. Profile
tuning has already been measured; repeating it without a material compiler/project change is
unlikely to help.
