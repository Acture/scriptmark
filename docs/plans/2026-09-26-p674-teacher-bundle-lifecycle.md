# P-674 — A complete teacher test bundle, with an explicit execution lifecycle

Linear: https://linear.app/acturea/issue/P-674
Parent: P-663 · Blocks: P-675, P-676, P-677

Revision 1, written against `7185534`.

## Scope

A teacher hands ScriptMark a directory of TOML specs plus whatever teacher code and data
they reference, and gets grades — with no generator, reference implementation or seed
required. Everything the bundle says is either honoured or refused before a student runs.

P-674 owns four things:

1. **The contract.** Independent cases and shared-state scenarios are declared, not
   inferred. Setup declares the scope it runs in and runs exactly once in that scope.
2. **The execution interface.** The orchestrator drives a real `Executor` trait; the
   Python backend implements it. One harness, one protocol.
3. **Evidence.** What a student's code was given, what it did (returned / raised / timed
   out / printed / wrote), and what the judge decided — kept apart, with every non-pass
   carrying who is responsible for it.
4. **Refusal.** Every configuration that today falls through to a default is rejected
   when the spec loads, or when the bundle is prepared, before any student is run.

**Not** in scope:

| Ticket | Owns | P-674 leaves it |
| -- | -- | -- |
| P-673 | which file / which function a student's code maps to | `find_student_file_with_hint` and `_fuzzy_lookup` move unchanged |
| P-675 | generation: binding order, seed recording, frozen replayable artefacts, generation errors | expansion moves into `prepare` unchanged, so it runs once per bundle instead of once per student |
| P-676 | reference answers: no fuzzy matching, oracle failure as a preparation error, versioning | oracle resolution moves into `prepare` unchanged; a case left with no expectation can never pass (D9) |
| P-677 | turning statuses and faults into a score; withholding grades on teacher/environment faults | `grading.rs` is untouched; `fault` is supplied, not consumed |
| P-678 | versioned result files, re-grading from saved evidence | new `CaseResult` fields are optional and additive |

## What is true today

Verified against `7185534`, plus one run of the CLI on a two-line student file.

**Lifecycle is inferred.** `orchestrator.rs:221` picks "chain mode" — one process for
every case — when `meta.imports` is non-empty **or any case sets `function`**. Adding a
helper import, or overriding the function on one case, silently makes every case in the
spec share interpreter state. Where setup runs, and how often, follows from that switch:

| | per-case mode | chain mode |
| -- | -- | -- |
| student `setup` | once per student, in its own process, value JSON round-tripped into every case | once per student, live object, deep-copied per case if `copy_refs` |
| setup that raises | the exception text fails to parse as JSON and becomes `null`; cases run with `null` (`orchestrator.rs:152-160`) | every case `Error` |
| `setup.file` | `python3 <path>` per student, unsandboxed, path relative to the grader's cwd (`orchestrator.rs:165`) | **silently dropped** by `filter_map` (`python.rs:1050`) |
| unknown `$ref` | `null` (`resolve.rs:13`) | the literal string `"$name"` (`python.rs:230`) |
| per-case `timeout` | honoured | ignored; one timeout for the chain, and on expiry **every** case is `Timeout` (`python.rs:1083`) |
| import guard during calls | on | **off** — restored at `python.rs:357`, so a function body can `import os` |

**Protocol and student output share stdout.** Both helpers restore the real stdout before
calling student code, then print their JSON to it. A correct student whose function calls
`print` gets `Error: Failed to parse helper output` on every case — reproduced with
`def add(a, b): print("debug", a, b); return a + b`. The payload also travels in `argv`,
which a large bundle will overflow.

**Every student shares the grader's working directory.** No `current_dir` is set, so a
student who writes `out.txt` writes it where `scriptmark` was launched, racing every other
student doing the same.

**Errors have no owner.** A Rhai evaluation error, a crashing Python checker script, an
exception inside a teacher `@checker`, a teacher module that fails to import, a missing
student file and a failed `spawn` all end as `Failed` or `Error` with a message string.
Nothing records whether the student, the teacher or the machine is responsible.

**Configuration falls through to defaults.**

