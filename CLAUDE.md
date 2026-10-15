# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Documentation workflow

`CLAUDE.md` is the canonical project instruction file; `AGENTS.md` and
`.github/copilot-instructions.md` are symlinks to it.

- Public documentation belongs in `docs/`. Private documentation is in the `notes/` git submodule:
  `https://github.com/Acture/obsidian-vault.git`, branch `project/scriptmark`.
  Start at `notes/README.md`; the teacher contract is
  `notes/docs/test-bundles.md`, and plans are in
  `notes/docs/plans/`. Do not copy private notes into the public documentation tree.
  The project branch root contains only this project's notes; the vault's `master`
  places them under `scriptmark/`. Keep shared automation and vault configuration on master.
- Default to the latest remote `project/scriptmark` notes. At the start of every
  session, after pulling code, and before reading or editing notes, follow the native
  Git update sequence in README's "Documentation and checkout" section. It explicitly
  fetches the project branch, checks for local work, switches to `project/scriptmark`
  and fast-forwards it. It handles single-branch clones. Never assume the gitlink is latest.
- Clone with `git clone https://github.com/Acture/scriptmark.git`, enter the clone,
  then initialize with `git submodule update --init --remote --no-single-branch -- notes`
  and `git -C notes switch project/scriptmark`. For existing notes, use the guarded
  update sequence instead. Stop on local edits, unpushed commits or divergence; preserve
  and reconcile them instead of resetting or force-pushing. If fetching fails, report
  that the latest notes were not verified; do not describe stale notes as current.
  Shared synchronization and submission tooling belongs to the notes repository;
  do not add a project-local notes updater.
- Edit private documentation only in `notes/`. A newer notes checkout can make the parent's gitlink
  dirty; review `git diff --submodule=log -- notes` and include the updated reference in
  the next code commit. Do not reset notes just to remove that expected difference.
- The gitlink remains a concrete saved checkpoint because Git requires one. Use
  `git submodule update --init --checkout -- notes` only for an explicitly requested
  historical reproduction, with clean notes. Pinned checkout is not the daily workflow.
- Stage specific documents in `notes/` and commit there. Publish with the master-owned
  `hooks/notes-boundary/submit_project.py --repo notes` under the notes repository's
  common Git directory; it runs the required remote check before pushing that SHA.
  The remote requires `notes-boundary/root/scriptmark` from GitHub Actions.
  Install/update this helper from the notes repository's trusted `origin/master` as
  shown in README. Direct pushes of unchecked commits are rejected. Only after a
  successful submission, stage `notes` here and commit/push its new gitlink.
- The README contains the complete command sequence. Keep original histories and
  resolve divergence without overwriting either side.
- Linear is the execution home. Keep runnable examples, code, measurements and raw
  artifacts here; maintain narrative/design documentation on the notes branch.
  Historical code paths inside imported plans describe their original revision.

## What is ScriptMark

Automated grading CLI for student programming assignments. Rust core with custom test engine (no pytest dependency), TOML-based test specifications, multi-language support planned.

## Build & Test Commands

```bash
cargo check                              # Fast type check
cargo build -p scriptmark                # Build CLI binary
cargo test -p scriptmark-core            # Core tests; requires Python
cargo test -p scriptmark                 # CLI and adapter integration tests
cargo test --workspace                   # Both crates
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt                                # Format all code
cargo fmt --check                        # Verify formatting
```

Single test: `cargo test -p scriptmark-core test_name -- --nocapture` (or `-p scriptmark`
for CLI tests). Python 3 is required by the subprocess executor, not by a Python SDK.

## Architecture

Two Rust crates live directly under `src/`: `src/core/` contains `scriptmark-core`,
and `src/cli/` contains the `scriptmark` CLI and its adapters. The CLI depends on the
core; core must not depend on CLI, HTTP, SQLite or terminal UI libraries.

Cargo workspace/build configuration stays at the repository root. There is no Python
package, PyO3 binding or Python wheel release. Embedded Python executor/checker code
and runnable Python assignment examples remain part of the Rust product.

Data flows: TOML specs + student files → Runner → grading record (evidence + score
revisions) → Display/DB/HTML/Canvas. `rescore` adds a revision from saved evidence.

```
core/src/models/     Data models, TOML spec parsing, grading policies
core/src/discovery   Student file discovery + archive extraction
core/src/runner/     PythonExecutor (teacher + student processes), harness/*.py,
                orchestrator, sandbox (Seatbelt / Landlock + seccomp, setrlimit),
                records (framed), parametrize, expander, oracle, linter
core/src/checker/    Checker trait + Rhai + Python checkers
core/src/record      Versioned evidence, score revisions, rescore reuse checks
cli/src/db/          SQLite: students, sessions (= revisions), results, similarity
cli/src/canvas/      Canvas HTTP client and offline bundle adapter
cli/src/tui/         ratatui terminal UI: students/sessions/similarity tabs
```

## Key Design Decisions

