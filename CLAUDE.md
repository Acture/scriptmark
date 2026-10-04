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
cargo test -p scriptmark                 # Core and integration tests; requires Python
cargo test --workspace                   # Includes the PyO3 binding crate
cargo clippy --all-targets               # Lint — must be 0 warnings
cargo fmt                                # Format all code
cargo fmt --check                        # Verify formatting
maturin develop                          # Build + install PyO3 Python bindings locally
```

Single test: `cargo test -p scriptmark test_name -- --nocapture`.
For the PyO3 crate, select a supported interpreter via `PYO3_PYTHON` when the machine's
default Python is newer than the PyO3 version supports; see the P-676 validation record.

## Architecture

Single `scriptmark` crate (lib + bin) with `scriptmark-py` as separate cdylib for PyO3.

All product source is under `src/`: Rust packages live in `src/crates/` and the
Python package entry point in `src/python/scriptmark/`. Workspace/build configuration
stays at the repository root, where Cargo and Maturin commands are run. The module
paths below are relative to `src/crates/scriptmark/src/`.

Data flows: TOML specs + student files → Runner → grading record (evidence + score
revisions) → Display/DB/HTML/Canvas. `rescore` adds a revision from saved evidence.

```
models/         Data models, TOML spec parsing, grading policies
discovery       Student file discovery + ZIP extraction
runner/         PythonExecutor (subprocess), orchestrator, sandbox (setrlimit),
                parametrize, expander, oracle, linter
checker/        Checker trait (8 impls) + Rhai + Python checkers
record          Grading record: versioned evidence, score revisions, rescore reuse checks
db/             SQLite (rusqlite bundled): students, sessions (= revisions), results, similarity
canvas/         Canvas LMS API client (reqwest + rustls): roster pull, grades push
tui/            ratatui terminal UI: students/sessions/similarity tabs
scriptmark-py   PyO3 bindings: grade, run, discover, load_spec (maturin, separate crate)
```

## Key Design Decisions

**No pytest.** ScriptMark is its own test engine. Student code runs in subprocess via embedded Python helper script (`python.rs::HELPER_SCRIPT`). The helper: syntax-checks with py_compile, suppresses input()/print() during module load, calls target function, returns JSON on stdout.

**Sandboxed execution.** Student subprocess runs with: (1) `env_clear()` — no access to host env vars, (2) import allowlist — only safe stdlib + teacher-specified `allowed_imports`, (3) `setrlimit` — kernel-enforced CPU/file size/fd/process limits, (4) `tokio::join!` pipe reading + explicit `child.kill()` on timeout. Python interpreter resolved to absolute path at construction time so `env_clear()` doesn't change which interpreter runs.

**TOML test specs** have three sections: `[vars]` (constants injected as Python globals), `[[setup]]` (function calls storing results for `$ref`), `[[cases]]` (scored test cases). Cases support `expect`, `expect_error`, checkers (`check = "sorted"` / `{ rhai = "..." }` / `{ python = "verifiers/check.py" }`), and `[cases.parametrize]` with oracle. `[meta]` supports `allowed_imports` for teacher-controlled package access.

**Grading** uses declared per-item points and aggregation in `assignment.toml`. The
default grade is the raw score scaled to the configured total; curves are opt-in.
Zero and withheld grades stay distinct. See the notes' teacher contract for policy fields.

**Similarity detection** computes both style similarity (raw code — catches identical variable names/spacing) and structural similarity (normalized — catches renamed variables). Combined score = max of both.

**ZIP extraction** auto-extracts student `.zip` submissions with size limits (5MB/file, 50MB total, 100 files max) to prevent zip bombs.

## Critical Files

- `src/crates/scriptmark/src/runner/python.rs` — PythonExecutor + embedded helper scripts (HELPER_SCRIPT, CHAIN_HELPER_SCRIPT). Contains import allowlist, env sanitization, `sandboxed_cmd()`, `spawn_with_timeout()`.
- `src/crates/scriptmark/src/runner/sandbox.rs` — SandboxConfig + setrlimit application. Platform-conditional: RLIMIT_AS skipped on macOS, RLIMIT_* constants differ between Linux (c_uint) and macOS (c_int).
- `src/crates/scriptmark/src/runner/orchestrator.rs` — Runs vars→setup→expand→oracle→execute pipeline per student, tokio parallel.
- `src/crates/scriptmark/src/models/spec.rs` — All TOML spec structs (TestSpec, TestCase, SetupStep, Parametrize, Oracle, LintConfig, CheckMethod).
- `src/crates/scriptmark/src/discovery.rs` — File discovery + ZIP archive extraction with size/count limits.
- `src/crates/scriptmark/src/record.rs` — The grading record every consumer reads (`--output`): evidence with submission/spec fingerprints, append-only score revisions, `check()` refusing evidence whose tests, submissions or matching changed.
- `src/crates/scriptmark/src/grading.rs` — GradingPolicy dispatch (templates + Rhai formulas).
- `src/crates/scriptmark/src/main.rs` — All CLI command handlers.
- `src/crates/scriptmark-py/src/lib.rs` — PyO3 bindings: grade(), run(), discover(), load_spec(), StudentResult, TestSpec classes.

## Conventions

- Workspace dependencies in root `Cargo.toml`, crates reference with `{ workspace = true }`
- `thiserror` for library error types, `anyhow` for CLI/binary error handling
- All checkers implement `Checker` trait in `src/crates/scriptmark/src/checker/mod.rs`
- Integration tests spawn real Python processes — need `python3` available
- `scriptmark-py` has `publish = false` (cdylib, distributed via PyPI/maturin, not crates.io)
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