| Configuration | What happens |
| -- | -- |
| `check = { exec = … }` / `{ wasm = … }` | `ExactChecker` (`python.rs:966`) |
| `check = "nonsense"` | `ExactChecker` |
| `check = { regex = … }` (documented in README) | no such field; `ExactChecker` |
| `expected = 5` (typo for `expect`) | ignored; the case expects `null` |
| `oracle.python` | ignored (`oracle.rs:52`); expectation `null` |
| `language = "cpp"`, `compile = …` | runs as Python / ignored |
| `expected_stdout` on a function case | ignored |
| `stdin` on a function case | ignored; `input()` returns `"0"` forever (patched at load, never restored) |
| a case with nothing to compare | passes only if the function returns `None` |
| `id` on a case | accepted, never used |
| a parametrized case with `function = …` | generated cases drop it and call `meta.function` (`expander.rs:31`) |
| Python checker / oracle `reference` paths | resolved against the grader's cwd, not the spec |

**The execution interface is dead.** `runner/executor.rs` declares a trait nothing
implements — its signature takes one `&StudentFile`; `PythonExecutor::execute_case` takes a
slice — and `run_all` takes `&PythonExecutor` directly. `scriptmark-py`'s `load_spec`
parses TOML itself and skips `spec_loader`.

## Decisions

### D1 — Two execution units, declared by where they are written

```toml
[[cases]]          # independent: its own process, its own working directory
[[scenarios]]      # shared state: one process, one working directory, steps in order
```

A **case** is one call judged once. Nothing survives from one case to the next: not
interpreter state, not objects, not files. A **scenario** is a named sequence of steps
run in order in one process; every step is judged and reported as its own case, named
`"<scenario> / <step>"`. Nothing else chooses the lifecycle. `imports`, `function`,
`vars` and `data_files` never change which unit a test belongs to, or how it is isolated.

`copy_refs` is deleted. It existed to fake isolation inside a shared process; a case now
has real isolation, and a scenario wants the sharing.

### D2 — Setup declares its scope and runs exactly once in it

| Written in | `scope` | Runs | May do |
| -- | -- | -- | -- |
| top-level `[[setup]]` | `"bundle"` (required) | once per grading run, in `prepare`, before any student | `file = "gen.py"` — a teacher script whose stdout is JSON |
| top-level `[[setup]]` | `"each"` (required) | once at the start of **every** case and **every** scenario, inside that unit's process | call student code, a method, or a teacher function |
| `[[scenarios.setup]]` | none (forbidden) | once at the start of that scenario, after `"each"` setup | same as `"each"` |

`scope` has no default. Inferring it from `file` vs `function` would be the implicit
switch this ticket removes; a spec without it is refused with a message naming both
values and what each means. A bundle value is JSON, frozen, identical for every student;
an `"each"` value is a live object that exists only inside its unit.

`"bundle"` setup may not call student code; `"each"` and scenario setup may not run a
`file`. A spec whose cases run the file as a script (D6) may not declare `"each"` setup.

### D3 — A call names its target explicitly

Every setup step, case and scenario step that calls something names exactly one of:

| Field(s) | Calls | Whose code |
| -- | -- | -- |
| `function = "name"` | a callable on the student module (a class constructs an object) | student |
| `method = "name"`, `object = "id"` | a method of the live object stored under `id` | student |
| `teacher = "name"` | a function exported by a teacher module (`imports`) | teacher — setup only |

`meta.function` is the default `function` for cases and steps that name no target.
`object` must name an id produced earlier **in the same unit** by a student call — an
`"each"` or scenario setup step, or a scenario step with an `id`. That rule is what lets
the harness attribute a failure by which call it was making (D7). `teacher` is setup-only:
a teacher action is never a scored case.

Setup and scenario steps store their return value under `id`; later calls reference it as
`"$id"` in `args`. `$$` escapes a literal leading `$`. Names in scope: `vars`, `"bundle"`
setup ids, teacher module exports, `"each"` setup ids, scenario setup ids and earlier step
ids. An unknown `$name` is a preparation error, never `null` and never a literal string.

Function lookup keeps today's `_fuzzy_lookup`; P-673 owns it. Methods and teacher
functions are looked up by exact name.

### D4 — Files: staged in, observed out, never the grader's cwd

- `meta.data_files = ["data/lines.csv", "fixtures/"]` — paths relative to the spec,
  checked to exist at load, copied (directories recursively) into every unit's working
  directory at the same relative path.
- Every unit runs in a fresh temporary directory — one per case, one per scenario — which
  is its `cwd` and is removed afterwards. The student file, teacher modules and scripts are
  passed as absolute paths, so the change of `cwd` cannot break them.
