# P-674 — A complete teacher test bundle, with an explicit execution lifecycle

Linear: https://linear.app/acturea/issue/P-674
Parent: P-663 · Blocks: P-675, P-676, P-677

Revision 2. Revision 1 (`b8d0939`, written against `7185534`) went through a six-lens
adversarial review: acceptance, migration of real spec shapes, fault attribution,
harness robustness, Rust feasibility, and scope. 70 findings were raised; three
refuters checked three of the lenses. The harness lens, and the refuters for rust and
scope, stalled on permission prompts, so the author verified those findings by
experiment instead. What changed, and why:

- **The payload moved off stdin.** On stdin it starved script-mode students reading
  `sys.stdin` or `open(0)` (D5).
- **Fault is no longer on the wire.** The harness reported its own `fault`. Student
  code in the same process could forge it, and it contradicted "attributed by which
  call". Rust now derives it from the record stream, the plan and the exit status (D7).
- **`SystemExit`, the serialiser, the in-process checker and death by signal each fell
  through to "environment".** Every one of them now has an owner. RLIMIT_CPU is derived
  from each unit's deadline, not fixed at 30 s (D6, D7).
- **Implicit `@checker` binding is removed.** A name-bound checker silently replaced
  `expect` and explicit checks in real specs: an always-true checker made exact cases
  pass for everyone (D8).
- **Checker runtime errors stay teacher faults, but the common case is refused
  before grading.** Revision 1 would have turned "student returned `None`" into an
  ungraded teacher error across a whole class, through Rhai checks that call methods
  on `result` (D9).
- **The `"bundle"` scope and `setup.file` are cut.** They were P-675's generation
  under another name. With one legal scope left, `scope` was ceremony: setup scope is
  now declared by placement, and existing specs keep loading (D2).
- **The import guard is fixed.** It was vetting the stdlib's own internal imports, so
  `import random`, `csv`, `datetime`, `json` or `pathlib` failed every case for a
  student. That is reproduced on `7185534` (D5).
- **The reference oracle gets a defined path.** "Moves unchanged" was
  unimplementable once `execute_case` goes (D11).
- **The interface types are specified.** P-675, P-676 and P-678 build on them, and
  `CaseResult` gains `cause` and `input` (D7, D12).

## Scope

A teacher hands ScriptMark a directory of TOML specs plus whatever teacher code and data
they reference, and gets grades. No generator, reference implementation or seed is
required. Everything the bundle says is honoured or refused before any student runs.

P-674 owns four things:

1. **The contract.** Independent cases and shared-state scenarios are declared, not
   inferred. Setup runs exactly once in the scope its placement declares.
2. **The execution interface.** The orchestrator drives a real `Executor` trait; the
   Python backend implements it with one harness and one protocol. The same interface
   runs a reference implementation.
3. **Evidence.** What the student's code was given, what it did (returned, raised,
   timed out, printed, wrote), and what the judge decided are kept apart. Every
   non-pass records whose it is and why.
4. **Refusal.** Every configuration that today falls through to a default is rejected
   when the spec loads, or when the bundle is prepared, before any student is run.

**Not** in scope:

| Ticket | Owns | P-674 leaves it |
| -- | -- | -- |
| P-673 | which file and which function a student's code maps to | `find_student_file_with_hint` and `_fuzzy_lookup` keep their rules; P-674 records what they picked |
| P-675 | generation: binding order, seed recording, frozen replayable artefacts, generated setup data | expansion runs once in `prepare`, unchanged; `setup.file` is refused until P-675 replaces it |
| P-676 | reference answers: provenance, versioning, answer priority, answers for stdout/files | the reference runs through the new interface with exact lookup; a failed reference is a preparation error (D11) |
| P-677 | turning status, fault and cause into a score; withholding grades | `grading.rs` logic is untouched |
| P-678 | versioned results, bundle digests, re-grading from evidence | `Bundle` and `CaseResult` are serialisable and carry what it needs; the digest is P-678's |

## What is true today

Checked against `7185534`, with the CLI run on small student files.

**Lifecycle is inferred.** `orchestrator.rs:221` picks "chain mode", one process for
every case, when `meta.imports` is non-empty **or any case sets `function`**. Adding a
helper import, or overriding the function on one case, silently makes every case in the
spec share interpreter state. Where setup runs, and how often, follows from that switch:

| | per-case mode | chain mode |
| -- | -- | -- |
| student `setup` | once per student, in its own process, value JSON round-tripped | once per student, live, deep-copied per case if `copy_refs` |
| setup that raises | the exception text fails JSON parsing and becomes `null`; cases run with `null` (`orchestrator.rs:152-160`) | every case `Error` |
| `setup.file` | `python3 <path>` per student, unsandboxed, relative to the grader's cwd (`orchestrator.rs:165`) | **silently dropped** (`python.rs:1050`) |
| unknown `$ref` | `null` (`resolve.rs:13`) | the literal string `"$name"` (`python.rs:230`) |
| per-case `timeout` | honoured | ignored; on expiry **every** case is `Timeout` (`python.rs:1083`) |
| import guard during calls | on | **off**, restored at `python.rs:357` |

