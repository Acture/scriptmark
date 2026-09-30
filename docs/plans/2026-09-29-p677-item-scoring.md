# P-677 — Score by grading item; tell student, teacher and environment errors apart

Linear: https://linear.app/acturea/issue/P-677
Parent: P-663 · Depends on: P-674 (merged) · Hands off to: P-678, P-680, P-673

Revision 2. Revision 1 (`50c0bf8`) came out of a design workflow: three code maps,
independent designs, an adversarial judge, and a synthesis that re-read the code. Two of the
three designs failed to return, so only the risk-first one was judged. Revision 1 then went
through a six-lens adversarial review (acceptance, fault attribution, wrong-number paths,
Rust feasibility, scope, persistence), with a skeptic refuting each lens. 17 findings
survived. What changed, and why:

- **Old results and old databases are not read.** Owner decision D-e. This removes
  Revision 1's `UnknownFault` reason, its legacy display state, and the DB migration. The
  migration was also broken: a fresh database would have run `ALTER TABLE ADD COLUMN` on
  columns `CREATE TABLE` had just made.
- **`summarize` shows stored results; it no longer re-scores.** Re-scoring with a
  different or missing `assignment.toml` silently changed grades (D12).
- **A missing item file withholds by default.** Owner decision D-f (D4).
- **A checker that errors on a student's value is the student's wrong answer.** Owner
  decision D-g. Before, a common wrong answer such as `None` reaching `len(result)` would
  have withheld much of a class (D5, D15).
- **A curve is shown next to the raw grade.** Owner decision D-h (D8).
- **Lint has three outcomes, not two.** Revision 1 withheld a student who forgot the file
  with `LintUnavailable`, and still gave full lint points when the tool started but failed
  (D10).
- **A missing-policy zero carries its reason into every export.** Otherwise it was
  indistinguishable from a wrong-answer zero (D9, D11).
- **Consumers colour and chart by fraction of `scale`**, not by a fixed 0–100 (D14).
- **Rust fixes:** an explicit `Default` for `GradingConfig`, full derives on the config
  types, and the `GradingItem` literal in `submission.rs:1128`.

## Owner decisions

2026-09-29:

- **D-a** NotSubmitted: explicit `[grading] missing` policy, default `withheld`. Canvas
  excused is always withheld.
- **D-b** An item that declares no points is worth 1. The total is reported as
  `score/max` and scaled to `[grading] scale` (default 100).
- **D-c** The implicit 0.9/0.1 lint blend goes. Lint counts only through an explicit
  policy field.
- **D-d** Points and in-item aggregation live in `assignment.toml [[items]]`.

2026-09-30:

- **D-e** Old result files and old databases are not supported. Reading one is refused
  with a message; there is no compatibility path.
- **D-f** A student who submitted but lacks an item's file: that item is withheld by
  default. `missing_file = "zero"` opts into 0.
- **D-g** A checker that errors while judging a student's value — function, Rhai or
  Python script — is the student's wrong answer, not a teacher fault. Only teacher code
  failing to load or be configured is the teacher's.
- **D-h** Whether a curve applies is configuration. When it does, the raw grade and the
  curved grade are both shown and exported.
- **D-i** The `--grading`, `--formula` and `--range` flags are removed.

Settled by the issue itself: the default policy is raw `score/max`; curves and formulas are
opt-in; a teacher or environment fault leaves the student ungraded, never 0, while the rest
of the batch is graded; an item's max does not depend on its case count. Within an item the
default is its pass rate: `points × passed / cases`.

## Scope

In: scoring per item, zero versus withheld with a reason, the policy in `assignment.toml`,
and making every consumer — JSON, CSV, DB, terminal, TUI, HTML, Python, Canvas push —
tell a zero from a withheld grade.

Not in:

| Ticket | Owns |
|---|---|
| P-673 | matching rules; ambiguous matches feed `PendingReview` later |
| P-675 | random case generation (unaffected: more cases never change an item's max) |
| P-676 | reference oracle |
| P-678 | versioned result contract, regrade revisions, re-scoring stored evidence |
| P-680 | push preview, receipts, checking the scale against Canvas `points_possible` |

## Facts checked against the code

- **`LintConfig`** lives in `models/spec.rs:12-21`. Its `weight` (default 0.1) is never
  read. The 5 tests in `linter.rs` build it literally.
- **`run_lint`** (`linter.rs:35-58`) ignores the exit status and counts stdout lines. A
  tool that starts and fails scores 100. `lint()` (`orchestrator.rs:217-224`) lints
  `files.first()`, whichever item that file belongs to.
- **Environment startup failure produces real cases.** `tests/integration.rs:861-875`
  pins `(Error, Environment, Spawn)` for `/nonexistent/python3`.
- **The runner cannot produce a `TestResult` with zero cases.** `spec_loader.rs:280`
  refuses an empty scenario, `:642` refuses `parametrize.count < 1`.
- **NoFile** is `(Error, Student, NoFile)` for every case of the bundle, and leaves
  `graded_files[b] = None` (`orchestrator.rs:134-149`).
- **Checker errors are teacher faults today:**
  - function checker: `judge.rs:686-693`;
  - Rhai: `rhai_checker.rs:94-110`;
  - Python script checker: `python_checker.rs:85-105` (timeout, non-zero exit with no
    stdout, unparseable verdict); a lost child is Environment.
  - Rhai checks are dry-run against the expectation before grading
    (`prepare.rs` `dry_run_rhai`); function and script checkers are not.
- **Breaking call sites:**
  - `apply_grading`/`GradingPolicy`: `main.rs:665,857`, `scriptmark-py/src/lib.rs:239-244`,
    `tests/integration.rs:275,297`, `db/mod.rs:327`.
  - `CourseConfig.grading` (`config.rs:57`) and `test_load_course_config`
    (`spec_loader.rs:855`).
  - `is_gradeable`: `main.rs:1054`, `tests/integration.rs:264,270`,
    `tests/input_equivalence.rs:350`, `db/mod.rs:325`. (`archive::is_gradeable` is
    unrelated.)
  - `GradingItem { .. }` literal: `models/submission.rs:1128`.
  - `db sessions` and `db history` print `avg_grade` and `pass_rate` (`main.rs:1236-1286`).
- **Rhai helpers:** `rhai_checker::engine()` sets operation, call-depth and expression
  limits (`rhai_checker.rs:49-52`). `rhai_checker::compile` returns `()`, so the policy
  compiles its own AST with that engine.
- **The sqrt template** is `lower + (upper-lower)/10 * sqrt(rate%)` (`grading.rs:37-40`),
  which is `lower + (upper-lower) * sqrt(fraction)`.

## Decisions

**D1. An item's max is its declared points.**
- `GradingItem` gains `points: u32` (default 1, D-b) and `aggregation: Aggregation`:
  - `proportional` (default): `score = points × passed / cases`;
  - `all_or_nothing`: `points` if every case passed, else 0.
- The number of cases never enters an item's max.
- `u32` keeps the `Eq`/`Ord` derives on `GradingItem`, `Assignment` and
  `AssignmentInput`, and rules out NaN points.

**D2. Items are declared in TOML, or derived.**
- `[[items]]` deserialises into `ItemDecl` (`deny_unknown_fields`, `aggregation`
  required), which converts to `GradingItem`. `GradingItem` itself keeps serde defaults
  for its new fields, because `Assignment` JSON and the literal constructors use it.
- With no `assignment.toml`, items are derived from the specs at 1 point, proportional.
  `basis.derived_items = true`, and the derived `[[items]]` block is printed to stderr so
  the teacher can paste it.

**D3. Policy lives in `assignment.toml [grading]`.**
- `GradingPolicy`, `TemplatePolicy`, `FormulaPolicy`, the sqrt default,
  `build_grading_policy`, the three flags (D-i) and `CourseConfig.grading` are deleted.
- The default is `curve = raw`: `score / max × scale`, `scale` defaulting to 100.
- `deny_unknown_fields` on `AssignmentConfig`, `AssignmentInfo`, `GradingConfig`,
  `ItemDecl`, so `missing = "witheld"` or `pionts` is refused, not ignored.
- `GradingConfig` has a hand-written `Default` equal to its serde defaults, and a test
  that `[grading]` omitted, `[grading]` empty and `GradingConfig::default()` are equal.

**D4. Missing work.**
- `missing` covers NotSubmitted and SubmittedEmpty: `withheld` (default, D-a) or `zero`.
- `missing_file` covers an item whose every case is `cause == NoFile`: `withheld`
  (default, D-f) or `zero`.
- Excused is always `Withheld(Excused)`, whatever the policy and whatever the evidence.
- A zero from either policy is `Graded` with `zero_reason` set (D9), so every export can
  tell it from a wrong-answer zero.

**D5. Item classification.** In declaration order; the first matching rule decides.
1. No `TestResult` for the item, or more than one: refuse the batch. The runner makes
   exactly one per spec and specs are unique (D13); anything else is a bug.
2. A non-passed case with no `fault`: refuse the batch, same reason.
3. Any non-passed case with `Fault::Teacher`: `Withheld(TeacherFault)`.
4. Any non-passed case with `Fault::Environment`: `Withheld(EnvironmentFault)`.
5. Every case is `NoFile`: the `missing_file` policy.
6. Otherwise score it. Every remaining non-pass is the student's.

`blocking_case` and `blocking_cause` name the case that decided a withheld item.

**D6. A withheld item withholds the student.**
- `grade = Withheld`, `final_grade = None`, `reason` = the first withheld item's reason in
  declaration order. Per-item scores are kept.
- Other students are unaffected.
- A partial total would reach Canvas looking like a real low grade, so there is none.

**D7. Student-level gates, before items, in order:**
1. excused → `Excused`
2. `error` set → `GradingTaskFailed`
3. `ReceivedUnmatched` → `PendingReview`
4. NotSubmitted / SubmittedEmpty → the `missing` policy
5. Executable → item classification (D5)

`submission_state` becomes required on `StudentReport` (D-e).

**D8. Curves are explicit, and shown beside the raw grade (D-h).**
- `curve` is `raw` (default), `template` (`linear`, `sqrt`, `log`, `strict`, with
  `0 <= lower <= upper <= scale`), or `formula`.
- A graded student always has `raw_grade = round(fraction × scale)`. `final_grade` is the
  curved value, equal to `raw_grade` under `raw`.
- The formula is compiled once, before any student runs, with `rhai_checker::engine()`.
  Its variables are `score`, `max`, `fraction`, `scale`. A compile error refuses the run.
- A runtime error, a non-number, a non-finite value or a value outside `[0, scale]`
  withholds that student with `FormulaError`. Nothing is clamped.
- A policy zero (D4) bypasses the curve: its `raw_grade` and `final_grade` are both 0.

**D9. Rounding and the result fields.**
- `raw_grade` and `final_grade` are rounded half away from zero to `decimals` (default 2,
  range 0–4). Item scores and `score` stay unrounded in JSON; the grades CSV writes them
  to `decimals` too.
- `StudentReport` gains:

  ```rust
  pub grade: GradeState,                 // required: Graded | Withheld
  pub reason: Option<Reason>,            // why withheld, or why a graded 0 is a policy 0
  pub score: Option<f64>, pub max: f64,  // unrounded
  pub raw_grade: Option<f64>,
  pub final_grade: Option<f64>,          // None exactly when Withheld
  pub items: Vec<ItemScore>,
  pub basis: GradeBasis,                 // scale, decimals, curve, derived_items
  ```

- `final_grade` is `None` exactly when `grade == Withheld`.

**D10. Lint counts only through `[grading] lint_points = N` (D-c).**
- The blend and `LintConfig.weight` go.
- `lint_points` adds `lint_points × lint_score / 100` to `score` and `lint_points` to
  `max`. It is refused when no spec declares `[lint]`.
- Lint runs on the file of the item whose spec declares `[lint]`, and has three outcomes:
  - `Scored(f64)`;
  - `NoFile`: that item is NoFile, so the student is already covered by `missing_file` —
    withheld, or 0 lint points under `missing_file = "zero"`;
  - `Failed(String)`: spawn failure, join error, empty command, or an exit code outside
    `[lint] ok_exit_codes` (default `[0, 1]`, since ruff and flake8 exit 1 on findings) →
    `Withheld(LintFailed)`.

**D11. Push is by state, never by number.**
- `export::grades_to_push(&[StudentReport]) -> Result<PushSet>` pushes only `Graded`
  students with a `canvas_user_id`, including real zeros.
- It returns skip counts per reason and refuses two reports sharing a `canvas_user_id`
  (a `HashMap::insert` would silently lose one).
- `push_grades` takes a `BTreeMap<u64, f64>`.
- Checking `basis.scale` against Canvas `points_possible` is P-680's.

**D12. `summarize` displays what `grade` stored.** It loses the policy flags and never
re-scores. Re-scoring stored evidence is P-678's.

**D13. Assignment loading is library code.**
- `load_assignment`, `reconcile_items` and `validate` move from `main.rs` to
  `scriptmark::assignment`, shared by `grade`, `run` and the Python `grade()`.
- `reconcile_items` refuses an orphan item or orphan spec.
- `load_specs_from_dir` refuses a duplicate `[meta] name`.
- Everything runs before `run_bundles`. Problems are collected into one
  `refusing to grade:` error:
  1. unknown key or enum value (serde);
  2. duplicate item id;
  3. orphan item or spec;
  4. duplicate `[meta] name`;
  5. total points 0;
  6. `scale` not finite or `<= 0`;
  7. `decimals > 4`;
  8. template bounds not finite or outside `0 <= lower <= upper <= scale`;
  9. formula does not compile;
  10. `lint_points` with no `[lint]`; empty lint command.

**D14. Consumers read, never recompute.**
- Withheld shows as `-` plus its reason; a graded 0 shows as `0` plus its `reason` when
  it is a policy zero.
- Colour thresholds and the HTML histogram use `final_grade / basis.scale`.
- Averages count graded students only, and are `-` when there are none.

**D15. A checker erroring on a student's value is the student's (D-g).**
- `CheckError` becomes `Student` for: a function checker that raised; a Rhai check that
  fails to evaluate or does not return a bool; a Python script checker that times out,
  exits non-zero without a verdict, or prints no verdict. The cause stays `Checker`,
  status `Failed`.
- A Python checker that cannot be spawned or is lost stays Environment. Teacher module
  import and setup failures stay Teacher.
- After grading, any item where every executable student failed at least one case with
  `Cause::Checker` prints a warning naming the checker: that is usually a teacher bug.
  So does an item where every executable student is NoFile (a wrong `[meta] file`). Both
  are diagnostics; no student's grade depends on another's.

## Types

```rust
// models/submission.rs
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation { #[default] Proportional, AllOrNothing }

pub struct GradingItem {                        // derives unchanged
	pub id: String,
	#[serde(default)] pub title: Option<String>,
	#[serde(default = "one")] pub points: u32,
	#[serde(default)] pub aggregation: Aggregation,
}

// models/config.rs — all Debug, Clone, Serialize, Deserialize
#[serde(deny_unknown_fields)]
pub struct ItemDecl { id, title: Option<String>, #[serde(default = "one")] points: u32, aggregation: Aggregation }
pub enum MissingPolicy { Withheld, Zero }        // snake_case
pub enum CurveTemplate { Linear, Sqrt, Log, Strict }
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Curve { Raw, Template { name: CurveTemplate, lower: f64, upper: f64 }, Formula { formula: String } }
#[serde(default, deny_unknown_fields)]
pub struct GradingConfig { missing, missing_file: MissingPolicy, scale: f64, decimals: u8, curve: Curve, lint_points: Option<u32> }
impl Default for GradingConfig { /* withheld, withheld, 100, 2, Raw, None */ }

// models/spec.rs
pub struct LintConfig { command, max_warnings, #[serde(default = "zero_one")] ok_exit_codes: Vec<i32> }  // weight removed

// models/result.rs
pub enum GradeState { Graded, Withheld }
pub enum Reason { NotSubmitted, SubmittedEmpty, Excused, PendingReview, GradingTaskFailed,
	MissingFile, TeacherFault, EnvironmentFault, FormulaError, LintFailed }
pub enum LintOutcome { Scored(f64), NoFile, Failed(String) }
pub struct ItemScore { item_id, points: u32, aggregation, state: GradeState,
	score: Option<f64>, reason: Option<Reason>, passed: usize, cases: usize,
	blocking_case: Option<String>, blocking_cause: Option<Cause> }
pub struct GradeBasis { scale: f64, decimals: u8, curve: Curve, derived_items: bool }
// StudentReport: fields from D9; lint_score -> lint: Option<LintOutcome>;
// submission_state required; is_gradeable removed; `spec_name` alias dropped.

// grading.rs
pub struct CompiledPolicy { config: GradingConfig, derived_items: bool, engine: Engine, ast: Option<AST> }
impl CompiledPolicy { pub fn compile(config: &GradingConfig, derived_items: bool) -> Result<Self> }
pub fn grade_all(reports: &mut [StudentReport], items: &[GradingItem], policy: &CompiledPolicy) -> Result<()>;
pub fn diagnostics(reports: &[StudentReport], items: &[GradingItem]) -> Vec<String>;  // D15 warnings

// export.rs (new)
pub struct PushSet { pub grades: BTreeMap<u64, f64>, pub skipped: BTreeMap<String, usize> }
pub fn grades_to_push(reports: &[StudentReport]) -> Result<PushSet>;
pub fn write_grades_csv<W: Write>(reports: &[StudentReport], items: &[GradingItem], w: W) -> Result<()>;
```

## `assignment.toml`

```toml
[assignment]
name = "hw3"

[grading]                 # optional; these are the defaults
missing = "withheld"      # "withheld" | "zero"
missing_file = "withheld" # "withheld" | "zero"
scale = 100
decimals = 2
curve = { kind = "raw" }
# curve = { kind = "template", name = "sqrt", lower = 60, upper = 100 }
# curve = { kind = "formula", formula = "fraction * scale" }
# lint_points = 1

[[items]]
id = "stats"              # a spec's [meta] name
points = 3
aggregation = "proportional"
```

## Scoring (`grade_all`)

Each report independently, from scratch — the pass is idempotent:

1. `max = Σ points + lint_points.unwrap_or(0)`.
2. Student gates (D7). A gate withholds and stops, except a policy zero: `Graded`,
   `score = 0`, `raw_grade = final_grade = 0`, `reason` set, each item a graded 0 with
   that reason.
3. Classify each declared item (D5). A scored item's score follows its aggregation.
4. Lint (D10), when `lint_points` is declared.
5. Any withheld item → student withheld (D6).
6. `score = Σ`, `fraction = score / max`, `raw_grade = round(fraction × scale)`.
7. Curve:

   | Curve | `final_grade` before rounding |
   |---|---|
   | raw | `fraction × scale` |
   | linear | `lower + fraction × (upper − lower)` |
   | sqrt | `lower + √fraction × (upper − lower)` |
   | log | `lower + ln(1 + 100·fraction) / ln 101 × (upper − lower)` |
   | strict | `upper` at 1; `lower + (fraction − 0.8)/0.2 × (upper − lower)` from 0.8; else `lower` |
   | formula | the compiled AST in a fresh scope; an int becomes f64 |

8. Out of `[0, scale]` or not finite → `Withheld(FormulaError)`. Otherwise round and
   mark `Graded`.

A 0 can only come from a policy zero, student-fault evidence, or a declared curve. Teacher,
environment, formula and lint-tool problems always give `None` with a reason.

Items and cases are walked in `Vec` order: no hash iteration, clock or randomness.

## Consumers

- **`main.rs`**
  - `grade`: load → specs → reconcile and validate → compile → `run_bundles` →
    `grade_all` → diagnostics → outputs. `save_session` stores the `GradingConfig` and
    items as JSON.
  - `run`: validates and compiles before running.
  - `summarize`: displays stored results (D12).
  - archive CSV: `submission_state` in serde form; the per-student sentinel row reports
    `grade` and `reason`; an unknown `--format` bails.
  - new `grades_{stem}.csv`: `student_id, student_name, canvas_user_id, grade, reason,
    score, max, raw_grade, final_grade`, then `<item>_score, <item>_state,
    <item>_reason` per item. Withheld cells are empty; a zero is `0`.
  - `grades push`: `export::grades_to_push`, skip counts per reason.
  - `db sessions` / `db history`: `-` for no grade; state and reason shown.
- **`display.rs`**: Student, Name, State, Reason, Score (`score/max`), Raw, Grade.
  `display_stats`: graded, zero and withheld-by-reason counts; average over graded.
- **`db`**: the schema gains `grade`, `reason`, `score`, `max_score`, `raw_grade`, and
  sets `PRAGMA user_version = 1`. Opening a database whose `user_version` is not 1 is
  refused (D-e). Row mappers propagate errors instead of `filter_map(ok)`, and numeric
  columns are not defaulted to 0. `avg_grade` and `pass_rate` become `Option`. Order by
  `final_grade DESC NULLS LAST`.
- **`tui/ui.rs`**: State and Score columns; detail pane shows state, reason and
  `score/points` per item.
- **`report_template.html`**: `_graded = grade === 'graded'`; state and reason badge;
  `score/max`; raw beside curved when they differ; chart and colours by fraction of
  scale; withheld sorted after graded.
- **`scriptmark-py`**: `grade(..., assignment=None)` through the shared path; getters
  for `grade`, `reason`, `score`, `max`, `raw_grade`.
- **Docs**: a Scoring section in `docs/test-bundles.md` with the schema, the reason
  table and the zero-versus-withheld rules; README examples; example `assignment.toml`
  files for `examples/bundles/*`.

## Commits (the crate builds and tests pass after each)

1. **Checker attribution (D15).** `CheckError::student`, the three checkers, judge tests.
2. **Scorer.** Model fields, `Aggregation`, `GradingConfig`, `ItemDecl`,
   `CompiledPolicy`, `grade_all`. Delete `GradingPolicy`, the flags and the blend. Fix
   every call site. Old results stop parsing (D-e).
3. **Assignment module and validation (D13).**
4. **Lint (D10).** `LintOutcome`, exit codes, lint on the declaring item's file.
5. **Export and push (D11).** `export.rs`, grades CSV, archive changes.
6. **DB.** New schema, version check, strict mappers, `db` subcommands.
7. **Display, TUI, HTML, Python, diagnostics.**
8. **Docs and example `assignment.toml` files**, with score assertions in
   `tests/examples.rs`.

## Regression tests, by acceptance item

**More random cases do not change an item's max.**
- `grading::more_cases_do_not_change_item_max`: 5 against 50 cases on a 2-point item
  next to a 1-point item; max stays 3 and the score is equal at equal pass fractions.
- `integration::parametrized_count_does_not_change_item_max`: a real spec, `count` 5
  against 50.
- `grading::all_or_nothing_aggregation`: 9/10 → 0, 10/10 → full.

**All correct and partial.**
- `grading::all_correct_and_partial`: all correct → 100; two 1-point items at 3/4 and
  1/1 → 1.75/2 = 87.5.
- `examples.rs`: alice full marks, bob partial.

**Missing.**
- `grading::not_submitted_withheld_by_default` (replaces
  `test_missing_student_gets_zero`).
- `grading::missing_zero_gives_zero_with_reason_and_no_curve`.
- `grading::excused_always_withheld` (under `missing = "zero"`, and with all passing).
- `grading::received_unmatched_is_pending_review`.
- `grading::missing_file_withheld_by_default_zero_by_policy`.
- `grading::lint_on_missing_file_follows_missing_file_policy`.

**Teacher formula error.**
- `grading::formula_that_does_not_compile_is_refused` (replaces
  `test_formula_error_gives_zero`): no report is touched.
- `grading::formula_runtime_error_withholds_only_that_student`.
- `grading::formula_out_of_range_is_withheld_not_clamped`: `1.0/0.0`, `scale + 1`, `-1`,
  `true`.
- `grading::teacher_fault_withholds_student_not_batch`, with `blocking_case`.
- `judge::checker_error_on_student_value_is_student_fault` for all three checker kinds.
- `integration::teacher_module_import_failure_withholds`.

**Environment startup failure.**
- `integration::env_startup_failure_withholds_everyone`: `/nonexistent/python3` →
  `Withheld(EnvironmentFault)` for every student.
- `linter::nonzero_exit_outside_ok_codes_is_failed_not_100`,
  `linter::spawn_failure_is_failed`.
- `grading::lint_failure_withholds_when_lint_points_declared`.
- `grading::task_panic_is_grading_task_failed`.

**Same evidence and policy, same result.**
- `grading::grade_all_is_idempotent_and_order_independent`: byte-identical JSON.
- `grading::rounding_half_away`: 2/3 → 66.67; 0 decimals → 67.
- `grading::curve_keeps_raw_grade`: sqrt at 0.25 → raw 25, final 80.

**Export and Canvas tell a zero from no grade.**
- `export::grades_csv_leaves_withheld_empty_writes_zero_and_reason`.
- `export::push_skips_withheld_and_excused_pushes_explicit_zero`.
- `export::push_refuses_duplicate_canvas_user`.
- `db::zero_and_withheld_round_trip_with_reason`.
- `db::old_database_is_refused`, `db::reopen_is_idempotent`.
- `db::session_average_none_when_nobody_graded`.
- `results::old_result_json_is_refused`.

**Refused before grading.**
- `assignment::validate_refusals`, table-driven over D13's list, plus
  `missing = "witheld"`, `pionts`, and a missing `aggregation`.
- `config::grading_default_matches_serde_default`.
- `spec_loader::duplicate_meta_name_is_refused`.

## Residual risks, accepted

- One teacher or environment fault on one item withholds the whole student. The per-item
  record says which item and case blocked.
- A buggy teacher checker now fails students instead of withholding them. The D15
  warning names it; the teacher fixes the checker and regrades.
- Removing the flags and `LintConfig.weight`, and refusing old results and databases, are
  breaking changes by decision.
- The pushed scale is not checked against Canvas `points_possible`; that is P-680's.