- `expect_files = { "out.txt" = "hello\n" }` on a case or step — after the call, the
  harness reads each path (relative, no `..`) from the working directory; a missing file
  or different content fails the case. Contents also reach Rhai and Python checkers as
  `context.files`, and an in-process checker can simply open the file.

### D5 — One harness, one framed protocol, payload on stdin

`HELPER_SCRIPT` and `CHAIN_HELPER_SCRIPT` are replaced by one `runner/harness.py`
(`include_str!`), which runs a unit:

teacher imports → import guard on → student import (or script) → `"each"` setup →
scenario setup → steps.

- **Payload** is JSON on stdin, read in full before any student code loads.
- **Records** go to a private duplicate of the original stdout, one line each, prefixed
  with a per-run nonce: `@@scriptmark:<nonce>@@ {json}`. The Rust side ignores every other
  line, so nothing a student writes can corrupt a record.
- **Student stdout** is captured per call by swapping `sys.stdout` for a capped buffer
  (64 KiB); a student printing in a loop cannot exhaust memory.
- **One record per call**, flushed as it completes, so a unit killed at its deadline still
  reports everything that finished; the first call without a record is the one in flight.
- **Import guard** is on for every student call and off for teacher code — today it is
  off for chain-mode calls. Its allowlist is unchanged.
- **`input()`** returns `"0"` while the student module loads (unchanged); during a call it
  reads the call's `stdin`, or raises `EOFError` if none was declared.
- **Values** go through one serialiser: tuples → lists, sets → sorted lists, dict keys →
  strings, non-finite floats → their `repr`, anything else → `str()`.

```jsonc
{"kind": "ready"}                                    // teacher + student modules loaded
{"kind": "call", "phase": "setup" | "step", "index": 0,
 "outcome": {"returned": {"value": …, "type": "int"}}
          | {"raised": {"type": "KeyError", "message": "…"}}
          | {"timeout": {}},
 "stdout": "…", "files": {"out.txt": "…" | null},
 "check": {"pass": true, "message": ""} | {"error": {"type": …, "message": …}},
 "fault": "student" | "teacher", "elapsed_ms": 3}
{"kind": "fatal", "stage": "teacher_import" | "student_syntax" | "student_import" | "harness",
 "fault": "teacher" | "student" | "environment", "error": {"type": …, "message": …}}
{"kind": "done"}
```

### D6 — Timeouts belong to calls

- `--timeout` is the default for every call: the student import, each setup call, each
  case, each step. A case or step may override it; a scenario's `timeout` is the default for
  its setup and steps.
- In the harness, each call runs under `signal.setitimer(ITIMER_REAL)`; the handler raises
  a `BaseException` subclass, so `except Exception` in student code cannot swallow it. A
  timed-out step is `Timeout`; later steps in the scenario still run.
- The Rust side kills the process at a deadline of *import + Σ call timeouts + 2 s*, then
  reads what the harness managed to write. The call in flight becomes `Timeout`; calls
  after it become `Error` — not run because an earlier call hung.
- `setitimer` is Unix-only, like the sandbox; elsewhere only the process deadline applies.

