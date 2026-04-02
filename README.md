# ScriptMark

[![CI](https://img.shields.io/github/actions/workflow/status/Acture/scriptmark/ci.yml?label=CI)](https://github.com/Acture/scriptmark/actions)
[![Crates.io](https://img.shields.io/crates/v/scriptmark-cli)](https://crates.io/crates/scriptmark-cli)
[![License](https://img.shields.io/crates/l/scriptmark-cli)](./LICENSE)

Automated grading CLI for student programming assignments. Rust core, TOML test specifications, no pytest dependency.

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

## Installation

### From source (recommended for now)

```bash
git clone https://github.com/Acture/scriptmark.git
cd scriptmark
cargo install --path crates/scriptmark-cli
```

### From crates.io (once published)

```bash
cargo install scriptmark-cli
```

Requires `python3` on PATH for running student Python code.

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
| `[meta]` | Target file, function, language, teacher imports |
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
scriptmark grade      Run tests + summarize + display grades
scriptmark run        Run tests only, output JSON
scriptmark summarize  Re-analyze existing results
scriptmark similarity Detect code similarity between submissions
scriptmark report     Generate HTML report
scriptmark db         Database management (init, import-roster, sessions, history)
scriptmark tui        Interactive terminal UI
scriptmark roster-pull  Pull roster from Canvas LMS
scriptmark grades-push  Push grades to Canvas LMS
```

## Architecture

8-crate Rust workspace:

```
scriptmark-core       Models, TOML parsing, discovery, grading, similarity
scriptmark-runner     PythonExecutor, Checker trait, orchestrator, parametrize, oracle
scriptmark-cli        clap CLI with 9 commands + HTML report template
scriptmark-db         SQLite persistence (rusqlite, bundled)
scriptmark-canvas     Canvas LMS API client (reqwest)
scriptmark-tui        ratatui terminal UI
scriptmark-py         PyO3 Python bindings (in development)
```

## License

[GPL-3.0-or-later](./LICENSE)