**No pytest.** ScriptMark is its own test engine. A unit (a case or a scenario) runs as two Python processes from embedded scripts (`runner/harness/`): the teacher side (`teacher.py`: teacher modules, function checkers, the plan, per-request deadlines, the records) and the student side (`student.py`: the student module only, one call per request, under its timer and import guard). They share `common.py` (framing, the record and transfer value encodings, `call()`). ScriptMark joins the two with a pair of pipes and reads length-prefixed JSON records from the teacher side; values cross by value (shared-class instances rebuilt from the receiver's copy, other objects as snapshots).

**Sandboxed execution.** Each side runs confined by default (`runner/sandbox.rs`): macOS Seatbelt via `sandbox-exec` with a generated profile, Linux Landlock + seccomp applied in `pre_exec`. The student side reads its interpreter and its own directory only; the teacher side also reads the bundle and a read-only source copy. Fail closed: no sandbox, no grading, unless `--no-sandbox` (recorded as `isolation: unisolated`). Both also get `env_clear()`, `setrlimit` (RLIMIT_AS only as a backstop), and an absolute interpreter path; the student side is watched against `memory_mb` (spec, case or scenario; default 256 MiB). The import allowlist is a teaching rule, not a boundary. Sandbox tests need a real sandbox: run `cargo test` outside any other Seatbelt sandbox (Claude Code's included), and on Linux from a filesystem with stable inodes (not a Docker Desktop bind mount).

**TOML test specs** have three sections: `[vars]` (constants injected as Python globals), `[[setup]]` (function calls storing results for `$ref`), `[[cases]]` (scored test cases). Cases support `expect`, `expect_error`, checkers (`check = "sorted"` / `{ rhai = "..." }` / `{ python = "verifiers/check.py" }` / `{ function = "..." }`, which may take the reserved `stdout` and `source`), and `[cases.parametrize]` with oracle. `[meta]` supports `allowed_imports` (a teaching rule) and `shared` (modules whose classes cross between the two sides by value). Prepare refuses a path into the bundle handed to student code: `data_files` stages it instead.

**Grading** uses declared per-item points and aggregation in `assignment.toml`. The
default grade is the raw score scaled to the configured total; curves are opt-in.
Zero and withheld grades stay distinct. See the notes' teacher contract for policy fields.

**Similarity detection** computes both style similarity (raw code — catches identical variable names/spacing) and structural similarity (normalized — catches renamed variables). Combined score = max of both.

**ZIP extraction** auto-extracts student `.zip` submissions with size limits (5MB/file, 50MB total, 100 files max) to prevent zip bombs.

## Critical Files

- `src/core/src/runner/python.rs` — PythonExecutor: stages a unit, starts and joins both sides, reads the records, watches memory, probes the interpreter's read roots.
- `src/core/src/runner/harness/{common,teacher,student}.py` — the two sides of a unit and what they share.
- `src/core/src/runner/sandbox.rs` — `Isolation`, `Confinement`, the Seatbelt profile and Landlock/seccomp rules, plus setrlimit. Platform-conditional: RLIMIT_AS skipped on macOS, RLIMIT_* constants differ between Linux (c_uint) and macOS (c_int).
- `src/core/src/runner/records.rs` — framed record reader with plan-derived bounds.
- `src/core/src/runner/orchestrator.rs` — Runs prepared bundles per student, tokio parallel.
- `src/core/src/models/spec.rs` — TOML test specification types.
- `src/core/src/discovery.rs` — File discovery and archive extraction with size/count limits.
- `src/core/src/input/local.rs` and `input/table.rs` — Assignment-relative local sources, explicit student/file manifests and CSV/XLSX roster mappings. Spreadsheet student IDs must be text; preserve row/sheet diagnostics.
- `src/core/src/record.rs` — Versioned grading evidence and score revisions, including reuse checks.
- `src/core/src/grading.rs` — GradingPolicy dispatch (templates + Rhai formulas).
- `src/cli/src/main.rs` — CLI command handlers and application orchestration.

## Conventions

- Workspace dependencies in root `Cargo.toml`, crates reference with `{ workspace = true }`
- `thiserror` for library error types, `anyhow` for CLI/binary error handling
- All checkers implement `Checker` trait in `src/core/src/checker/mod.rs`
- Integration tests spawn real Python processes — need `python3` available
- `scriptmark-core` exposes grading models and operations; CLI adapters import it directly, without compatibility re-exports.
- Core's `test-support` feature exposes shared graded/withheld report fixtures for adapter tests; it is enabled only by the CLI's dev-dependency.
- `[input]` in `assignment.toml` configures either submission paths plus a mapped CSV/XLSX roster, or `[[input.students]]` with exact files/archives. `grade`, `run` and `match` resolve the same input; rescore reconstructs it from saved CLI overrides and the recorded assignment file. `summarize --roster` and `db import-roster` read tables through `Config::roster_table` with `--assignment`'s layout. DB saves import the identities frozen in the record (`Record::roster`). File and function matching remains owned by `matching`.
- Publish both Rust packages with `cargo publish --workspace`, which orders workspace dependencies before their consumers.
- Platform-specific code uses `#[cfg(target_os = "macos")]` / `#[cfg(target_os = "linux")]` for rlimit types

## TOML Spec Example

```toml
[meta]
name = "find_larger_number"
file = "Lab5_1.py"
function = "find_larger_number"
language = "python"
allowed_imports = ["numpy"]  # extra packages beyond safe stdlib

[vars]
EPSILON = 0.001

[[setup]]
id = "data"
function = "load_data"
args = ["$EPSILON"]

[[cases]]
name = "basic"
args = [3, 5]
expect = 5

[[cases]]
name = "random pairs"
[cases.parametrize]
[cases.parametrize.args]
a = "int(-100, 100)"
b = "int(-100, 100)"
[cases.parametrize.random]
count = 20
seed = 42
[cases.parametrize.oracle]
rhai = "if a >= b { a } else { b }"
```