A case with no target and no `meta.function` runs the file as `__main__` ("script mode",
today's IO mode) — inside the harness, so it gets the same guard, capture and records. Its
observation is stdout (plus any exception); `expect`, `check` and `object` are refused on
it, `expected_stdout`, `expect_files` and `expect_error` are allowed.

### D7 — Evidence: observation, then verdict, then fault

The executor returns observations; `runner/judge.rs` turns them into `CaseResult`s. The
judge applies, in order: outcome vs `expect_error`; the value check (explicit checker,
in-process verdict, or exact against `expect`); `expected_stdout` (exact); `expect_files`.
A case passes only if every expectation it declared passes.

`CaseResult` gains two optional fields (`#[serde(default)]`, absent in older results):

```rust
pub fault: Option<Fault>,     // Student | Teacher | Environment; None when Passed
pub stdout: Option<String>,   // what the student printed during the call, when non-empty
```

`TestStatus` keeps its five variants; together with `fault` they separate what P-677
needs to score differently:

| What happened | status | fault |
| -- | -- | -- |
| no file matched `meta.file` | `Missing` | student |
| target function / method not found | `Missing` | student |
| returned the wrong value; wrong or missing exception; stdout or file mismatch | `Failed` | student |
| raised an exception nobody expected; syntax or import error; setup call raised | `Error` | student |
| a call exceeded its timeout; killed at the deadline | `Timeout` | student |
| not run because an earlier call in the unit hung or the process died | `Error` | student |
| teacher module failed to import; teacher setup function raised | `Error` | teacher |
| a checker could not decide: Rhai evaluation error, Python checker crash / timeout / bad output, in-process checker raised or returned neither `bool` nor `(bool, str)` | `Error` | teacher |
| nothing to judge against (e.g. an oracle produced no expectation) | `Error` | teacher |
| spawn failed; harness crashed; records missing or malformed without a kill | `Error` | environment |

Faults are assigned by **which call the harness was making**, never by reading a
traceback. A checker that crashes on a malformed student value is a teacher fault: it is
the checker's job to return `False` for a wrong answer, and the safe side of an ambiguous
case is "ungraded", not "zero". Rhai expressions are compiled at load, so a syntax error is
a refused spec, not a runtime fault.

`Checker::check` returns `Result<CheckOutput, String>`; `Err` is "the checker could not
decide". `CheckOutput` stays the Python checker script's wire format.

### D8 — Load, prepare, run

```text
load_spec(path)      -> Result<TestSpec, SpecError>        static: shape, fields, paths
prepare(specs, exec) -> Result<Vec<Bundle>, PrepareError>  once per run: teacher code
run_all(students, bundles, exec, …) -> Vec<StudentReport>  per student, per unit
```

**`load_spec`** refuses (all problems reported together, with the file path):
unknown fields anywhere (`deny_unknown_fields`); `language` other than `python`; `compile`;
a detailed `check` naming zero or several kinds, `exec`/`wasm`, an unknown builtin,
`tolerance` without `approx`, a Rhai expression that does not compile, a Python checker
script that does not exist; an oracle naming several kinds or `python`; setup without
`scope`, with the wrong target for its scope, or with a duplicate id; `expect` together
with `expect_error`; `id` on a case; `object` that does not name an earlier student-produced
id; a scenario with no steps, a parametrized step, or a step in script mode; script-mode
cases with `expect`/`check` or alongside `"each"` setup; missing `data_files`; `expect_files`
paths that are absolute or contain `..`; a zero timeout; a spec with no cases and no
scenarios. Every teacher path (`imports`, `data_files`, `setup.file`, Python checker
scripts, oracle `reference`) is resolved against the spec's directory and made absolute.

**`prepare`** runs teacher code once per bundle: imports every teacher module in a
throwaway interpreter and records its exports and `@checker` registry; runs `"bundle"`
setup scripts (cwd = spec directory, `--timeout`, stdout must be JSON); expands
parametrized cases and resolves oracles (both unchanged, now once instead of per
student). It then refuses: an unknown `$ref`; a `teacher` or `check.function` name the
modules do not export; a checker whose dependency parameters name nothing in scope; a case
or step with nothing to judge — no `expect`, `expect_error`, `expected_stdout`,
`expect_files`, `check`, bound `@checker` or oracle; duplicate case names after expansion.
Any failure is a `PrepareError`, and the CLI refuses to grade — the same stance
`build_local_input` already takes on input errors.

**`run_all`** is generic over `E: Executor`. Each unit is one task holding one permit of
the existing semaphore, so independent cases of one student run concurrently; results are
reassembled in declaration order (cases, then scenarios).

```rust
pub trait Executor: Send + Sync + 'static {
	fn language(&self) -> &str;
	fn prepare(&self, spec: &TestSpec) -> impl Future<Output = Result<TeacherRuntime, PrepareError>> + Send;
	fn run(&self, plan: &UnitPlan) -> impl Future<Output = UnitObservation> + Send;
}
```

`UnitPlan` is language-neutral (student file, teacher modules, values, staged files, calls
with targets, args, timeouts, stdin and expectations the harness needs). File matching
moves unchanged from `PythonExecutor` to `runner/matching.rs`; P-673 owns what it does.

### D9 — A case that declared nothing never passes

The judge's last rule: if none of the case's expectations was evaluated, the result is
`Error` / teacher — "nothing to judge". `prepare` already refuses such cases statically;
this guards the paths `prepare` cannot see, chiefly an oracle that yields no value (whose
proper handling is P-676's).

### D10 — Spec shapes in use today, and how they are spelled now

Shapes, not content — no course material is reproduced here.

| Shape | Today | Now |
| -- | -- | -- |
| fixed args / expect against `meta.function` | per-case | unchanged |
| several functions in one file, `function` per case, no imports | **chain** (shared process) | unchanged spelling, now independent |
| a student loader called once in setup, its result passed as `$id` to many cases | chain, deep-copied per case | `scope = "each"`: re-run in every case; or a scenario if sharing is wanted |
| setup values consumed by a teacher `@checker` via parameter names | chain | `scope = "each"`; injection unchanged |
| a teacher module exporting a data-file path used as `$NAME` | chain | unchanged; or `data_files` + a relative path |
| stdin → stdout scripts | IO mode | script mode, unchanged spelling |
| parametrized with Rhai / reference oracle | per student | unchanged spelling; expanded once |

Specs with top-level `[[setup]]` or `copy_refs` stop loading, with a message that names the
fix. That is the intended outcome of "refuse, do not guess".

## Touch list

| File | Change |
| -- | -- |
| `models/spec.rs` | `deny_unknown_fields`; `Call`, `SetupScope`, `Scenario`; `data_files`, `expect_files`; typed check / oracle validation; delete `copy_refs` |
| `models/result.rs` | `Fault`; `CaseResult.fault`, `CaseResult.stdout` |
| `spec_loader.rs` | path resolution for every teacher path; static validation; `SpecError::Invalid` |
| `runner/executor.rs` | real trait, `UnitPlan`, `UnitObservation` |
| `runner/prepare.rs` (new) | `Bundle`, `prepare`, `PrepareError`, dynamic validation |
| `runner/judge.rs` (new) | observation → `CaseResult` |
| `runner/harness.py` (new) | the one harness |
| `runner/python.rs` | `PythonExecutor: Executor`; spawn, stdin payload, deadline, record parsing; old helpers deleted |
| `runner/matching.rs` (new) | `find_student_file_with_hint`, moved |
| `runner/orchestrator.rs` | generic; unit tasks; no chain switch |
| `runner/oracle.rs`, `runner/expander.rs` | called from `prepare`; expander keeps every non-parametrize field |
| `runner/resolve.rs` | deleted — references resolve in the harness |
| `checker/*` | `Result<CheckOutput, String>` |
| `main.rs`, `scriptmark-py/src/lib.rs` | `prepare` before `run_all`; bindings' `load_spec` goes through `spec_loader` |
| `display.rs`, CSV archive | show `fault`; CSV gains a trailing `fault` column |
| `examples/bundles/{pure_function,shared_object,file_io}/` | one bundle each, a correct and a wrong student |
| `docs/test-bundles.md` (new), `README.md` | teacher-facing contract; drop the non-existent `regex` checker |

## Tests

Mapped to the ticket's acceptance lines.

**Runs with a complete teacher bundle alone.** Every example bundle grades end-to-end
with no `parametrize`, oracle or seed; the correct student passes everything, the wrong
one fails exactly the cases it should.

**Pure function, shared object, file read/write.** One integration test per example
bundle: a pure function with `expect`/`expect_error`; a class whose object is built in
scenario setup and driven by `method` steps, with a later step observing state an earlier
step left; a function reading a staged data file and another writing `expect_files`, with
two concurrent units writing the same filename without interfering, and nothing written
to the grader's cwd.

**Imports do not change isolation.** The same spec with and without an `imports` line: a
student function that increments a module global returns `1` in every case both times.
Same for a case-level `function` override.

**Setup count, timeout, stdout, error origin are verifiable.** Setup calls append a line
to an absolute path passed through `vars`; after a run, `"bundle"` wrote one line per run,
`"each"` one per unit, scenario setup one per scenario. A step that loops forever is
`Timeout` while its neighbours keep their results; a case's own `timeout` overrides the
default. A student that prints inside the graded function passes and its output is in
`stdout` (the reproduction above). The fault table in D7 is one test per row.

**Refused before running.** One test per refusal in D8, each asserting the message names
the problem and that `run_all` is never reached.

**Unit tests:** spec validation table; judge table (observation → status/fault); record
parsing (noise lines, a truncated stream after a kill, a missing `done`, a forged line
with the wrong nonce).

## Verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test -p scriptmark
PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo check -p scriptmark-py
ruff check crates/scriptmark/src/runner/harness.py
```

`tests/canvas_fetch.rs` binds local ports and fails inside the Claude Code sandbox; it is
run outside it.
