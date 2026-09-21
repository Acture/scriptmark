# P-669 — Unified assignment / student / submission input model

Linear: https://linear.app/acturea/issue/P-669
Parent: P-663 · Milestone: Canvas 与本地提交可统一导入

Revision 2 — rewritten after an adversarial design review (6 blockers, 11 majors).

## Scope

Build the typed input contract that both entry points (Canvas, local) produce and that
everything downstream consumes. **Not** in scope: Canvas HTTP fetching and attachment
download (P-670), XLSX / column mapping (P-672), teacher matching rules (P-673), the
teacher test package (P-674), per-item scoring and zero-vs-ungraded policy (P-677).

## Decisions

### D1 — Identity has one total key

```rust
pub enum StudentKey {
    Number(String),      // confirmed 学号: roster hit, or Canvas sis_user_id
    CanvasUser(u64),     // Canvas user with no SIS id
    Extracted(String),   // token pulled off a local filename, unconfirmed
}
```

`Display` renders `2024010001`, `canvas:12345`, `local:notes` — the prefixed forms can
never be mistaken for a 学号. `StudentReport.student_id` carries that rendering, so the db
primary key, the CSV column and the TUI search key stay unambiguous. `student_number`,
`canvas_user_id`, `sis_user_id` and `login_id` remain separate optional provenance fields
on `StudentIdentity`; the key never replaces them.

Ordering is `(StudentKey, source_ordinal, first_path)` — total, and independent of
`read_dir` order, so duplicates sort stably.

### D2 — Key comparison is exact text after a trim, on every adapter

Trim ASCII/Unicode whitespace and strip a UTF-8 BOM. Never case-fold, never parse as an
integer, never strip zeros. `sis_user_id` deserializes as `Option<String>` only: a payload
sending `"sis_user_id": 24010003` unquoted is a type error, not a silent coercion, and a
test pins that.

`"012345"` and `"12345"` are different students and are never merged. Because keys are
exact text they cannot collide, so the diagnostic is named for what it is — a suspicion
that an upstream export dropped padding:

> one pass over the sorted students, bucketed by zero-stripped key; every bucket holding
> more than one distinct raw key emits one `SuspectedZeroPaddedVariant { keys }` at
> Warning. Keys stay distinct.

The asymmetric case (roster has `24010003`, submission carries `0024010003`) therefore
yields a `NotInRoster` submission **and** a `NotSubmitted` roster entry, plus that
diagnostic linking them. That is the 明确结果; auto-repair is deliberately not attempted.

### D3 — The two axes are orthogonal; the ticket's four outcomes are computed

Storing delivery state and roster state in one enum lets them contradict each other
(`ReceivedUnmatched` + `Matched`), and hides `SubmittedEmpty` whenever a non-roster student
submits nothing usable. Stored separately:

```rust
enum SubmissionState { NotSubmitted, SubmittedEmpty, Executable }   // delivery only
enum RosterMatch     { Matched(usize), Ambiguous(Vec<usize>), NotInRoster, NoRoster }
```

`Executable` = at least one file with a recognised language (P-674 may tighten it).
The ticket's four outcomes come out of a method, so the pair can never disagree:

```rust
fn outcome(&self) -> SubmissionOutcome   // NotSubmitted | SubmittedEmpty | ReceivedUnmatched | Executable
```

`ReceivedUnmatched` dominates for *reporting*: anything received from someone not on the
roster is unmatched regardless of whether its files would run. Execution is gated on the
delivery axis instead, so an unmatched submitter's code still runs — a teacher needs that
output to resolve the clash — and `apply_grading` withholds the grade.

`NotSubmitted` is produced only by `not_submitted()`, which always sets `Matched` or
`Ambiguous`, so `(NotSubmitted, NotInRoster)` does not arise in tree. `received()` carries a
`debug_assert!` against the empty-attempt case that would otherwise let a caller construct
it.

### D4 — The runner takes `&[StudentSubmission]`, not `AssignmentInput`

