# ScriptMark

[![CI](https://img.shields.io/github/actions/workflow/status/Acture/scriptmark/ci.yml?label=CI)](https://github.com/Acture/scriptmark/actions)
[![Crates.io](https://img.shields.io/crates/v/scriptmark)](https://crates.io/crates/scriptmark)
[![PyPI](https://img.shields.io/pypi/v/scriptmark)](https://pypi.org/project/scriptmark/)
[![License](https://img.shields.io/crates/l/scriptmark)](./LICENSE)

Automated grading CLI for student programming assignments. Rust core, TOML test specifications, Python bindings via PyO3.

## Installation

### Rust CLI

```bash
cargo install scriptmark
```

### Python

```bash
pip install scriptmark
```

```python
import scriptmark

results = scriptmark.grade(["submissions/"], "tests/")
for r in results:
    print(f"{r.student_id}: {r.grade:.1f} ({r.passed}/{r.total})")
```

### From source

```bash
git clone https://github.com/Acture/scriptmark.git
cd scriptmark
cargo install --path crates/scriptmark
```

Requires `python3` on PATH for running student Python code.

## Features

- **Custom test engine** -- runs student code in subprocess, no pytest/unittest required
- **TOML-based specs** -- declarative test cases with expected values, checkers, and parametrization
- **8 built-in checkers** -- exact, approx, sorted, set_eq, contains, regex, text, plus Rhai expressions and external Python checkers
- **Parallel execution** -- tokio-based orchestrator grades all students concurrently
- **Sandboxed execution** -- env isolation, import allowlist, resource limits (setrlimit), timeout with process kill
- **Parametrize + oracle** -- auto-generate test cases with random inputs and teacher reference implementations
- **Canvas LMS integration** -- pull rosters, push grades
- **Similarity detection** -- style + structural code similarity scoring
- **Interactive TUI** -- browse students, sessions, and similarity in the terminal
- **HTML reports** -- standalone dashboard with per-student breakdowns
- **SQLite storage** -- persist grading history across sessions
- **Python API** -- `scriptmark.grade()`, `scriptmark.run()`, `scriptmark.discover()`, `scriptmark.load_spec()`

## Quick Start

1. Write a TOML test spec:

```toml
[meta]
name = "find_max"
file = "lab5.py"
function = "find_max"
language = "python"

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

2. Grade submissions:

```bash
scriptmark grade submissions/ -t tests/
```

## TOML Spec Format

Each `.toml` file defines tests for one function/file:

| Section | Purpose |
|---------|---------|
| `[meta]` | Target file, function, language, teacher imports, `allowed_imports` |
| `[vars]` | Constants injected as Python globals |
| `[[setup]]` | Call functions, store results as `$ref` for later cases |
| `[[cases]]` | Scored test cases with `expect`, `check`, or `expect_error` |
| `[cases.parametrize]` | Auto-generate cases with random args + oracle |
| `[lint]` | Optional style scoring via pylint/ruff |

### Checkers

| Checker | Usage |
|---------|-------|
| `exact` (default) | `expect = 42` |
| `approx` | `expect = 3.14` with `tolerance = 0.01` |
| `text` | Normalized multiline comparison (strips trailing whitespace + blank lines) |
| `sorted` | Validates array is sorted |
| `set_eq` | Unordered array comparison |
| `contains` | Substring match |
| `regex` | `check = { regex = "^\\d+$" }` |
| Rhai expression | `check = { rhai = "result > 0 && result < 100" }` |
| Python script | `check = { python = "verifiers/check.py" }` |

## CLI Commands

```
scriptmark grade        Run tests + summarize + display grades
scriptmark run          Run tests only, output JSON
scriptmark summarize    Re-analyze existing results
scriptmark similarity   Detect code similarity between submissions
scriptmark report       Generate HTML report
scriptmark db           Database management (init, import-roster, sessions, history)
scriptmark tui          Interactive terminal UI
scriptmark roster-pull  Pull roster from Canvas LMS
scriptmark grades-push  Push grades to Canvas LMS
```

## Architecture

Single Rust crate (`scriptmark`) with modular structure:

```
models/       Data models, TOML spec parsing, grading policies
discovery     Student file discovery + ZIP extraction
runner/       PythonExecutor, orchestrator, sandbox, parametrize, oracle
checker/      Checker trait + 8 built-in implementations + Rhai + Python
db/           SQLite persistence (rusqlite, bundled)
canvas/       Canvas LMS API client (reqwest + rustls)
tui/          Interactive terminal UI (ratatui)
```

`scriptmark-py` is a separate PyO3 cdylib crate providing Python bindings, distributed via PyPI.

## License

[GPL-3.0-or-later](./LICENSE)
