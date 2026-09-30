# Writing a test bundle

A test bundle is a directory of TOML specs, plus any teacher modules and data files they
name. Each spec is one grading item. ScriptMark checks the whole bundle before it runs a
single student: anything it cannot honour is an error then, not a zero later.

Runnable examples, each with a correct and a wrong student:

| Fixture | Example |
| -- | -- |
| a pure function | [`examples/bundles/pure_function`](../examples/bundles/pure_function) |
| a shared object | [`examples/bundles/shared_object`](../examples/bundles/shared_object) |
| reading and writing files | [`examples/bundles/file_io`](../examples/bundles/file_io) |

```sh
scriptmark grade examples/bundles/shared_object/submissions -t examples/bundles/shared_object/tests
```

## Cases and scenarios

```toml
[meta]
name = "account"          # the grading item
file = "bank.py"          # which student file to test
language = "python"
function = "deposit"      # optional: the default target for cases that name none

[[cases]]                 # independent: its own process and working directory
name = "deposit"
args = [50]
expect = 150

[[scenarios]]             # shared state: steps run in order in one process
name = "ada's account"
[[scenarios.steps]]
name = "..."
```

A **case** is judged alone. Nothing survives from one case to the next: not module
globals, not objects, not files. A **scenario** is for tests that need state to carry
over. Each step is reported as its own case, named `"<scenario> / <step>"`.

Nothing else changes how a test runs. `imports`, a per-case `function`, `vars` and
`data_files` never turn independent cases into shared ones.

## What a call runs

| Fields | Calls |
| -- | -- |
| `function = "f"` | a function (or class) in the student's file |
| `method = "m"`, `object = "id"` | a method of an object an earlier student call produced |
| `attribute = "a"`, `object = "id"` | reads an attribute of that object |
| `teacher = "t"` | a function in a teacher module — setup only, never scored |
| `script = true` | runs the student's file as a program: stdin in, stdout out |

`args` may use `"$id"` for a value named in `[vars]`, exported by a teacher module, or
produced by setup or an earlier step with `id = "..."`. Write `"$$5"` for the literal
string `"$5"`. A name that means nothing is an error before grading.

## Setup

Where you write setup is its scope, and it runs exactly once there:

| Written | Runs |
| -- | -- |
| top-level `[[setup]]` | at the start of **every** case and **every** scenario, inside its process |
| `[[scenarios.setup]]` | once, at the start of that scenario |

```toml
[[setup]]
id = "data"
function = "load_data"    # the student's loader
args = ["$DATA_FILE"]

[[cases]]
name = "lookup"
function = "find"
args = ["$data", 3]
expect = "A"
```

Each case gets a fresh `data`: one case mutating it cannot disturb another. If you want
calls to share and mutate one object, write a scenario.

A setup call that fails stops the unit it runs in: every case of a top-level `[[setup]]`,
every step of a scenario. The failure is the student's when setup called student code, and
the teacher's when it called a `teacher` function.

## Expectations

| Field | Passes when |
| -- | -- |
| `expect = v` | the call returns `v` (numbers compare numerically: `2 == 2.0`, exactly — no float rounding) |
| `expect_error = "ValueError"` | the call raises it, or a subclass of it |
| `expected_stdout = "..."` | the call printed exactly this |
| `expect_files = { "out.txt" = "..." }` | the call left this file with this content |
| `check = ...` | the checker says yes |

A case must declare at least one, and passes only if every one it declares holds. A
checker that compares against `expect` decides how it holds: `approx`, `set_eq`,
`contains`, `text`, a Rhai check that uses `expected`, and every Python or function
checker, which always receive it. Beside any other check — `sorted`, or a Rhai check that
never mentions `expected` — the value must also equal `expect` exactly.

What a returned value looks like to `expect`: tuples are lists, sets are lists in sorted
order (mixed types fall back to a stable order), dict keys are strings, and an integer too
large for a 64-bit number is `{ "$bigint" = "123…" }` — its exact decimal digits, or
`hex()` past Python's 4300-digit limit. A value whose JSON would take more than 4 MiB of
UTF-8 is reported
as `{ "$too_large" = "… bytes of JSON" }`. Judge either with a function checker, which
always gets the real value.

