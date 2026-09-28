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

**A checker that cannot decide is your error, not the student's.** A Rhai error, a checker
script that crashes, or a function checker that raises anything but `AssertionError`
all mark the case `error` with fault `teacher`, so the student is not scored on your
bug. The most common trap is refused before grading: a Rhai check that errors when the
student returns nothing. Guard it:

```toml
check = { rhai = "result != () && result.len() > 2" }
check = { rhai = "type_of(result) == \"array\" && result.contains(5)" }
```

A Rhai expression may run at most a million operations; one that loops past that has not
decided, and is a `checker` error like any other.

## Files

```toml
[meta]
data_files = ["data/poem.txt", "fixtures/"]   # copied into every unit's working directory
```

Every case and every scenario runs in its own temporary directory with your data files
staged in it and the student's file copied beside them. `open("data/poem.txt")` and
`Path(__file__).parent / "data" / "poem.txt"` both work. Relative writes land in that
directory, which is removed afterwards; `expect_files` reads them back. Neither the
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
| `error` | student | `raised` / `syntax` / `load` / `unserialisable` | the student's code crashed |
| `timeout` | student | `timeout` / `killed` | it ran too long |
| `error` | student | `killed` | the process died during the call (a signal) |
| `missing` | student | `no_file` / `no_target` | no file, function, method or attribute to call |
| `error` | student | `dependency` | a value it needs was never produced |
| `error` | student or teacher | `setup` | a setup call failed: the student's or the teacher's, by who owns the call |
| `error` | the culprit's | `not_run` | an earlier call in the unit hung or killed the process |
| `error` | student | `protocol` | the unit's records were forged or repeated |
| `error` | teacher | `checker` / `teacher_import` / `nothing_to_judge` | the bundle's fault |
| `error` | environment | `spawn` / `harness` | the machine's, or the grader's own, fault |

An error message, or a function checker's message, longer than 65,536 characters is cut,
and says how long it was.

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

Results can change on regrading. Students who printed inside a graded function, or who
imported an allowed module such as `random` or `csv`, used to fail every case and now
pass. Cases that a name-bound `@checker` used to judge silently are now judged by what
they declare. A case that paired `sorted` with `expect` used to pass any sorted list; the
value must now equal `expect` too.