**The import guard rejects the standard library.** `builtins.__import__` is checked for
every import in the process, including the ones an allowlisted module makes internally.
In a fresh interpreter under the guard, `random`, `csv`, `datetime`, `json`, `pathlib`,
`statistics` and `decimal` all fail. A student whose file starts `import random` gets
every case `Failed: ImportError: Module 'os' is not allowed in student code`. That was
reproduced through the CLI.

**Protocol and student output share stdout.** A correct student whose function calls
`print` gets `Error: Failed to parse helper output` on every case. The payload travels
in `argv`.

**Every student shares the grader's working directory.** No `current_dir` is set.

**Errors have no owner.** Rhai evaluation errors, crashing checker scripts, exceptions
in teacher `@checker`s, a teacher module that fails to import, a missing student file, a
failed `spawn`, and a student's `exit()` all end as `Failed` or `Error` with a message
string. `exit()` produces no JSON at all.

**Checking is bound implicitly.** A teacher `@checker("f")` in an imported module
replaces the judgement of *every* case calling `f`: its `expect` is never compared and
its explicit `check` never runs (`python.rs:419-426`, `1217-1251`). Adding an import
changes how existing cases are judged.

**Configuration falls through to defaults.**

| Configuration | What happens |
| -- | -- |
| `check = { exec = … }` / `{ wasm = … }` / `"nonsense"` | `ExactChecker` (`python.rs:966`) |
| `check = { regex = … }` (documented in README) | no such field; `ExactChecker` |
| `expected = 5` (typo for `expect`) | ignored; the case expects `null` |
| `expect = inf` / `nan` | loads as `null` |
| `check = "approx"` with no `expect` | compared against `null`; everyone fails |
| `oracle.python`; an unknown `oracle.check` | ignored; copied unchecked into `case.check` |
| a Rhai oracle returning an array | `null` (`oracle.rs:54-65`) |
| `language = "cpp"`, `compile = …` | runs as Python / ignored |
| `expected_stdout` or `stdin` on a function case | ignored; `input()` returns `"0"` forever |
| a case with nothing to compare | passes only if the function returns `None` |
| `id` on a case | accepted, never used |
| a parametrized case with `function = …` | generated cases drop it (`expander.rs:31`) |
| Python checker / oracle `reference` paths | resolved against the grader's cwd |

**The execution interface is dead.** `runner/executor.rs` declares a trait that nothing
implements, and whose signature does not match `PythonExecutor`. `run_all` takes
`&PythonExecutor`. `scriptmark-py`'s `load_spec` parses TOML itself, skipping `spec_loader`.

## Decisions

### D1 — Two execution units, declared by where they are written

```toml
[[cases]]          # independent: its own process, its own working directory
[[scenarios]]      # shared state: one process, one working directory, steps in order
```

A **case** is one call judged once. Nothing survives from one case to the next:
interpreter state, objects and files are all fresh. A **scenario** is a named sequence
of steps run in order in one process. Every step is judged and reported as its own
case, named `"<scenario> / <step>"`.

Nothing else chooses the lifecycle. `imports`, `function`, `vars` and `data_files` never
change which unit a test belongs to or how it is isolated. `copy_refs` is deleted: it
faked isolation inside a shared process, a case now has real isolation, and a scenario
wants the sharing.

### D2 — Setup scope is its placement, and it runs exactly once there

| Written in | Runs |
| -- | -- |
| top-level `[[setup]]` | once at the start of **every** case and **every** scenario, inside that unit's process |
| `[[scenarios.setup]]` | once at the start of that scenario, after the top-level setup |

A setup value is a live object that exists only inside its unit. Top-level setup
therefore reproduces today's `copy_refs = true`, since each case gets a fresh value,
and is the faithful migration for the specs that use it. A teacher who wants a value
shared and mutated across calls writes a scenario.

`setup.file` is refused: "not supported — put fixed data in `vars`, `data_files` or a
teacher module; generated inputs arrive with P-675". No real spec uses it. A spec whose
cases run as scripts (D6) may not have top-level setup.

### D3 — A call names its target explicitly

| Field(s) | Calls | Whose code | Where |
| -- | -- | -- | -- |
| `function = "name"` | a callable on the student module (a class constructs an object) | student | setup, case, step |
| `method = "name"`, `object = "id"` | a method of the live object stored under `id` | student | setup, case, step |
| `attribute = "name"`, `object = "id"` | reads an attribute of that object (no `args`) | student | case, step |
| `teacher = "name"` | a function exported by a teacher module | teacher | setup only |

- `meta.function` is the default `function` for cases and steps that name no target.
- `object` must name an id produced **in the same unit** by an earlier student call:
  a top-level or scenario setup step, or a scenario step with an `id`.
- A `teacher` call may not take a student-produced id as an argument. Together with the
  previous rule, this means the harness always knows whose code a call runs (D7).
- Setup steps and scenario steps store their value under `id`, and later calls use it
  as `"$id"` in `args`. `$$` escapes a literal leading `$`.
- Function lookup keeps today's `_fuzzy_lookup` for student targets (P-673 owns it).
  Methods, attributes and teacher functions are looked up by exact name.