## Checkers

| Spelling | |
| -- | -- |
| `check = "approx"` (with `expect`) | `exact`, `approx`, `set_eq`, `contains`, `text` compare against `expect`; `sorted` checks order; beside `expect`, the value must also equal it |
| `check = { builtin = "approx", tolerance = 0.01 }` | |
| `check = { rhai = "result != () && result.len() > 2" }` | `result`, `expected`, `context.stdout`, `context.files` |
| `check = { python = "verifiers/check.py" }` | reads `{result, expected, context}` as JSON on stdin; prints `{"pass": ..., "message": ...}` |
| `check = { function = "is_history_of" }` | a teacher-module function run on the live value |

A **function checker** takes `(result, expected)` and then, by name, whatever else it
needs: `def chk(result, expected, acct)`. Each further parameter is filled from the names
in scope (`vars`, teacher exports, setup and step ids) — the live objects, as the call
left them — and `stdout` is always the call's output. A parameter with a default, or
`**kwargs`, is filled only when its name is in scope. Return `True`/`False` or
`(bool, message)`; raise `AssertionError` to reject a malformed answer. Checkers apply only
where a case names them, and never on a script case (a script has no returned value).
If the step that produces a parameter failed, the case is the student's `dependency`
error, not a checker crash.

**A checker that fails on the student's answer rejects it.** A Rhai error, a checker
script that crashes, times out or prints no verdict, or a function checker that raises
anything — each on this student's value — marks the case `failed` with fault `student`
and cause `checker`: a `None` reaching `len()` is a wrong answer. Only teacher code that
cannot load, or a checker still running when the unit stopped, is yours. When every
student fails an item through its checker, grading prints a warning naming it: that is
usually a bug in the checker, not a class that got it wrong. Rhai checks are dry-run
against `expect` before grading, and a guard keeps a wrong answer a plain `wrong`:

```toml
check = { rhai = "result != () && result.len() > 2" }
check = { rhai = "type_of(result) == \"array\" && result.contains(5)" }
```

A Rhai expression may run at most a million operations; one that loops past that on an
answer has rejected it, like any other checker failure.

## Files

```toml
[meta]
data_files = ["data/poem.txt", "fixtures/"]   # copied into every unit's working directory
```

