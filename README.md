# ScriptMark

[![CI](https://img.shields.io/github/actions/workflow/status/Acture/scriptmark/ci.yml?label=CI)](https://github.com/Acture/scriptmark/actions)
[![Crates.io](https://img.shields.io/crates/v/scriptmark)](https://crates.io/crates/scriptmark)
[![PyPI](https://img.shields.io/pypi/v/scriptmark)](https://pypi.org/project/scriptmark/)
[![License](https://img.shields.io/crates/l/scriptmark)](https://spdx.org/licenses/GPL-3.0-or-later.html)

Automated grading CLI for student programming assignments. Rust core, TOML test specifications, Python bindings via PyO3.

## Documentation and checkout

Project documentation lives in
[Acture/obsidian-vault, branch `project/scriptmark`](https://github.com/Acture/obsidian-vault/tree/project/scriptmark/scriptmark),
mounted here as the `notes/` submodule. Start with
[the project index](https://github.com/Acture/obsidian-vault/blob/project/scriptmark/scriptmark/README.md)
or open `notes/scriptmark/README.md` locally. The teacher guide is
`notes/scriptmark/docs/test-bundles.md`; design records are under
`notes/scriptmark/docs/plans/`. Access to the notes repository is required to initialize it.

Clone with the pinned documentation commit:

```fish
git clone --recurse-submodules https://github.com/Acture/scriptmark.git
cd scriptmark
```

For an existing clone, or after pulling code changes, restore the recorded commit:

```fish
git submodule sync -- notes
git submodule update --init --recursive -- notes
```

The parent repository pins a commit. The `branch = project/scriptmark` setting in
`.gitmodules` is used by `--remote`; it does not automatically advance that pin.
To adopt already-pushed documentation updates, start with a clean submodule:

```fish
git -C notes status --short
git submodule update --remote -- notes
git diff --submodule=log -- notes
git add notes
git commit -m "docs: update ScriptMark notes"
git push
```

To edit documentation, first leave the detached checkout created by initialization:

```fish
git -C notes fetch origin project/scriptmark
git -C notes switch project/scriptmark
git -C notes pull --ff-only origin project/scriptmark
# Edit notes/scriptmark/; inspect the changes before staging.
git -C notes diff -- scriptmark/
git -C notes add -- scriptmark/
git -C notes commit -m "docs(scriptmark): describe the change"
git -C notes push origin HEAD:refs/heads/project/scriptmark
and git add notes
and git commit -m "docs: update ScriptMark notes"
and git push
```

Push the notes successfully **before** updating the code repository's gitlink. If a
fast-forward fails, resolve the divergence while preserving both versions. Do not
force-push or discard notes. Runtime code, examples and artifacts stay in this repository;
documentation edits belong on the notes branch. Original documentation history and
the P-673 worktree handoff are recorded in
[the migration record](https://github.com/Acture/obsidian-vault/blob/project/scriptmark/scriptmark/MIGRATION.md).

## Installation

```bash
cargo install scriptmark     # Rust
pip install scriptmark        # Python
```

## Quick Start

Write a TOML test spec, point ScriptMark at student submissions:

```bash
scriptmark grade submissions/ -t tests/
```

```
┌─────────┬─────────────┬──────────┬─────────────────┬───────┬─────┬───────┬───────┐
│ Student ┆ ID          ┆ State    ┆ Reason          ┆ Score ┆ Raw ┆ Grade ┆ Cases │
╞═════════╪═════════════╪══════════╪═════════════════╪═══════╪═════╪═══════╪═══════╡
│ Alice   ┆ 20000000001 ┆ GRADED   ┆                 ┆ 10/10 ┆ 100 ┆   100 ┆ 12/12 │
│ Bob     ┆ 20000000002 ┆ GRADED   ┆                 ┆  7/10 ┆  70 ┆    70 ┆  9/12 │
│ Carol   ┆ 20000000003 ┆ WITHHELD ┆ not_submitted   ┆     - ┆   - ┆     - ┆   0/0 │
└─────────┴─────────────┴──────────┴─────────────────┴───────┴─────┴───────┴───────┘
```

Each test spec is a grading item, worth its declared points however many cases it runs.
A teacher's or the machine's failure withholds a grade — shown as `-` with a reason, and
never pushed — instead of scoring it as 0.

83 students graded in under 2 seconds on an M1 Mac.

## CLI Usage

```bash
# Grade with roster and database storage; points and any curve come from assignment.toml
scriptmark grade submissions/ -t tests/ -r roster.csv --db grades.db -a archive/

# Run tests only (raw JSON output)
scriptmark run submissions/ -t tests/ -o results.json

# Inputs and oracle answers are frozen beside the results (output/results.cases.json);
# grade late submissions on the same tests and answers, into results of their own
scriptmark grade late/ -t tests/ --replay output/results.cases.json -o output/late.json

# Detect plagiarism
scriptmark similarity submissions/ --threshold 0.8

# Generate HTML report
scriptmark report results.json -o report.html

# Canvas LMS: find a course, fetch an assignment, grade it offline, push grades back
export CANVAS_TOKEN=... CANVAS_URL=https://canvas.university.edu
scriptmark canvas courses
scriptmark canvas assignments --course-id 12345
scriptmark canvas fetch --course-id 12345 --assignment-id 67890 -o canvas/hw1
scriptmark grade --canvas canvas/hw1 -t tests/
scriptmark grades-push --course-id 12345 --assignment-id 67890 output/results.json

# Or just pull the roster
scriptmark roster-pull --course-id 12345

# Browse results interactively
scriptmark tui grades.db
```

## Python API

```python
import scriptmark

# One-shot grading, under the assignment.toml beside tests/ (or pass assignment=...).
# freeze= keeps the generated inputs; replay= grades on ones kept earlier.
results = scriptmark.grade(["submissions/"], "tests/", freeze="output/cases.json")
for r in results:
    if r.grade is None:
        print(f"{r.student_id}: withheld ({r.reason})")
    else:
        print(f"{r.student_id}: {r.grade} ({r.score}/{r.max} points)")

# Discover student files (convenience view — drops non-submitters and orphan files)
# Keys are rendered student keys: a bare 学号 once a roster confirms it, otherwise
# `local:<token>` — the prefix means nothing has vouched for that filename token yet.
subs = scriptmark.discover(["submissions/"])  # {'local:alice': ['path/to/alice_lab5.py'], ...}

# The full input model: every student keeps an outcome, nothing is dropped
inp = scriptmark.load_input(["submissions/"], roster="roster.csv")
for s in inp["students"]:
    print(s["identity"]["key"], s["state"])   # not_submitted | submitted_empty | executable
print(inp["unmatched"], inp["diagnostics"])

# Load and inspect a spec
spec = scriptmark.load_spec("tests/test_lab5.toml")
print(spec.name, spec.function, spec.num_cases)
```

## TOML Test Specs

```toml
[meta]
name = "find_max"
file = "lab5.py"
function = "find_max"
language = "python"
allowed_imports = ["numpy"]  # optional: extra packages beyond safe stdlib

[[cases]]
name = "basic"
args = [[3, 1, 5, 2]]
expect = 5

[[cases]]
name = "negative"
args = [[-3, -1, -5]]
expect = -1

[[cases]]
name = "random inputs"
[cases.parametrize.args]             # one line per parameter, in call order
nums = "list(int(-100, 100), 5, 20)"
[cases.parametrize]
samples = [[[7]], [[0, 0, 0]]]        # inputs always run, like fixed args
[cases.parametrize.random]
count = 20
seed = 42                             # omit for 0; "random" draws one and records it
[cases.parametrize.oracle]
rhai = "nums.sort(); nums[nums.len() - 1]"
```

A fixed or generated case can instead use an independent teacher implementation:

```toml
[[cases]]
name = "reference answer"
args = [[3, 1, 5, 2]]
[cases.oracle]
reference = "solutions.py"
function = "expected_max"
```

Reference answers are computed before grading and frozen for reuse. `--replay` verifies
their sources and configuration, then reuses the answers without recomputing them.
References can also supply declared exceptions, stdout and text files; see the
[reference bundle](examples/bundles/reference_oracle) and
[answer contract](https://github.com/Acture/obsidian-vault/blob/project/scriptmark/scriptmark/docs/test-bundles.md#reference-implementations).

Every case runs in its own process and working directory. When state should carry over —
an object built once and driven step by step — say so with a scenario:

```toml
[[scenarios]]
name = "ada's account"

[[scenarios.setup]]
id = "acct"
function = "Account"
args = ["ada", 100]

[[scenarios.steps]]
name = "deposit"
method = "deposit"
object = "acct"
args = [50]
expect = 150

[[scenarios.steps]]
name = "balance kept"
attribute = "balance"
object = "acct"
expect = 150
```

Anything a bundle cannot honour — an unknown field, a checker that does not exist, a
case with nothing to judge — is refused before a single student runs. Every failure says
whose it is: the student's, the teacher's, or the machine's. See
[the teacher guide](https://github.com/Acture/obsidian-vault/blob/project/scriptmark/scriptmark/docs/test-bundles.md) for the full contract, and
[examples/bundles](examples/bundles) for a pure function, a shared object and file I/O.

### Scoring

`assignment.toml` beside the tests directory says what each item is worth and how a
grade is reached. Every field has a default; without the file, each spec is an item worth
1 point and the grade is `score / max × 100`.

```toml
[assignment]
name = "lab5"

[grading]
missing = "withheld"       # or "zero": a student who submitted nothing
missing_file = "withheld"  # or "zero": a submission without an item's file
scale = 100
decimals = 2
# curve = { kind = "template", name = "sqrt", lower = 60, upper = 100 }  # shown beside the raw grade
# lint_points = 1           # lint counts only when given points

[[items]]
id = "find_max"            # a spec's [meta] name
points = 3
aggregation = "proportional"  # points × pass rate; or "all_or_nothing"
```

See [the scoring contract](https://github.com/Acture/obsidian-vault/blob/project/scriptmark/scriptmark/docs/test-bundles.md#scoring) for every rule.

### Checkers

| Checker | Usage |
|---------|-------|
| `exact` (default) | `expect = 42` |
| `approx` | `check = { builtin = "approx", tolerance = 0.01 }` with `expect` |
| `text` | Normalized multiline comparison |
| `sorted` / `set_eq` / `contains` | Collection checks |
| Rhai expression | `check = { rhai = "result != () && result > 0" }` |
| Python script | `check = { python = "verifiers/check.py" }` |
| Teacher function | `check = { function = "is_valid" }`, run on the live value |
| Output and files | `expected_stdout = "..."`, `expect_files = { "out.txt" = "..." }` |

## Features

- **Custom test engine** -- subprocess execution, no pytest dependency
- **Isolated units** -- a process and directory per case, env isolation, import allowlist, setrlimit, timeout with kill; contains accidents, not a security sandbox ([details](https://github.com/Acture/obsidian-vault/blob/project/scriptmark/scriptmark/docs/test-bundles.md#what-grading-does-not-defend-against))
- **Parallel** -- tokio orchestrator, grades 80+ students in seconds
- **Parametrize + oracle** -- random inputs with teacher reference implementations
- **Per-item scoring** -- declared points and aggregation; zero and withheld kept apart in every export
- **Canvas LMS** -- roster pull, grades push
- **Similarity detection** -- style + structural code comparison
- **TUI + HTML reports** -- interactive browser and standalone dashboards
- **SQLite** -- persist grading history across sessions

## License

[GPL-3.0-or-later](https://spdx.org/licenses/GPL-3.0-or-later.html)