Handing the executor the whole `AssignmentInput` would put `workflow_state`, `late`,
`missing`, `excused`, attachment URLs and `login_id` inside the core scoring path — exactly
the 入口特有字段 the ticket bars. `run_all(&[StudentSubmission], &[TestSpec], …)` keeps
`AssignmentInput` a boundary object P-673/P-677 can extend without touching the runner, and
keeps the 12 integration-test sites to one line each.

One report per input element, **in input order** (today's spawn order is HashMap-random).
A `JoinError` produces an error-state report rather than a student silently disappearing.

### D5 — Nothing is silently dropped, and the anti-overwrite rule holds past RAM

- Extracted key, not in roster → retained student, `outcome() == ReceivedUnmatched`.
- No extractable key → `unmatched: Vec<UnmatchedArtifact>`.
- Recognisable owner, unusable file type → `IgnoredFile` diagnostic; an owner with only
  ignored files is `SubmittedEmpty`.
- Two archive entries flattening to one name → `ArchiveNameCollision`, not a silent skip.

Retaining duplicates in memory is pointless if SQLite merges them again. `save_session`
currently writes `INSERT OR REPLACE` into a table with `UNIQUE(session_id, student_id)`
(`db/results.rs:51`, `db/schema.rs:36`), and `import_roster` upserts on
`id TEXT PRIMARY KEY` while counting rows *iterated* rather than stored. So: duplicate
`student_id`s are rejected before any write with a hard error (fail-fast, per CLAUDE.md),
and `import_roster` returns rows actually stored. No schema change.

`avg_grade` currently divides a `filter_map(final_grade)` sum by `reports.len()`
(`db/results.rs:37-41`) — with ungraded students that average is wrong; divide by the
graded count.

### D6 — A non-submitter is never given a numeric zero

`apply_template` and `apply_formula` both open with
`if report.status() == TestStatus::Missing { final_grade = Some(0.0) }`
(`grading.rs:19-22`, `63-66`), and an empty `test_results` *is* `Missing`. Flowing
non-submitters through unchanged would write a hard 0.0 into the summary, the CSV, the db
average — and `cmd_grades_push` pushes every report with `final_grade.is_some()`
(`main.rs:562-574`), so a routine `grade` + `grades-push` would post zeros to Canvas for
students who never submitted.

Rule: `apply_grading` skips any report whose `submission_state` is present and not
`Executable`, leaving `final_grade: None`. The guard is on `submission_state`, **not** on
`status() == Missing` — a student who did submit but whose specs produced nothing keeps
today's behaviour, and legacy reports (`submission_state: None`) are untouched.
`cmd_grades_push` filters on `Executable` and keys on `canvas_user_id`, never on
`student_id.parse::<u64>()`. Turning any non-executable outcome into a number is P-677.

### D7 — Canvas payload types live in `input/canvas.rs`; `canvas/client.rs` is untouched

`canvas/mod.rs` re-exports only `{CanvasClient, CanvasError}`, so its `CanvasUser` /
`CanvasSubmission` are unreachable from an adapter or a test; and `CanvasSubmission` is a
grade-push response shape (`{id, user_id, score, grade}`) with no `submission_history`,
`submission_type` or `body`. P-669 defines its own serde payload structs mirroring the API
JSON. P-670 maps its HTTP DTOs into them — it owns that wiring, not this ticket.

The core stays source-neutral: `source_status: Option<SourceStatus>` (only Canvas fills
it), and `Attachment.url` / `content_type` / `size` are `Option` — `late: false` on a local
submission is a false statement, not a neutral default.

### D8 — Attachments become files only once they are on disk

`normalize()` is pure and download is P-670, so attachments have no local path of their
own. Signature:

```rust
fn normalize(payload: &CanvasPayload, roster: Option<&Roster>, downloads: &HashMap<u64, PathBuf>) -> AssignmentInput
```

`downloads` maps attachment id → downloaded path. The fixture fills it from committed
files; P-670 fills it after downloading. An attachment absent from `downloads` yields a
`PendingDownload` diagnostic and does **not** make a student `Executable`. Provenance runs
the whole chain: `FileOrigin::Attachment { attempt, attachment_id }` sits beside `Direct`
and `Archive`.

`SubmittedEmpty` = no attachments **and** no usable body. A Canvas `online_text_entry`
carrying a non-empty body but no file gets its own diagnostic rather than being mislabelled.

### D9 — The roster is the roster of record whenever one is supplied

Canvas `users` supplies identity enrichment only (name, `sis_user_id`, `login_id`); it
never creates or removes membership. With `roster: None` on the Canvas path, enrollment
*is* the roster and `NoRoster` is not used.

`pull_roster` collapses `sis_user_id` / `login_id` / stringified Canvas id into one key with
a silent `insert` (`canvas/client.rs:110-117`) and `save_roster_csv` writes no Canvas id —
that path is deliberately **not** migrated here; it moves in P-670, and until then it can
feed non-conforming keys into the model.

### D10 — Grading-item identity is the existing one; no new dead type

The repo already has a grading unit with an identity and a live association:
`TestSpec.meta.name` → `TestResult.spec_name` (`orchestrator.rs:229`). Adding a parallel
`GradingItem` that nothing references would be dead code, and `points` pre-empts P-677.
P-669 records the association and stops there.

`AssignmentInfo` does gain `canvas_course_id`, `canvas_assignment_id` and `attempt_policy`,
because the ticket demands course/assignment ids be stored separately — and those are wired
for real: `load_assignment_config` has zero callers today, so `cmd_grade` / `cmd_run` gain
`--assignment <file>`, defaulting to `assignment.toml` beside the tests dir. An unread
config key is a knob that silently does nothing.

`AttemptPolicy::Latest` = highest `attempt` integer; `submitted_at` is a documented
tie-break only. The workspace has no date library, so a timestamp comparison would be a
lexicographic string compare — correct only for uniform UTC and silently wrong otherwise.

### D11 — Diagnostics are data, not pre-rendered strings

`kind: DiagnosticKind` is a data-carrying enum deriving `Display` via `thiserror`, matching
how `RosterError` and `DiagnosticError` already work in these files; there is no separate
`message` field to drift from it. `InputSource` lives on `AssignmentInput` only — a
per-student copy would let an input claim `Local` while holding Canvas students.

Fail-fast boundary: structural failures (`NotADirectory`, malformed CSV) stay `Err`;
per-record anomalies become diagnostics. Adapters and the validator never print — only
`main.rs` renders.

### D12 — Extend, never rename

`StudentFile` (6 uses in `runner/python.rs`, which P-673 owns) and
`StudentReport.student_id` (db schema, CSV headers, TUI, similarity) keep their names.
New `StudentReport` fields are `Option<_>` + `#[serde(default)]`; `submission_state` is
**not** a bare enum with a `Default`, because every legacy record would then claim a state
it never had. Every new enum gets `#[serde(rename_all = "snake_case")]` to match
`TestStatus`.

`cmd_similarity` and `cmd_report` keep their own `split('_')` key extraction for now — a
recorded decision, not an oversight; routing them through the model is a follow-up.

## Model (`models/submission.rs`)

```
Assignment        { name, canvas_course_id: Option<u64>, canvas_assignment_id: Option<u64> }
StudentKey        { Number(String) | CanvasUser(u64) | Extracted(String) }
StudentIdentity   { key, student_number, canvas_user_id, sis_user_id, login_id, name, sortable_name, email }
Attachment        { id, filename, content_type: Option, size: Option, url: Option }
SourceStatus      { workflow_state, late, missing, excused }          // Canvas only
SubmissionAttempt { attempt, submitted_at, source_status: Option, attachments, files }
StudentFile       { path, language, origin: FileOrigin }
FileOrigin        { Direct | Archive { archive, entry } | Attachment { attempt, attachment_id } }
SubmissionState   { NotSubmitted | SubmittedEmpty | Executable }
RosterMatch       { Matched(usize) | Ambiguous(Vec<usize>) | NotInRoster | NoRoster }
SubmissionOutcome { NotSubmitted | SubmittedEmpty | ReceivedUnmatched | Executable }   // computed
StudentSubmission { identity, roster_match, state, attempts, selected: Option<usize> }
UnmatchedArtifact { path, reason }
InputSource       { Canvas { course_id, assignment_id } | Local { scanned_dirs, roster_path } }
InputDiagnostic   { severity, kind: DiagnosticKind, location: Option<SourceLocation> }
SourceLocation    { file: Option<PathBuf>, sheet: Option<String>, row: Option<usize> }
AssignmentInput   { assignment, source, roster: Option<Roster>, students: Vec<_>, unmatched, diagnostics }
```

`files` is **not** a fourth parallel copy of the same bytes: `StudentFile`s live on the
attempt, and `StudentSubmission::files()` reads through `selected`. A submission whose
files come from attempt 1 while `selected` points at attempt 2 is therefore not
representable.

`diagnostics` and `unmatched` are sorted by `(location, kind)` — `read_dir` order is not
stable across filesystems, so an unsorted vector would be flaky in CI.

## Equivalence: a projection, asserted against an absolute table

```rust
struct ProjectedStudent { key: String, outcome: SubmissionOutcome, artifacts: Vec<String> }
```

`key` is `StudentKey::raw()`, not its `Display` form. Whether a student number counts as
*confirmed* is a property of the source — Canvas vouches for its own SIS ids, a local
filename token vouches for nothing — so comparing the prefixed form would report a
difference in confidence as a difference in identity.

A cross-source `assert_eq!` alone can pass on two empty vectors — which is exactly what a
gitignored fixture tree produces in CI. So the test asserts each side against a
hand-written expected table **first**, checks `students.len()` on both, and only then
compares the two.

The one shape difference is deliberate and documented in the test: the local fixture names
files `{sid}_{name}` (the real Canvas bulk-download convention) while a Canvas attachment
carries `{name}`, so the test strips a leading `{key}_` before comparing. That
canonicaliser lives in the test, not the model — it is a matching rule, and matching is
P-673.

## Fixture design

`crates/scriptmark/tests/fixtures/hw1/` — `.gitignore` gained
`!crates/scriptmark/tests/fixtures/**` (verified with `git add -n`: the bare `submissions`,
`*.csv`, `templates` and `config.toml` rules would otherwise swallow the tree, and the test
would then pass on two empty vectors in CI).

| Key | Canvas | Local | Expected |
|---|---|---|---|
| `2024010001` | 1 attachment, downloaded | `2024010001_lab1.py` | `Executable`; roster has two rows for it → `RosterMatch::Ambiguous` + `DuplicateRosterEntry`, and no name is guessed |
| `2024010002` | attempts 1 and 2, different attachments | `2024010002_lab1.py` | `Executable`, `selected.attempt == 2` |
| `0024010003` / `24010003` | both present | both | distinct students, `SuspectedZeroPaddedVariant` |
| `2024010004` | placeholder row: `workflow_state: unsubmitted`, `attempt: null` | roster only | `NotSubmitted` |
| `2024010005` | `workflow_state: submitted`, `attempt: 1`, `attachments: []` | `2024010005_notes.txt` | `SubmittedEmpty` |
| `9999999999` | submitted, enrolled, absent from roster CSV | `9999999999_lab1.py` | `ReceivedUnmatched` |
| — | — | `_scratch_v2.py` | `unmatched` artifact |

Only the local side can have orphan files: Canvas attributes every attachment to a user by
construction, so `unmatched` is asserted per side rather than compared across them.

`_scratch_v2.py` — the leading underscore matters. `extract_sid` is `stem.split('_').next()`
and returns `None` only for an empty first token, as its own test pins; `scratch_v2.py`
would yield a student keyed `scratch`. The `_`-split rule is a placeholder P-673 replaces,
so these expectations are provisional.

Canvas-only unit tests (unreachable from the equivalence fixture): `sis_user_id: null` →
retained under `CanvasUser`, `MissingStudentNumber`; unquoted `"sis_user_id": 24010003` →
deserialize error; no-roster → every entry `NoRoster`, zero `NotSubmitted`; a Canvas value
with a trailing space matching a trimmed CSV value.

The local adapter test copies the fixture tree into a `tempfile::tempdir()` and builds the
zip there. `discover_submissions` writes `.scriptmark_extracted/` next to its input
(`discovery.rs:39`), which is gitignored and would accumulate invisibly in the source tree,
its stale-skip would freeze what later runs see, and parallel `#[test]` threads would race
on one extraction directory.

Two further tests: a legacy `results.json` still deserializes with
`submission_state.is_none()`; and running the local adapter twice over the same tempdir
produces byte-identical `serde_json::to_string(&input)`.

## Touch list

| File | Change |
|---|---|
| `models/submission.rs` | The model above; delete `SubmissionSet` |
| `models/config.rs` | `AssignmentInfo` += canvas ids + `attempt_policy` |
| `models/result.rs` | `StudentReport` += `canvas_user_id`, `submission_state` (both `Option`); `derive(Default)` |
| `roster.rs` | `Roster { entries, diagnostics }`, `RosterEntry`, `lookup() -> Unique/Ambiguous/Missing` |
| `discovery.rs` | Local adapter → `AssignmentInput`; archive manifest for provenance; no silent `continue` |
| `input/mod.rs`, `input/canvas.rs` | Canvas payload structs + pure `normalize()` |
| `grading.rs` | Skip non-`Executable`; two `StudentReport` literals |
| `runner/orchestrator.rs` | `run_all(&[StudentSubmission], …) -> Vec<StudentReport>`, input order |
| `runner/oracle.rs` | `StudentFile::direct()` |
| `main.rs` | grade/run via adapter; `--assignment`; `cmd_summarize`; both import-roster sites; `cmd_grades_push` on `canvas_user_id`; print diagnostics |
| `db/roster.rs` | `import_roster(&Roster)`, count stored rows, populate `canvas_id` |
| `db/results.rs` | Reject duplicate `student_id` before write; fix `avg_grade` denominator |
| `db/mod.rs` | 2 `import_roster` call sites, 3 `StudentReport` literals |
| `scriptmark-py/src/lib.rs` | `run()` returns a list; `discover()` keeps its dict, documented lossy; add `load_input()` |
| `tests/integration.rs` | 12 `SubmissionSet` + 15 `StudentFile` literals, 28 map assertions |
| `crates/scriptmark/tests/input_equivalence.rs` | New |
| `.gitignore` | `!crates/scriptmark/tests/fixtures/**` |

Ergonomics matter here — 40-odd construction sites get rewritten, and if building a
one-student input in a test is painful, P-673/P-674 will rewrite these files again.
So: `StudentFile::direct(path, language)`, `StudentSubmission::from_files(key, paths)`,
`derive(Default)` on `StudentReport`, `by_id(&[StudentReport], &str)`, and
`Roster::from_pairs(&[(id, name)])`.

## Release note

`models/mod.rs` does `pub use submission::*`, so `SubmissionSet`, `StudentFile`'s field set
and `run_all`'s signature are all public API of the crate published at 0.2.0, and
`scriptmark.run()` changes from dict to list. Bump both `Cargo.toml` and `pyproject.toml`
to 0.3.0 and update the README's Python example.

## Verification — the exact CI commands

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                       # workspace root, all members + doctests
```

`-p scriptmark` is not enough: it never compiles `scriptmark-py`, which calls both
`discover_submissions` and `run_all`. Locally `scriptmark-py` cannot build because the
machine's Python is 3.14 and pyo3 0.24 tops out at 3.13 (pre-existing, unrelated), so the
local substitute is
`PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo clippy --all-targets -p scriptmark-py -- -D warnings`.