Every case and every scenario runs in its own temporary directory with your data files
staged in it and the student's file copied beside them. `open("data/poem.txt")` and
`Path(__file__).parent / "data" / "poem.txt"` both work. Relative writes land in that
directory, which is removed afterwards; `expect_files` reads them back. Each unit also
gets a `HOME` and a `TMPDIR` of its own there, so nothing one unit leaves behind reaches
another. Python runs isolated (`python -I`): user site-packages and `PYTHON*` environment
variables are ignored, so install what your modules need into the interpreter itself, or
into a virtual environment you pass with `--python`. Neither the
working directory nor the import allowlist is a security boundary; see
[What grading does not defend against](#what-grading-does-not-defend-against).

Teacher modules are loaded from the bundle, not the working directory. Find your own
files with `Path(__file__).parent`, not a relative path; a module may import a sibling
module from its own directory. A reference implementation runs as teacher code: the
import allowlist does not apply to it. Student code can import a `.py` file you staged
through `data_files`, if its name is in `allowed_imports`; the standard library always
wins over a file of the same name.

What a student prints outside any call — from a thread they left running, say — is
discarded. Only output during a call is evidence. Results travel on a channel of their
own, so nothing a student does with stdout can disturb them.

## Timeouts

`--timeout` applies to every call: loading the student's file, each setup call, each case
and step, and each function checker. `timeout = n` on a case, step or scenario
overrides it; every timeout is between 1 and 86400 seconds. A step that times out is
reported as `timeout`, and later steps still run. A unit that stops answering altogether
is killed when all its calls' timeouts have passed: the call it was in is `timeout` /
`killed`, and the calls after it are `not_run`.

## What each result means

Every case that does not pass says whose it is (`fault`) and why (`cause`):

| status | fault | cause | meaning |
| -- | -- | -- | -- |
| `failed` | student | `wrong` | a value, exception, output or file was wrong |
| `failed` | student | `rejected` | a function checker raised `AssertionError` |
| `failed` | student | `checker` | a checker failed on the answer: raised, crashed, timed out, or gave no verdict |
| `error` | student | `raised` / `syntax` / `load` / `unserialisable` | the student's code crashed |
| `timeout` | student | `timeout` / `killed` | it ran too long |
| `error` | student | `killed` | the process died during the call (a signal) |
| `missing` | student | `no_file` / `no_target` | no file, function, method or attribute to call |
| `error` | student | `dependency` | a value it needs was never produced |
| `error` | student or teacher | `setup` | a setup call failed: the student's or the teacher's, by who owns the call |
| `error` | the culprit's | `not_run` | an earlier call in the unit hung or killed the process |
| `error` | student | `protocol` | the unit's records were forged or repeated |
| `error` | teacher | `checker` / `teacher_import` / `nothing_to_judge` | the bundle's fault: a checker still running at the deadline, a module that failed to import, a case with nothing to judge |
| `error` | environment | `spawn` / `harness` | the machine's, or the grader's own, fault |

An error message, or a function checker's message, longer than 65,536 characters is cut,
and says how long it was.

## Scoring

Each test spec is a **grading item**, identified by its `[meta] name`. An item is worth
its declared points however many cases it runs: adding fifty random cases to a question
does not make it weigh more. `assignment.toml`, beside the tests directory, declares the
items and the policy; without it, every spec is an item worth 1 point.

```toml
[[items]]
id = "stats"                  # the spec's [meta] name
title = "Mean and clamp"      # optional, for people
points = 10                   # default 1
aggregation = "proportional"  # required: "proportional" or "all_or_nothing"

[grading]                     # optional; these are the defaults
missing = "withheld"
missing_file = "withheld"
scale = 100
decimals = 2
curve = { kind = "raw" }
```

- **An item's score.** `proportional` is `points × passed / cases`, the item's pass
  rate. `all_or_nothing` is `points` when every case passed, else 0.
- **The grade.** `score` is the sum of item scores; `max` is the sum of points. The raw
  grade is `score / max × scale`. A declared `curve` maps that fraction onto the grade —
  `template` (`linear`, `sqrt`, `log`, `strict`, between `lower` and `upper`), or
  `formula`, a Rhai expression over `score`, `max`, `fraction` and `scale`. The raw grade
  is kept beside a curved one, in every output.
- **Rounding.** The raw and final grades are rounded half away from zero to `decimals`
  places, once. Item scores are kept unrounded in the JSON results.
- **Lint** counts only through `lint_points = N`, which adds `N × lint score / 100` to
  the score and `N` to the max. A linter that exits outside `[lint] ok_exit_codes`
  (default `[0, 1]`) has failed, and is never read as a clean file.

### Zero or no grade

A grade is a number, or withheld with a reason. There is no partial total: a grade
missing one item would reach Canvas looking like a real low grade.

| Reason | When | Grade |
|---|---|---|
| `excused` | the source excused the student | withheld, whatever the policy |
| `grading_task_failed` | grading this student failed outright | withheld |
| `pending_review` | the submission matched no roster student | withheld |
| `not_submitted` / `submitted_empty` | nothing, or only empty files, arrived | withheld; 0 under `missing = "zero"` |
| `missing_file` | a submission lacks an item's file | withheld; that item 0 under `missing_file = "zero"` |
| `teacher_fault` | a case failed through the bundle's fault | withheld |
| `environment_fault` | a case failed through the machine's or grader's fault | withheld |
| `formula_error` | the formula failed, or gave a value outside `0..=scale`, for this student | withheld, never clamped |
| `lint_failed` | the linter did not run properly, with `lint_points` declared | withheld |

A 0 comes only from the student's own evidence, a `missing` or `missing_file` policy —
recorded with its reason, so it never reads as wrong answers — or a declared curve.
Withholding one student never stops the others being graded.

Everything that makes a policy unusable is refused before a student runs: an unknown key,
an item declared twice, an item with no spec or a spec with no item, two specs with one
name, 0 points in total, a bad `scale`, `decimals` or curve bound, a formula that does not
compile, `lint_points` without a `[lint]`.

After grading, an item every student failed through its checker, or where no student
handed in the file, is flagged: that is usually a bug in the bundle, not the class.

### Outputs

The JSON results carry each student's `grade`: its `state`, `reason`, `score`, `max`,
`raw_grade`, `final_grade`, every item's score, and the policy it was reached under.
`grade --archive` also writes `grades_<tests>.csv`, one row per student, where a withheld
grade is an empty cell and a zero is `0`. `grades-push` sends graded students only — real
zeros included — and says how many it skipped and why. `summarize`, `report`, the TUI
and the database show what `grade` stored; none of them re-scores. Results and databases
written before per-item grading are refused, not reinterpreted.

## What grading does not defend against

Every unit runs in its own process and directory, and its results travel on a channel of
their own. That keeps accidents contained: stray output, a crash, a hang or a leftover
file cannot touch another case or another student. It is not a sandbox against a student
who sets out to beat the grader. Student code runs as your user, in the same process as
the harness that records its calls and as your function checkers, so it can:

- read the bundle, expected values included, by absolute path, or import past the
  allowlist with `importlib`;
- reach the harness through the interpreter and forge the records of its own calls,
  a passing function checker's verdict included.

Read the submission behind a result that surprises you, and grade on a machine or
account where a student's code can do no harm.

## Refused before grading

These are errors when the bundle is loaded or prepared, before any student runs:

- unknown fields (`expected = 5` for `expect`);
- an unknown checker; a check naming two kinds;
- `approx` without `expect`;
- `check` together with `expect_error`;
- a case with nothing to judge;
- `inf` or `nan` in `expect`;
- a language other than Python;
- an `object` no earlier student call produced;
- an unknown `$name`; one name meaning two things;
- a teacher module that fails to import;
- a checker function that does not exist or asks for a name that means nothing;
- a Rhai check that cannot judge `None`;
- a function checker on a script case; `check` together with `oracle.check`;
- `sorted` on a script case, whose value is text; `sorted` beside an `expect` that is
  not itself sorted, which no value could pass; `oracle.check` together with
  `expect_error`;
- an `expect` of the wrong shape for its checker (`approx` needs a number, `set_eq` an
  array, `text` a string);
- `inf` or `nan` in `args`; `expected_stdout` longer than the 64 KiB of output kept;
- `stdout` as a var, export or id (checkers reserve it);
- a name two teacher modules export differently;
- a tests directory with no specs;
- a reference implementation that does not return;
- missing data files.

## Migrating an older spec

| Was | Now |
| -- | -- |
| `@checker("f")` in a teacher module | `check = { function = "check_f" }` on each case meant to use it — decorating no longer binds, and importing a module that still uses it fails with that instruction |
| `copy_refs` | delete it: every case already gets fresh values |
| `setup.file = "gen.py"` | put fixed data in `[vars]`, `data_files` or a teacher module |
| stdin → stdout cases with no function | add `script = true` |
| a Rhai check like `result.len() > 0` | `result != () && result.len() > 0` |
| `[lint] weight = 0.1` | delete it, and give lint points with `[grading] lint_points` |
| `grade -g sqrt --range 60,100` or `--formula` | `[grading] curve = { kind = "template", name = "sqrt", lower = 60, upper = 100 }` or `{ kind = "formula", formula = "..." }` |

Results can change on regrading. Students who printed inside a graded function, or who
imported an allowed module such as `random` or `csv`, used to fail every case and now
pass. Cases that a name-bound `@checker` used to judge silently are now judged by what
they declare. A case that paired `sorted` with `expect` used to pass any sorted list; the
value must now equal `expect` too.

Grades change too. They were a curve — `sqrt` by default — over the pass rate of every
case pooled, so an item with more cases weighed more. They are now the raw
`score / max × scale` over declared items, and a curve applies only when declared. A
checker that raised on a student's answer, and a missing file or a submission that never
arrived, used to count as the teacher's fault or a 0; see [Scoring](#scoring).
