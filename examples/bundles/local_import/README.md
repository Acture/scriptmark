# Local roster or explicit submission list

From the repository root:

```fish
cargo run -p scriptmark -- match -t examples/bundles/local_import/tests -o output/local-matches.json
cargo run -p scriptmark -- grade -t examples/bundles/local_import/tests -o output/local.json
cargo run -p scriptmark -- export output/local.json -o output/local-grades.xlsx

# The same class, with every submitted file assigned explicitly.
cargo run -p scriptmark -- grade -t examples/bundles/local_import/tests --assignment examples/bundles/local_import/manifest.toml -o output/manifest.json
```

`001` earns full points, `002` earns zero, and `003` remains ungraded because no work
was submitted. Student numbers remain text, including their leading zeros.

`assignment.toml` selects row 2 as the CSV header and maps its Chinese headings.
To use an Excel roster, save the same table as `roster.xlsx`, keep the student ID
column as **text**, change `input.roster.path`, and set `input.roster.sheet` to the
worksheet name. Multiple-sheet workbooks require an explicit sheet. Columns accept
either exact header names or one-based column numbers.

Paths in either configuration are relative to that configuration file. CLI paths
are relative to the working directory; positional submissions and `--roster` override
the corresponding configured paths. The explicit `input.students` list is a separate
input mode and cannot be combined with a roster table or submission directories.
It accepts individual files and archives; `files = []` records a non-submitter.
File-to-item and function matching still use the normal `[matching]` rules.

Run `match` to see import diagnostics without executing student code. Invalid rows,
numeric Excel student IDs and invalid/duplicate submission paths stop grading, with
the source file, worksheet and row where available. Repeated identical roster IDs
are reported and merged; conflicting identities stop grading.