**Names in scope** for `$ref`s and checker parameters are `vars`, teacher exports, and
the ids of top-level setup, scenario setup and earlier steps. A name bound in more than
one of these is a preparation error that names both places.

A teacher module's **exports** are its `__all__` when it defines one. Otherwise they are
its public names, excluding modules, and excluding classes and functions whose
`__module__` is another module. A helper that does `from pathlib import Path` does not
export `Path`, but a `DATA_FILE = Path(…)` constant is exported.

### D4 — Files: staged in, observed out, never the grader's cwd

- `meta.data_files = ["data/lines.csv", "fixtures/"]`: paths are relative to the
  spec, checked to exist at load, and copied (directories recursively) into every
  unit's working directory at the same relative path.
- Every unit runs in a fresh temporary directory, which is its `cwd`: one per case,
  one per scenario. **The matched student file is copied into it**, so
  `Path(__file__).parent / "data.csv"` finds the staged file and stray writes stay
  inside the directory. Evidence keeps the original path. Teacher modules are loaded
  from their absolute spec-relative paths.
- `expect_files = { "out.txt" = "hello\n" }` on a case or step: after the call, the
  harness reads each path (relative, no `..`) from the working directory. A missing file
  or different content fails the case. `null` is not writable in TOML, so "must not
  exist" is out of scope.
- The directory is removed when the unit ends, including after a kill. Staging and
  removal run on blocking threads.

### D5 — One harness, one framed protocol

`HELPER_SCRIPT` and `CHAIN_HELPER_SCRIPT` are replaced by `runner/harness.py`
(`include_str!`, checked with `ruff`). It runs one unit:

teacher imports → `ready` → inject `vars` as student globals → student load (guarded,
timed) → top-level setup → scenario setup → steps (each: call, then its check).

