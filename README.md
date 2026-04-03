# ScriptMark

[![CI](https://img.shields.io/github/actions/workflow/status/Acture/scriptmark/ci.yml?label=CI)](https://github.com/Acture/scriptmark/actions)
[![Crates.io](https://img.shields.io/crates/v/scriptmark)](https://crates.io/crates/scriptmark)
[![PyPI](https://img.shields.io/pypi/v/scriptmark)](https://pypi.org/project/scriptmark/)
[![License](https://img.shields.io/crates/l/scriptmark)](./LICENSE)

Automated grading CLI for student programming assignments. Rust core, TOML test specifications, Python bindings via PyO3.

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
┌─────────┬─────────────┬────────┬────────┬────────┬───────┬───────────┬───────┐
│ Student ┆ ID          ┆ Status ┆ Passed ┆ Failed ┆ Total ┆ Pass Rate ┆ Grade │
╞═════════╪═════════════╪════════╪════════╪════════╪═══════╪═══════════╪═══════╡
│ Alice   ┆ 22300110012 ┆ PASSED ┆     10 ┆      0 ┆    10 ┆   100.0%  ┆ 100.0 │
│ Bob     ┆ 22300110039 ┆ FAILED ┆      7 ┆      3 ┆    10 ┆    70.0%  ┆  83.7 │
│ Carol   ┆ 22300110046 ┆ FAILED ┆      3 ┆      7 ┆    10 ┆    30.0%  ┆  69.0 │
└─────────┴─────────────┴────────┴────────┴────────┴───────┴───────────┴───────┘
```

83 students graded in under 2 seconds on an M1 Mac.

## CLI Usage

```bash
# Grade with roster, grading curve, and database storage
scriptmark grade submissions/ -t tests/ -r roster.csv -g sqrt --db grades.db

# Run tests only (raw JSON output)
scriptmark run submissions/ -t tests/ -o results.json

# Detect plagiarism
scriptmark similarity submissions/ --threshold 0.8

# Generate HTML report
scriptmark report results.json -o report.html

# Canvas LMS: pull roster / push grades
CANVAS_TOKEN=... scriptmark roster-pull --canvas-url https://... --course-id 12345
CANVAS_TOKEN=... scriptmark grades-push --canvas-url https://... --course-id 12345 --assignment-id 67890 results.json

# Browse results interactively
scriptmark tui grades.db
```

## Python API

```python
import scriptmark

# One-shot grading
results = scriptmark.grade(["submissions/"], "tests/")
for r in results:
    print(f"{r.student_id}: {r.grade:.1f} ({r.passed}/{r.total})")

# Discover student files
subs = scriptmark.discover(["submissions/"])  # {'alice': ['path/to/alice_lab5.py'], ...}

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
[cases.parametrize]
count = 20
seed = 42
[cases.parametrize.args]
nums = "list(int(-100, 100), 5, 20)"
[cases.parametrize.oracle]
rhai = "nums.sort(); nums[nums.len() - 1]"
```

### Checkers

| Checker | Usage |
|---------|-------|
| `exact` (default) | `expect = 42` |
| `approx` | `expect = 3.14` with `tolerance = 0.01` |
| `text` | Normalized multiline comparison |
| `sorted` / `set_eq` / `contains` | Collection checks |
| `regex` | `check = { regex = "^\\d+$" }` |
| Rhai expression | `check = { rhai = "result > 0" }` |
| Python script | `check = { python = "verifiers/check.py" }` |

## Features

- **Custom test engine** -- subprocess execution, no pytest dependency
- **Sandboxed** -- env isolation, import allowlist, setrlimit, timeout with kill
- **Parallel** -- tokio orchestrator, grades 80+ students in seconds
- **Parametrize + oracle** -- random inputs with teacher reference implementations
- **Canvas LMS** -- roster pull, grades push
- **Similarity detection** -- style + structural code comparison
- **TUI + HTML reports** -- interactive browser and standalone dashboards
- **SQLite** -- persist grading history across sessions

## License

[GPL-3.0-or-later](./LICENSE)
