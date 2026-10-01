# P-676: independent reference answers and frozen expectations

Implementation on the P-675 baseline `e3b0fa5`. Live execution state belongs to Linear
P-676; this file records the implementation contract and local validation.

## Contract

- `cases.oracle` supplies answers for fixed or generated inputs. A template can also
  use its existing `parametrize.oracle`, but the two cannot be declared together.
- `reference` is an exact file and `function` an explicit public, inspectable entry.
  Fixed calls validate positional arity; generated calls validate the parameter names
  and order too. This implements the reference-signature item previously listed in
  P-794; generated setup data and terminal argument display remain there.
- The reference uses the existing `Subject::Reference` executor, with the case's
  stdin, args, vars, teacher imports, data files and timeout. Independent cases only:
  oracle scenarios, methods, attributes, scripts and top-level setup are refused.
- Return expectations are the default. `returns = true` explicitly permits `None`;
  `returns = false` supports output/file tasks. `stdout` and `files` select additional
  observations. `raises` declares the intended exception type; unrelated failures,
  timeouts, load/signature failures and incomplete observations abort preparation.
- Sources for the same observation conflict instead of silently overriding one another.
  Disjoint fixed expectations and a checker over the returned answer can coexist.
- File observations now reject oversized or invalid UTF-8 files instead of silently
  comparing a truncated/replacement-decoded prefix. The existing limits still apply.

## Freezing and replay

Format 2 retains the generated-input map and adds answers indexed by spec and concrete
case. Returns are tagged so an expected null survives serialization. Each answer set
records the expanded, unresolved spec/configuration, default timeout, ScriptMark version,
runtime identity, protocol digest, source fingerprints and a checksum.

Reference files, teacher imports, Python checker scripts and staged data (recursively)
are SHA-256 fingerprinted. Python identity includes the resolved executable and its
content hash. Source changes during preparation abort the batch. Runtime environment
and undeclared transitive dependencies are not snapshotted: declare local helpers as
imports/data files and change `oracle.version` when ambient dependencies change.
Absolute source paths mean relocation requires fresh preparation.

Replay checks the inputs and answer contract, restores the saved expectations and does
not execute reference entries or Rhai oracles. Changed sources/configuration/runtime,
missing answers, corrupt checksums and format 1 files require fresh preparation. A
fresh run also refuses to overwrite a different answer contract unless `--fresh` is
given. To retain previously random inputs after a correction, pin their recorded seed
before preparing afresh. P-678's evidence-based regrading remains separate.

Answers are resolved before any student runs and persisted with the existing results
and archive flow. A teacher preparation failure writes no new results and preserves
previous results/frozen bundles. The artifact checksum catches accidental edits, not a
hostile writer with access to the teacher's files.

## Validation

- `cargo test --workspace --offline`: all 368 tests passed, including the new oracle
  suite, CLI preservation/no-publication test and runnable reference example.
- `cargo clippy --workspace --all-targets --offline -- -D warnings` and `cargo fmt
  --all --check` are the Rust checks. Build the Python binding with
  `set -x PYO3_PYTHON /opt/homebrew/bin/python3.13` in fish: this repository's
  PyO3 does not support the machine's default Python 3.14.
- `ruff check` passes on the modified harness and new Python examples. The changed
  harness function and examples are formatted with tabs; `ty check` passes on the
  examples. Whole-harness `ty check` has the same 10 pre-existing diagnostics as HEAD;
  the diagnostic kinds/messages were compared. Whole-file formatting also already
  differed at HEAD, so only the changed function was formatted.
- Existing Canvas tests require permission to bind localhost mock-server ports;
  the full suite passed with that permission. No external Canvas service is used.
- Final review tightened conflicts for equivalent paths such as `out.txt` and
  `./out.txt`; the library/oracle suites and Rust checks passed again after that edit.

Coverage includes correct/wrong students, fixed/sampled/seeded inputs, exception
expectations, stdout/files/stdin/vars, explicit null round trips, unchanged reference
call counts across students/replay, configuration/source/data/helper drift, mutation
during preparation, corruption/missing answers, arity/order errors, missing entries,
timeouts, incomplete output/files, conflicts and old-format refusal.

See `docs/test-bundles.md` and `examples/bundles/reference_oracle` for teacher-facing
usage. Review and delivery state are tracked in Linear P-676.