- **Payload.** A JSON file in a private temp directory (not the unit's `cwd`). The path
  is passed in `argv`; the harness reads the file and deletes it before any student code
  loads. fd 0 is the case's `stdin` in script mode and empty otherwise.
- **Records** go to a private `dup` of fd 1, each prefixed with a newline and a per-run
  nonce: `\n@@scriptmark:<nonce>@@ {json}`. Rust accepts the prefix anywhere in a line
  and ignores everything else. The nonce protects against accidental output. It does
  not stop a determined student inside the same process, who could reach any harness
  state, so the records carry observations, never verdicts about who is at fault, and
  a second record for the same call is treated as tampering (D7).
- **Student stdout** is captured per call in a buffer capped at 64 KiB. The record
  carries `stdout_truncated`.
- **stdin.** During module load `input()` returns `"0"`, as today. During a call,
  `sys.stdin` is a text stream over that call's declared `stdin`, empty if none was
  declared, and `input` is the builtin, which echoes its prompt into the captured
  stdout just as CPython does. In script mode the case's `stdin` is the real fd 0.
- **Import guard.** It runs only for imports whose importing module is the student
  module or the student script (`globals["__name__"]`), so allowlisted stdlib modules can
  import their own internals. It is on for every student call, including chain-mode-
  shaped specs, and off for teacher code. The allowlist is unchanged. It is a policy
  check, not a security boundary, since `importlib.import_module` bypasses it, and the
  docs say so.
- **Values** are serialised inside the student call's scope, under its timer and guard,
  by one total serialiser. Tuples become lists; sets become lists sorted by their JSON
  text; dict keys become `str`, and a collision is an error; non-finite floats become
  their `repr`; other objects become `str()`, falling back to `<TypeName>` when `str`
  raises. Cycles and depth beyond 100 become placeholders. A failure is the outcome
  `unserialisable`.
- **Exceptions.** The harness catches `BaseException` around every student phase. A
  `SystemExit` is recorded like any other exception, except that in script mode code
  `0`/`None` means the program finished normally.
- **Exit.** After `done` the harness calls `os._exit(0)`, so threads a student left
  running cannot hold the unit open.

```jsonc
{"kind": "ready"}                                   // teacher modules imported
{"kind": "call", "phase": "load" | "setup" | "step", "index": 0,
 "target": {"requested": "add", "resolved": "add_nums"},
 "outcome": {"returned": {"value": …, "type": "int"}}
          | {"raised": {"type": "KeyError", "types": ["KeyError", "LookupError", "Exception", …], "message": "…"}}
          | {"timeout": {}}
          | {"unserialisable": {"type": …, "message": …}}
          | {"missing": {"message": "function 'f' not found"}}
          | {"unresolved": {"name": "acct"}},       // a $ref / object whose producer failed
 "stdout": "…", "stdout_truncated": false, "files": {"out.txt": "…" | null}, "elapsed_ms": 3}
{"kind": "check", "index": 0, "verdict": {"pass": true, "message": ""} | {"error": {…}} | {"rejected": "…"}}
{"kind": "fatal", "stage": "teacher_import" | "harness", "error": {"type": …, "message": …}}
{"kind": "done"}
```

### D6 — Timeouts belong to calls; the process deadline is the backstop

- `--timeout` is the default for every call: the student load, each setup call, each
  case, each step, and each in-process check. A case or step may override it. A
  scenario's `timeout` is the default for its setup and steps.
- The harness runs each call under `setitimer(ITIMER_REAL, t, 0.05)`: a repeating timer
  whose handler sets a per-call flag and raises a `BaseException` subclass. If the flag
  was set, the call is `timeout`, whatever it returned, so a bare `except:` cannot turn
  a timeout into a pass. Later steps in a scenario still run. That choice keeps partial
  credit.
- Rust sets the unit's deadline to *Σ call timeouts + 2 s*, and its **RLIMIT_CPU to
  ⌈deadline⌉ + 1**. The kernel's CPU limit therefore can never fire before the
  harness's own timers. At the deadline Rust kills the process and reads whatever records
  arrived.
- `setitimer` is Unix-only, like the sandbox. Elsewhere only the deadline applies.

**Script mode** is declared: `script = true` on a case. It runs the student file as
`__main__` inside the harness. Its value is its stdout: `expected_stdout` is its
expectation, and `check` (default exact) compares them. `check = "text"` is a
whitespace-tolerant output check. On a script case, `args`, `expect`, `object` and a
target are refused. A case with no target, no `meta.function` and no `script = true`
is refused.

### D7 — Evidence: observation, then verdict, then owner

The executor returns a `UnitObservation`. `runner/judge.rs` turns it into `CaseResult`s.
Fault and cause are derived in Rust from three things: which call the plan says was
running (the first call without a record), what kind of code that call runs (D3), and
how the process ended.

`CaseResult` gains three optional fields (`#[serde(default)]`, absent in older results):

```rust
pub fault: Option<Fault>,          // Student | Teacher | Environment; None when Passed
pub cause: Option<Cause>,          // why, in one word — for P-677, without parsing messages
pub stdout: Option<String>,        // what the student printed during the call, when non-empty
pub input: Option<CaseInput>,      // {target: {requested, resolved}, args (with $refs by name), stdin}
```

| What happened | status | fault | cause |
| -- | -- | -- | -- |
| no file matched `meta.file` | `Missing` | student | `no_file` |
| target function / method / attribute not found | `Missing` | student | `no_target` |
| wrong value, wrong/missing exception, stdout or file mismatch, checker said no | `Failed` | student | `wrong` |
| in-process checker raised `AssertionError` | `Failed` | student | `rejected` |
| raised an exception nobody expected | `Error` | student | `raised` |
| student file has a syntax error / raised while loading | `Error` | student | `syntax` / `load` |
| the return value could not be serialised | `Error` | student | `unserialisable` |
| a call exceeded its timeout | `Timeout` | student | `timeout` |
| process died (signal, deadline kill, or exit without `done`) during a student call | `Timeout` if deadline/SIGXCPU, else `Error` | student | `killed` |
| not run: an earlier call in the unit hung or killed the process | `Error` | student | `not_run` |
| not run: a `$ref` / `object` it needs was never produced | `Error` | student | `dependency` |
| not run: a setup call failed (raised, timed out, missing target, killed) | `Error` | owner of that setup call: student for `function`/`method`, teacher for `teacher` | `setup` |
| two records for one call (tampering) | `Error` | student | `protocol` |
| teacher module failed to import in a unit (it passed `prepare`) | `Error` | teacher | `teacher_import` |
| a checker could not decide: Rhai error, checker script crash / bad output, in-process checker raised (not `AssertionError`), timed out, or returned neither `bool` nor `(bool, str)` | `Error` | teacher | `checker` |
| nothing was judged | `Error` | teacher | `nothing_to_judge` |
| spawn failed; died before `ready` with no teacher imports; harness crashed; a record did not parse; checker script could not spawn | `Error` | environment | `spawn` / `harness` |

The judge applies, in order:

1. the outcome against `expect_error`, matched against the exception's MRO names, so a
   `ValueError` subclass satisfies `expect_error = "ValueError"`;
2. the value check: the case's `check`, or exact against `expect`;
3. `expected_stdout` (exact; fails if the capture was truncated);
4. `expect_files`.

A case passes only if every expectation it declared passes. Exact comparison treats
numbers numerically (`2 == 2.0`), as Python does. `Checker::check` returns
`Result<CheckOutput, CheckError { fault, message }>`: spawn and wait failures are
environment, and everything else a checker does wrong is teacher. `CheckInput.context`
carries the observation, `{stdout, files}`.

### D8 — Checking is declared on the case, never bound by name

`@checker` name binding is removed, along with the `checker` builtin. An in-process
checker judges a case only when the case says `check = { function = "name" }`. `name`
must be a function exported by a teacher module. It is called as
`fn(result, expected, **deps)`: `expected` is the case's `expect`, and each further
parameter is filled from the names in scope, with `stdout` also available. Teacher
modules that use `@checker` stop importing, with a pointer to the docs. The two real
specs that relied on binding (one had an always-true checker overriding exact cases)
need `check = { function = … }` on the cases meant to use it.

### D9 — A checker that cannot decide is the teacher's, and the common trap is refused first

A Rhai expression is compiled at load with strict variables, so a misspelled name is
refused. In `prepare`, each Rhai check is also evaluated against `result = ()` and,
when the case has one, against its own `expect`. An error there is refused before any
student runs, with a suggested guard (`result != () && result.len() >= 1`). A student
who returns `None` is the most common wrong answer, and a check that cannot judge it is
ambiguous: that is the teacher's to resolve, not the grader's to guess.

At run time a checker that still cannot decide is `Error` / teacher / `checker`.
Attributing the error to the student would let a teacher typo such as `result.lenght()`
(which compiles, and fails exactly like `().len()`) quietly zero a class. An
in-process checker that wants to fail a malformed answer returns `False`, or raises
`AssertionError`, which is `Failed` / student / `rejected`.

### D10 — Load, prepare, run

```text
load_spec(path)       -> Result<TestSpec, SpecError>          static: shape, fields, paths
prepare(specs, exec)  -> Result<Vec<Bundle>, PrepareError>    once per run; runs teacher code
run_all(students, bundles, exec, opts) -> Vec<StudentReport>  per student, per unit
```

**`load_spec`**: serde (`deny_unknown_fields` everywhere) reports the first shape error
with its line and column. `check` has a hand-written `Deserialize`, so an unknown key
such as `regex`, `exec` or `wasm` is named. Every semantic problem in the file is then
reported together. It refuses:

- `language` other than `python`; `compile`;
- an unknown builtin (in the shorthand, `builtin`, or `oracle.check`); an expect-dependent builtin as `oracle.check`;
- a detailed `check` naming zero or several kinds; `tolerance` without `approx`;
- `exact`, `approx`, `set_eq`, `contains` or `text` without an `expect` (or, in script
  mode, without `expected_stdout`);
- `check` together with `expect_error`; `expect` together with `expect_error`;
- a Rhai expression that does not compile; a Python checker script that does not exist;
- `null` anywhere inside `expect` (TOML has no null, so it can only come from
  `inf`/`nan`);
- an oracle naming several kinds, or `python`;
- `setup.file`; a duplicate setup id; a setup step without a target or with two;
- `id` on a case; `object` not naming an earlier student-produced id; `attribute` with
  `args`; a `teacher` call outside setup, or taking a student-produced id;
- a scenario with no steps, a parametrized step, or a script step;
- the script-mode rules in D6; top-level setup alongside script cases;
- missing `data_files`; `expect_files` paths that are absolute or contain `..`;
- a zero timeout; a spec with no cases and no scenarios.

Every teacher path (`imports`, `data_files`, Python checker scripts, oracle `reference`)
is resolved against the spec's directory and made absolute. `TestSpec` stays
`Deserialize`, so tests can build one, but `prepare` re-runs the same validation, and
`run_all` takes only `Bundle`s. There is no unvalidated path to execution.

**`prepare`** runs teacher code once per bundle:

- it imports the teacher modules in a unit-shaped sandbox (a fresh cwd with
  `data_files` staged, the same interpreter and environment) and records their exports;
- it expands parametrized cases, as today, and resolves oracles (D11);
- it dry-runs the Rhai checks (D9).

It refuses, all problems together:

- a teacher module that fails to import;
- an unknown `$ref`; a name bound twice (D3);
- a `teacher` or `check.function` name the modules do not export;
- an in-process checker parameter that names nothing in scope for that unit;
- a case or step with nothing to judge: none of `expect`, `expect_error`,
  `expected_stdout`, `expect_files` or `check`, after oracle resolution;
- duplicate case names after expansion.

The CLI refuses to grade on a `PrepareError`, the stance `build_local_input` already
takes on input errors.

**`run_all`** is generic over `E: Executor` and takes `Arc<E>` and `Arc<[Bundle]>`:

- the student file is matched once per (student, bundle), before any unit is
  dispatched, with the hint set to `meta.function`, else the first declared `function`
  target (today's chain rule), and every unit of that bundle uses the same file;
- each unit is one task holding one permit of the existing semaphore; judging and lint
  run under `spawn_blocking`;
- a panicking unit task makes that unit's cases `Error` / environment and leaves the
  rest alone;
- results come back in declaration order: cases, then scenarios.

### D11 — The reference implementation runs through the same interface

`oracle.reference` becomes a one-call `UnitPlan` for the `Reference` subject: the
reference file at its spec-relative absolute path (no student-file matching), exact
function lookup (no `_fuzzy_lookup`), `--timeout`, and a fresh unit directory with
`data_files` staged. `prepare` receives the raw `CallObservation`. A reference that
does not return a value is a `PrepareError` naming the case: the smallest form of
P-676's rule, and it keeps the null-expectation path closed. A Rhai oracle's result is
converted totally (arrays and maps included); `()` or a conversion failure is a
`PrepareError`. P-676 then owns provenance, versioning, priority and answers for
stdout and files, all on this interface.

### D12 — The interface types

As built (`runner/executor.rs`, `runner/prepare.rs`, `runner/orchestrator.rs`):

```rust
pub trait Executor: Send + Sync + 'static {
	fn language(&self) -> &str;
	/// Pick the student's file for a spec. P-673 owns the rule.
	fn locate<'a>(&self, files: &'a [StudentFile], spec: &TestSpec) -> Option<&'a StudentFile>;
	/// Import teacher modules in a unit-shaped sandbox and report what they export.
	fn inspect(&self, spec: &TestSpec, timeout_secs: u64) -> impl Future<Output = Result<TeacherRuntime, String>> + Send;
	fn run(&self, plan: &UnitPlan) -> impl Future<Output = UnitObservation> + Send;
}

pub struct UnitPlan {
	pub subject: Subject,                 // Student | Reference — decides lookup (fuzzy | exact)
	pub file: PathBuf,                    // copied into the unit's cwd before running
	pub script: Option<ScriptRun>,        // {stdin, timeout, files}; None = call mode
	pub imports: Vec<String>,             // absolute teacher module paths
	pub vars: Arc<BTreeMap<String, Value>>,
	pub data_files: Vec<(PathBuf, String)>, // (source, relative destination)
	pub allowed_imports: Vec<String>,
	pub load_timeout: u64,
	pub setup: Vec<CallPlan>,
	pub steps: Vec<CallPlan>,
}
pub struct CallPlan {
	pub target: Target,                   // Function{name} | Method{object,name} | Attribute{object,name} | Teacher{name}
	pub args: Vec<Value>, pub stdin: Option<String>, pub timeout: u64,
	pub id: Option<String>, pub files: Vec<String>, pub check: Option<InProcessCheck>, // {function, expected}
}
pub struct UnitObservation {                            // Serialize + Deserialize
	pub ready: bool,
	pub load: Option<CallObservation>,
	pub setup: Vec<CallObservation>,                    // in plan order, a prefix of the plan
	pub steps: Vec<CallObservation>,
	pub checks: BTreeMap<usize, CheckObservation>,      // Verdict | Rejected | Error
	pub fatal: Option<Fatal>,
	pub done: bool,
	pub exit: Exit,                                     // Code | Signal | Deadline | Spawn
	pub protocol_error: Option<String>,
	pub stderr: String,                                 // tail, for diagnosing a crash
}
pub struct Bundle {                                     // Serialize
	pub spec: TestSpec,                                 // validated, paths absolute, cases expanded, oracles resolved
	pub teacher: TeacherRuntime,                        // exports and their parameters
	#[serde(skip)] pub units: Vec<Unit>,                // derived: {plan, scored: [(name, case)]}
}
pub struct RunOptions { pub concurrency: Option<usize>, pub python: String } // python runs checker scripts
```

`vars` and `parametrize.args` become `BTreeMap`, so a `Bundle` serialises
deterministically and P-678 can digest it. P-675 and P-676 add provenance *to* `Bundle`
cases rather than building a parallel type.

### D13 — Spec shapes in use today, and how they are spelled now

Shapes, not content: no course material is reproduced here.

| Shape | Today | Now | Result changes on regrade? |
| -- | -- | -- | -- |
| fixed args / expect against `meta.function` | per-case | unchanged | only if the student printed (now passes) or imported a stdlib module (now loads) |
| several functions in one file, `function` per case, no imports | chain | unchanged spelling, now independent; file match uses the same hint | guard now on during calls; `input()` in a call raises `EOFError` |
| a student loader called in top-level setup, its result passed as `$id` to many cases | chain, deep-copied | unchanged spelling: re-run per case (top-level setup) | no, beyond the above |
| setup values feeding a teacher checker through parameter names | chain + `@checker` binding | `check = { function = … }` on those cases | yes — cases that were silently judged by the bound checker are now judged by what they declare |
| a teacher module exporting a data path used as `$NAME` | chain | unchanged | no |
| Rhai checks calling methods on / indexing `result` | chain or per-case | guard with `result != () && …` (refused in `prepare` until then) | a student returning `None` is `Failed`, not an error |
| stdin → stdout scripts | IO mode | `script = true` | the guard now applies; a crash after correct output is `Error` |
| parametrized with Rhai / reference oracle | per student | unchanged spelling; expanded and resolved once | no |

Specs with `copy_refs`, `setup.file` or `@checker` stop loading or preparing, with
messages that name the fix.

## Touch list

| File | Change |
| -- | -- |
| `models/spec.rs` | `deny_unknown_fields`; `Scenario`; `method`/`attribute`/`object`/`teacher`; `script`; `data_files`, `expect_files`; hand-written `CheckMethod` deserializer; `BTreeMap`s; delete `copy_refs` |
| `models/result.rs` | `Fault`, `Cause`, `CaseInput`; the four new `CaseResult` fields; `Default` |
| `spec_loader.rs` | path resolution; static validation (`validate`); `SpecError::Invalid`; `load_spec_str` for tests |
| `runner/executor.rs` | the trait and D12's types |
| `runner/prepare.rs` (new) | `Bundle`, `TeacherRuntime`, `prepare`, `PrepareError` |
| `runner/judge.rs` (new) | observation → `CaseResult` |
| `runner/harness.py` (new) | the one harness |
| `runner/python.rs` | `PythonExecutor: Executor`; payload file, spawn, deadline, record parsing, kill-then-drain; old helpers deleted; `find_student_file_with_hint` stays here |
| `runner/sandbox.rs` | CPU limit per unit |
| `runner/orchestrator.rs` | generic; unit tasks; matching once per bundle; no chain switch |
| `runner/oracle.rs`, `runner/expander.rs` | called from `prepare`; reference through `Executor::run`; total Rhai conversion; expander keeps every non-parametrize field |
| `runner/resolve.rs` | deleted — `$ref`s resolve in the harness |
| `checker/*` | `Result<CheckOutput, CheckError>`; `context` carries the observation; numeric exact equality |
| `main.rs`, `scriptmark-py/src/lib.rs` | `prepare` before `run_all`; bindings' `load_spec` through `spec_loader`; `num_scenarios` |
| `display.rs`, CSV archive | show fault and cause; CSV gains trailing `fault`, `cause` columns |
| `grading.rs`, `db/mod.rs` tests | `CaseResult` literals gain `..Default::default()` |
| `Cargo.toml` | `tempfile` becomes a runtime dependency |
| `tests/integration.rs` | rewritten against the new contract |
| `examples/bundles/{pure_function,shared_object,file_io}/` | one bundle each, with a correct and a wrong student |
| `docs/test-bundles.md` (new), `README.md` | teacher-facing contract and migration notes; drop the non-existent `regex` checker |

## Tests

Mapped to the ticket's acceptance lines.

**Runs with a complete teacher bundle alone.** Every example bundle grades end to end
with no `parametrize`, oracle or seed. The correct student passes everything, and the
wrong one fails exactly the cases it should, with the expected causes.

**Pure function, shared object, file read/write.** One integration test per example
bundle:

- **Pure function:** `expect` / `expect_error`, with a student that imports `random`
  and `csv` and prints inside the graded function, and still passes.
- **Shared object:** a class built in scenario setup, driven by `method` steps, with a
  later `attribute` step observing state an earlier step left.
- **File read/write:** a function reading a staged data file, through both a cwd-relative
  path and `Path(__file__).parent`, and another satisfying `expect_files`. Two
  concurrent units write the same filename without interfering, and nothing is written
  to the grader's cwd or to the submission directory.

**Imports do not change isolation.** The same spec with and without an `imports` line:
a student function that increments a module global returns `1` in every case both times.
The same holds for a case-level `function` override.

**Setup count, timeout, stdout, error origin are verifiable.**

- **Setup count:** setup calls append a line to an absolute path passed through `vars`.
  Top-level setup writes one line per unit, scenario setup one per scenario.
- **Timeouts:** a looping step is `Timeout` while its neighbours keep their results, and
  so is one wrapped in a bare `except:`. A case's own `timeout` overrides the default.
  A scenario of four 10-second hangs is four `Timeout`s, with no SIGXCPU death.
- **stdout:** a printing student's output is in `stdout`. A script's `input(prompt)` and
  `sys.stdin.read()` both see the case's stdin.
- **Error origin:** each row of the D7 table has one test.

**Refused before running.** One test per refusal in D10 (load and prepare), each
asserting the message names the problem and that no student ran.

**Unit tests:**

- spec validation table;
- judge table (observation → status, fault, cause);
- record parsing: noise lines, a record glued to a student's partial line, a stream cut
  short after a kill, a missing `done`, a forged line with the wrong nonce, and a
  duplicate record;
- the serialiser's edge cases.

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

## Code review corrections

The implementation (`1a2bb21..7dd3b9e`) went through a six-lens adversarial code review,
each lens with its own refuter: harness, judge, contract, Rust, tests, conformance. About
80 findings survived. Everything below is fixed and has a test. Where the design above
now reads differently from the code, the code and this section are right.

**Student code handled outside a call crashed the harness, and the whole unit was blamed
on the environment.** There were six ways in:

- a `@property` or `__getattr__` that raises, which runs during method or attribute
  lookup;
- an exception whose `__str__` raises;
- an integer too large for the grader's JSON, which was read as tampering, or above
  4300 digits, which crashed the harness;
- a lone surrogate on the record channel;
- `expect_files` hitting a path the student had blocked;
- a `.PY` extension.

Now the whole call runs inside `call()`, under its timer and guard: the lookup, the
body, and serialisation.

- A property is read once. `getattr_static` decides between "missing" and "raised".
- `error_of` and the serialiser are total. Integers outside the 64-bit range are
  `{"$bigint": "…"}`.
- The channel replaces unencodable characters.
- `observe_files` treats any `OSError` as "not the file asked for".
- The student file loads through an explicit `SourceFileLoader`.

**A broken stream no longer erases what arrived.** Only a record that repeats or comes
out of order, which is somebody else writing, discards the unit as `protocol`. A harness
crash after `ready`, or a record that does not parse, keeps every recorded call, and only
the unreported ones become `environment` / `harness`. That matches the D7 row "a record
did not parse". Other records are now also refused:

- a `teacher_import` fatal after `ready`;
- a repeated `ready`, `done` or `fatal`;
- anything after `done`;
- more than 32 MiB on the channel.

**Checker dependencies.** A parameter whose producing step failed no longer arrives as
`None`, which blamed the teacher's checker. It is `student` / `dependency`. Parameters are
reported with their kind and default: `**kwargs` and defaulted parameters are filled only
when named. `stdout` is always the call's output, and a var, export or id called `stdout`
is refused. A name two teacher modules export differently is refused, naming both modules.
`@checker` fails with the instruction to write `check = { function = … }`.

**Refusals added in `load_spec`:**

- a function checker on a script case (it had failed every student as `environment`);
- `check` with `oracle.check`, which had silently replaced it;
- `expect_error` with a reference or Rhai oracle;
- parametrize on an attribute; a reference oracle on a method;
- an `expect` of the wrong shape for `approx`, `set_eq` or `text` (`text` had passed
  any non-string);
- `inf`/`nan` in `args`;
- `expected_stdout` longer than the capture;
- timeouts outside 1–86400 s.

**Refusals added in `prepare`:** an empty spec set, a zero default timeout, and a
reference implementation that returns `None`.

**Script mode.**

- A syntax error is `syntax`.
- Truncated output fails explicitly.
- A script that exits with the expected error must still print `expected_stdout`.
- `sys.argv` is the script's own.
- The capture is a real `TextIOWrapper`, so `sys.stdout.buffer` and `reconfigure` work.
- A clean exit is judged by the exit code, not its text.

**Process.**

- The harness runs in its own process group, and the whole group is killed at the
  deadline and after exit.
- Pipes are read into bounded, shared buffers, so a drain timeout keeps what arrived and
  stderr keeps only its tail.
- `PYTHONSAFEPATH=1` and an explicit `sys.path` scrub mean a submission named `json.py`
  cannot shadow the harness's imports.
- Teacher module directories are appended to `sys.path`, not prepended.
- RLIMIT_CPU's hard limit sits one second above the soft one, so Linux sends SIGXCPU,
  not SIGKILL. D6's "can never fire first" overstated it: the CPU limit is a backstop
  for one busy core, and multi-threaded native code can still reach it early.
- Deadline arithmetic saturates.
- Rhai runs with an operation limit, so a looping expression is a checker error, not a
  hang.

**Design points restated.**

- Sets serialise in natural sorted order, falling back to JSON-text order only for mixed
  types. D5's "sorted by their JSON text" would put 10 before 9.
- A setup failure is always `error`, never `timeout`.
- `not_run` inherits the fault of the call that stopped the unit.
- The file-matching hint is the chain rule as it actually was: the first function a case
  names, then `[meta] function`. D10 had them reversed.
- The reference implementation runs as teacher code, outside the import guard, as D5
  says teacher code does.
- Exact comparison of integers is exact: `2**63` is not `2**63 - 1`.

**Evidence.** `TestResult.file` records which student file was graded. A blanket result
still names the target it asked for. The terminal failure listing shows the cause.

**CLI and bindings.** `--timeout` accepts 1–86400. `--concurrency` must be at least 1.
In the Python bindings, an invalid bundle is a `ValueError` from every entry point, and a
missing path is `FileNotFoundError`.

**Left for their owners.**

- An unparseable generator expression still yields `null` arguments. That belongs to
  P-675, which makes generation errors a preparation failure.
- A function checker that trips over a student object's own methods is still the
  teacher's `checker` error, by design (D9). Attributing it by traceback frame is a
  follow-up.
- Reference oracles still resolve one at a time; making them concurrent belongs to P-676.

### Re-review of the fix commit

A narrow two-lens re-review of `972f42e`, each lens refuted, found ten more issues.
They are all fixed and tested:

- **A student who re-wraps, detaches or closes `sys.stdout` crashed the harness.**
  Re-wrapping is the usual UTF-8 idiom. The capture's bytes are now held apart from the
  wrapper, cannot be closed, and are flushed in the student's own scope.
- **Past 4300 digits, `$bigint` dropped the sign and the digits.** It is now exact
  decimal, or `hex()` beyond Python's digit limit.
- **`error_of` still ran student code.** It called an exit code's `__eq__` and a
  `__str__` that raises `BaseException`. `exit(0.0)` also counted as clean.
- **The syntax check wrote `__pycache__`** into the script's directory. It now compiles
  in memory.
- **Equal constants in two teacher modules counted as duplicates.** They are now
  compared by equality, not identity.
- **The `sys.path` scrub stopped students importing staged helper modules.** The work
  directory now goes back on the path, after the standard library.
- **A legitimately large return value was taken for tampering.** A value over 4 MiB is
  now `{"$too_large": …}`, still returned, and still judged live by a function checker.
  A flooded channel keeps the records that arrived and blames the student for the rest.
- **In script mode, truncated output hid a timeout or crash.** The truncation check now
  applies only once the script has finished.
- **A student thread printing between calls could garble a record,** and the
  environment would be blamed. Records now have a pipe of their own: fd 3 in the
  harness, created by the grader. The student's stdout is `/dev/null` at the process
  level, and a capture during each call. Nothing a student does to stdout — rewrapping
  it, writing to `sys.__stdout__` or fd 1, printing from a thread — can reach the
  records. A test floods `sys.__stdout__` and fd 1 from a thread while large records are
  written; it fails on the old shared channel. Windows builds keep the nonce-framed
  stdout channel, because inheriting a handle there is not portable.
- **Ctrl-C no longer reached the units** once they had their own process groups. The CLI
  now kills every live unit group on interrupt, and a guard kills a unit's group if its
  run is dropped.

