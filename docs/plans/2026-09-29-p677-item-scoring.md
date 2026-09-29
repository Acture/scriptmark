# P-677 — Score by grading item; tell student, teacher and environment errors apart

Linear: https://linear.app/acturea/issue/P-677
Parent: P-663 · Depends on: P-674 (merged) · Hands off to: P-678, P-680, P-673

Revision 1. Produced by a design workflow (three code maps, independent designs, an
adversarial judge, a synthesis that re-read the code). Two of three designs failed to
return, so only the risk-first design was judged; this revision goes through a separate
adversarial review before any code is written.

## Owner decisions (2026-09-29)

- **D-a** NotSubmitted: explicit `[grading] missing` policy, default `withheld`. Canvas
  excused is always withheld.
- **D-b** An item that declares no points is worth 1. The total is reported as
  `score/max` and scaled to `[grading] scale` (default 100).
- **D-c** The implicit 0.9/0.1 lint blend goes. Lint counts only through an explicit
  policy field.
- **D-d** Points and in-item aggregation live in `assignment.toml [[items]]`.

Settled by the issue itself: the default policy is raw `score/max`, curves and formulas are
opt-in, a teacher or environment fault leaves the student ungraded — never 0 — and an
item's max does not depend on its case count.

## Corrections checked against the code

- **`LintConfig` location.** It lives in `models/spec.rs:12-21`, including `weight`, whose default of 0.1 is never read. It is not in `runner/linter.rs`. The 5 tests in `linter.rs` build the struct literally.
- **Environment startup failure produces real cases.** `tests/integration.rs:861-875` pins `(Error, Environment, Spawn)` for `with_python_cmd("/nonexistent/python3")`. The expected state is `Withheld(EnvironmentFault)`, not NoEvidence.
- **The runner cannot produce a TestResult with zero cases.**
  - `spec_loader.rs:280` refuses a scenario with no steps.
  - `:642` refuses `parametrize.count < 1`.
  - NoEvidence is therefore reachable only from hand-built or legacy JSON.
- **`StudentReport::is_gradeable` callers:**
  - `main.rs:1054`
  - `tests/integration.rs:264,270`
  - `tests/input_equivalence.rs:350`
  - `db/mod.rs:325`
  - `archive::is_gradeable` (`archive.rs:103`) is an unrelated function and is not touched.
- **Call sites that break when `apply_grading` and `GradingPolicy` go:**
  - `main.rs:665,857`
  - `scriptmark-py/src/lib.rs:239-244`
  - `tests/integration.rs:275,297`
  - `db/mod.rs:327`
  - `CourseConfig.grading` (`config.rs:57`) and `test_load_course_config` (`spec_loader.rs:855`)
- **`Assignment` is Serialize/Deserialize and built literally** at `input/canvas.rs:388,989` and `main.rs:407`. A new required serde field on `GradingItem` would break those.
- **No `points_possible` exists anywhere** in the code.
- **Reusable Rhai helpers exist:** `rhai_checker::compile(expr, names) -> Result<(), String>` and `rhai_checker::engine()`.
- **The current sqrt template is not the curve the old design described.** It computes `lower + (upper-lower)/10 * sqrt(rate%)` (`grading.rs:37-40`). The ported template has to keep this formula in fraction form, `lower + (upper-lower)*sqrt(fraction)`, which is the same thing.

## Decisions

**D1. An item's max is its declared points.**
- `GradingItem` gains `points: u32` (default 1, per D-b) and `aggregation: Aggregation`, which is `proportional` or `all_or_nothing`.
- Scoring:
  - `proportional`: `score = points * passed / counted`.
  - `all_or_nothing`: `score = points` only if every case passed, else 0.
- The number of cases never enters an item's max.
- Rationale: this is the core acceptance item. u32 keeps the `Eq`/`Ord` derives on `GradingItem`, `Assignment` and `AssignmentInput`, and rules out NaN points.

**D2. Aggregation is chosen explicitly in TOML. Derived items use a documented default.**
- The TOML-facing type is a separate `ItemDecl` with `deny_unknown_fields`, in which `aggregation` is required.
  - A missing aggregation is a serde error.
  - `ItemDecl` converts into `GradingItem`.
- The model struct `GradingItem` keeps `#[serde(default)]` for both new fields, so stored `Assignment` JSON and the literal constructors keep working.
- With no `assignment.toml`:
  - Items are derived at 1 point each, proportional.
  - `basis.derived_items = true`.
  - The derived `[[items]]` block is printed to stderr so the teacher can paste it into a file.
- Rationale: D-b sanctions the 1-point default. A derived default that is documented and recorded is not "implied by case count".

**D3. Policy lives in `assignment.toml [grading]`. The CLI flags go.**
- `GradingPolicy`, `TemplatePolicy`, `FormulaPolicy` and the sqrt `Default` are deleted.
- `--grading`, `--formula` and `--range` are removed from `GradeArgs` and `SummarizeArgs`, together with `build_grading_policy`.
- The default is `curve = raw`: `score / max * scale`, with `scale` defaulting to 100 (D-b).
- `deny_unknown_fields` goes on `AssignmentConfig`, `AssignmentInfo`, `GradingConfig` and `ItemDecl`.
- `CourseConfig.grading` is removed. Its loader is never called by the CLI, and its test is updated.
- Rationale: one recorded source of policy is what makes "same evidence + policy gives the same result" hold. Typos such as `missing = "witheld"` or `pionts` are refused instead of silently dropped.

**D4. Missing policies.**
- `missing` covers NotSubmitted and SubmittedEmpty. It is `withheld` (default, D-a) or `zero`.
- Excused is always Withheld(Excused), even under `missing = "zero"` and even with an all-pass submission (D-a).
- `missing_file` covers an item whose cases are all `cause == NoFile`. It is `zero` (default) or `withheld`.
  - The default is zero because NoFile carries `Fault::Student` and fault is authoritative.
  - The scorer never moves the blame from Student to Teacher.
- Wrong-pattern guard: if every Executable student is NoFile on an item, grading prints a warning naming `[meta] file` and the item. It is a diagnostic only, so one student's result never depends on the rest of the batch.
- Rationale: a forgotten file is an ordinary student mistake and must produce a defined score. The warning covers the case where the teacher's file pattern is wrong.

**D5. Item classification, in fixed order, trusting fault. The first matching rule decides.**
1. No TestResult for the item: NoEvidence.
2. More than one TestResult for the item: DuplicateEvidence.
3. The TestResult has zero cases: NoEvidence.
4. Any non-Passed case with Teacher fault: TeacherFault.
5. Any non-Passed case with Environment fault: EnvironmentFault.
6. Any non-Passed case with no fault, or any Passed case that carries a fault: UnknownFault (legacy or inconsistent evidence; it is never treated as a student fault).
7. Every case is NoFile: apply `missing_file`.
8. Otherwise the item is scored. Every remaining non-pass is a student fault (Wrong, Raised, Timeout, NoTarget, Syntax, and NotRun inherited from a student culprit).

Rationale: the P-674 contract makes fault authoritative, and `result.rs:99-100` says a missing fault must not be read as "no fault". The fixed order gives deterministic output.

**D6. A withheld item withholds the student.**
- `grade_state = Withheld` and `final_grade = None`.
- `withheld_reason` is the reason of the first withheld item in declaration order.
- Per-item states are kept, with `blocking_case` and `blocking_cause`.
- Other students are unaffected.
- Rationale: the issue says the student stays ungraded, never 0. A partial total would reach Canvas looking like a real low grade.

**D7. Student-level gates are checked before items, in this order:**
1. excused: Excused
2. `error` is set: GradingTaskFailed
3. ReceivedUnmatched: PendingReview (this is the only pending-review hook; P-673 maps ambiguous matches onto it later)
4. NotSubmitted or SubmittedEmpty: the `missing` policy
5. `submission_state` is None (legacy) or Executable: fall through to item classification

**D8. Curves and formulas only by explicit declaration. Formulas compile once and are never clamped.**
- `curve` is `raw` (default), `template` (linear, sqrt, log or strict, with `0 <= lower <= upper <= scale`), or `formula`.
- The formula is compiled into an AST once, in `CompiledPolicy::compile`, before `run_bundles`.
  - The variables are `score`, `max`, `fraction` and `scale`.
  - A compile failure refuses the run, so no student is graded.
- A runtime error, a result that is not a number, a non-finite result, or a result outside `[0, scale]` withholds that student only, with FormulaError. The value is never clamped.
- `missing = "zero"` bypasses the curve, so it gives a real 0.
- Rationale: this fixes the batch-wide zero bug and the hidden clamp to 100.

**D9. Round once.**
- `final_grade = round_half_away(scaled, decimals)`.
- `decimals` defaults to 2, with a range of 0 to 4.
- The unrounded value is kept in `basis.unrounded`.
- Every consumer reads `final_grade` as is and never recomputes.

**D10. Lint counts only through `[grading] lint_points = N` (D-c).**
- The 0.9/0.1 blend and `LintConfig.weight` are deleted.
- There is no synthetic item: `score = Σ item scores + lint_points * lint_score / 100` and `max = Σ points + lint_points`.
- `run_lint` returns a `Result`. A spawn failure, a spawn-blocking join error or an empty command gives `lint_score = None`, which gives Withheld(LintUnavailable) when lint points are declared.
- Lint runs on the graded file of the bundle whose spec declares `[lint]`, not on `files.first()`.
- Validation refuses `lint_points` when no spec has `[lint]`.

**D11. Push is reason-aware and cannot send a wrong scale.**
- The filter lives in a library function, `export::grades_to_push`. It pushes only students with `grade_state == Graded` and a `canvas_user_id`.
- It reports skip counts per reason: no Canvas user, excused, withheld(reason), legacy.
- It refuses when:
  - a report has `final_grade` set but no `grade_state` (legacy, no re-derivable basis), or
  - two reports share a `canvas_user_id` (`HashMap::insert` would silently overwrite).
- An explicit zero is pushed.
- Checking the pushed scale against Canvas `points_possible` is P-680's; `basis.scale` is
  recorded so P-680 can do it without re-deriving.

**D12. Legacy results.**
- Loading never re-scores.
- Consumers show `final_grade` set with no `grade_state` as "legacy", never as graded, and push refuses it.
- `summarize` re-scores, so legacy failed cases with no fault become UnknownFault. This is intended, and the notice is printed.

**D13. Assignment loading is library code.**
- `load_assignment`, `reconcile_items` and `validate_assignment` move from `main.rs` into `scriptmark::assignment`.
- `reconcile_items` returns a `Result`: an orphan spec or an orphan item becomes a hard refusal.
- `grade`, `run`, `summarize` and the Python `grade()` share this path.
- `load_specs_from_dir` refuses duplicate `[meta] name`. DuplicateEvidence stays as the scorer's second line of defence.

**D14. Scope is trimmed.**
- No duplicate student_id refusal in `grade_all` (that is P-673's).
- No PushPlan or SkipReason structure.
- TUI, HTML and Python only read the new fields: they show withheld as a dash with its reason and never recompute.
- `GradeBasis` is kept minimal, and P-678 hoists it into the versioned contract.

## Types

```rust
// models/submission.rs
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation { #[default] Proportional, AllOrNothing }

pub struct GradingItem {          // derives unchanged (Eq, Ord)
    pub id: String,
    #[serde(default)] pub title: Option<String>,
    #[serde(default = "one")] pub points: u32,
    #[serde(default)] pub aggregation: Aggregation,
}

// models/config.rs
#[derive(Deserialize)] #[serde(deny_unknown_fields)]
pub struct ItemDecl { pub id: String, #[serde(default)] pub title: Option<String>,
    #[serde(default = "one")] pub points: u32, pub aggregation: Aggregation }   // -> GradingItem

pub enum MissingPolicy { Withheld, Zero }                 // snake_case
pub enum CurveTemplate { Linear, Sqrt, Log, Strict }
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Curve { #[default] Raw, Template { name: CurveTemplate, lower: f64, upper: f64 }, Formula { formula: String } }

#[serde(deny_unknown_fields)]
pub struct GradingConfig {
    #[serde(default)] pub missing: MissingPolicy,           // Withheld
    #[serde(default = "zero_policy")] pub missing_file: MissingPolicy, // Zero
    #[serde(default = "hundred")] pub scale: f64,
    #[serde(default = "two")] pub decimals: u8,
    #[serde(default)] pub curve: Curve,
    #[serde(default)] pub lint_points: Option<u32>,
}
// AssignmentConfig (deny_unknown_fields): items: Vec<ItemDecl>, #[serde(default)] grading: GradingConfig

// models/result.rs
pub enum GradeState { Graded, Withheld }
pub enum WithheldReason { NotSubmitted, SubmittedEmpty, Excused, PendingReview, GradingTaskFailed,
    MissingFile, TeacherFault, EnvironmentFault, UnknownFault, NoEvidence, DuplicateEvidence,
    UnexpectedEvidence, FormulaError, LintUnavailable }

pub struct ItemScore {
    pub item_id: String, pub points: u32, pub aggregation: Aggregation, pub state: GradeState,
    #[serde(default)] pub score: Option<f64>,          // unrounded; None when withheld
    #[serde(default)] pub reason: Option<WithheldReason>, // also set on a graded 0 via missing policy
    pub passed: usize, pub counted: usize,
    #[serde(default)] pub blocking_case: Option<String>,
    #[serde(default)] pub blocking_cause: Option<Cause>,
}
pub struct GradeBasis { pub scale: f64, pub decimals: u8, pub curve: Curve,
    pub derived_items: bool, pub unrounded: Option<f64>,
    #[serde(default)] pub formula_error: Option<String> }

// StudentReport new fields, all #[serde(default)]:
pub grade_state: Option<GradeState>,      // None = legacy/unscored
pub withheld_reason: Option<WithheldReason>,
pub score: Option<f64>, pub max: Option<f64>,
pub items: Vec<ItemScore>,
pub excused: bool,
pub basis: Option<GradeBasis>,
// final_grade: rounded scaled value; None exactly when not Graded.

// grading.rs
pub const FORMULA_VARIABLES: [&str; 4] = ["score", "max", "fraction", "scale"];
pub struct CompiledPolicy { config: GradingConfig, derived_items: bool, engine: rhai::Engine, ast: Option<rhai::AST> }
impl CompiledPolicy { pub fn compile(c: &GradingConfig, derived_items: bool) -> Result<Self, Vec<String>>; }
pub fn grade_all(reports: &mut [StudentReport], items: &[GradingItem], p: &CompiledPolicy);
pub fn round_half_away(x: f64, d: u8) -> f64;
pub fn all_nofile_items(reports: &[StudentReport]) -> Vec<String>; // D4 warning

// export.rs (new)
pub struct PushSet { pub grades: BTreeMap<u64, f64>, pub skipped: BTreeMap<String, usize> }
pub fn grades_to_push(reports: &[StudentReport]) -> anyhow::Result<PushSet>;
pub fn write_grades_csv<W: Write>(reports: &[StudentReport], items: &[GradingItem], w: W) -> anyhow::Result<()>;
```

`is_gradeable()` is removed. The old tests assert the new states instead. It has no other in-library callers once push moves to `export`.

## Config schema (`assignment.toml`)

```toml
[assignment]
name = "hw3"
canvas_course_id = 123
canvas_assignment_id = 456

[grading]                 # optional; the values shown are the defaults
missing = "withheld"      # "withheld" | "zero"
missing_file = "zero"     # "zero" | "withheld"
scale = 100
decimals = 2
curve = { kind = "raw" }
# curve = { kind = "template", name = "sqrt", lower = 60, upper = 100 }
# curve = { kind = "formula", formula = "fraction * scale" }
# lint_points = 1

[[items]]
id = "stats"              # == a spec's [meta] name
points = 3
aggregation = "proportional"   # required when declared
```

`validate_assignment(&AssignmentConfig, &[TestSpec]) -> Vec<String>` and `CompiledPolicy::compile` run before `run_bundles`. All problems are collected into one refusal, `"refusing to grade: ..."`. The refusals are:
1. An unknown key or enum value (serde).
2. A declared item that omits `aggregation`.
3. Duplicate item ids.
4. An orphan item or an orphan spec.
5. Duplicate `meta.name`.
6. Total points of 0.
7. `scale` is not finite, or `scale <= 0`.
8. `decimals > 4`.
9. Template bounds that are not finite, or that fail `0 <= lower <= upper <= scale`.
10. A formula that fails `rhai_checker::compile`.
11. `lint_points` with no `[lint]` in any spec.
12. An empty lint command.

## Scoring algorithm (`grade_all`)

**Per report.** Each report is processed independently, with no shared state between reports.

1. **Reset** every scoring field. This makes the pass idempotent.
2. **Compute max.** `max = Σ points + lint_points.unwrap_or(0)`.
3. **Apply the student-level gates** from D7. A gate that fires withholds the student with its reason and stops.
   - Under `missing = "zero"`, the student is instead Graded:
     - `score = 0` and `final_grade = Some(0.0)`;
     - each item gets `ItemScore { Graded, score 0, reason NotSubmitted or SubmittedEmpty }`;
     - no curve is applied.
4. **Build the evidence index.** Group `test_results` by `item_id` with a linear scan in `Vec` order. Any `item_id` that is not declared marks the student UnexpectedEvidence.
5. **Classify each declared item** in declaration order (D5). A scored item gets `counted = cases.len()` and `passed = #Passed`, then its aggregation per D1.
6. **Lint.** If `lint_points` is declared:
   - a finite `lint_score` in `[0, 100]` adds `lint_points * lint_score / 100`;
   - anything else withholds with LintUnavailable.
7. **Withhold if needed.** If UnexpectedEvidence is set or any item is Withheld, the student is Withheld. The reason is UnexpectedEvidence, else the first withheld item's reason. `score` and `final_grade` are None, and `items` are kept.
8. **Sum.** `score = Σ` item scores, then `fraction = score / max`.
9. **Apply the curve.**

| Curve | Result |
|---|---|
| raw | `fraction * scale` |
| linear | `lower + fraction * (upper - lower)` |
| sqrt | `lower + sqrt(fraction) * (upper - lower)` |
| log | `lower + ln(1 + 100 * fraction) / ln(101) * (upper - lower)` |
| strict | `upper` if `fraction == 1`; `lower + (fraction - 0.8) / 0.2 * (upper - lower)` if `fraction >= 0.8`; otherwise `lower` |
| formula | evaluate the precompiled AST in a fresh Scope. An int is converted to f64. Anything else withholds with FormulaError, and the message goes in `basis.formula_error`. |

10. **Range guard.** A result that is not finite or lies outside `[0, scale]` withholds with FormulaError. It is never clamped.
11. **Finish.** `final_grade = round_half_away(scaled, decimals)`, `grade_state = Graded`, and `basis` is filled in.

**Where a 0 can come from.** Only these produce a zero:
- `missing = "zero"`
- a NoFile item under `missing_file = "zero"`
- student-fault evidence that scores 0
- a declared curve or formula whose result is 0

Teacher, environment, unknown-fault, formula and lint-tool problems always give None plus a reason.

**Determinism.** Items and cases are walked in `Vec` order. There is no HashMap iteration, no clock and no randomness.

## Consumer changes, file by file

- **`grading.rs`**
  - Rewrite as described above.
  - Delete the blend (`:56-62`), the `Missing → 0` path (`:27,74`) and the eval-error zero (`:96-99`).
- **`models/result.rs`**
  - Add the new fields.
  - Remove `is_gradeable`.
  - Keep `pass_rate` and `status`, documented as informational only.
- **`models/config.rs`**
  - Add `GradingConfig`, `ItemDecl` and `deny_unknown_fields`.
  - Remove `GradingPolicy` and `CourseConfig.grading`.
- **`models/spec.rs`**
  - Remove `LintConfig.weight`.
- **`spec_loader.rs`**
  - Refuse duplicate `meta.name`.
  - Refuse an empty lint command.
  - Update `test_load_course_config`.
- **`assignment.rs`** (new)
  - Holds `load_assignment` (returns `Assignment`, `AttemptPolicy`, `GradingConfig`, `derived: bool`), `reconcile_items -> Result`, and `validate_assignment`.
- **`runner/orchestrator.rs`**
  - Set `report.excused = student.is_excused()` in the spawn closure (`:84-92`) and in the panic branch (`:104-111`).
  - `lint()` takes the graded file of the lint-declaring bundle.
  - A lint `Err` maps to None.
- **`runner/linter.rs`**
  - `run_lint -> Result<LintResult, String>`; a spawn failure or an empty command is `Err`.
- **`main.rs`**
  - Remove the flags and `build_grading_policy`.
  - `cmd_grade` order:
    1. load
    2. specs
    3. reconcile and validate
    4. compile
    5. `run_bundles`
    6. `grade_all`
    7. warn on items where every student is NoFile
    8. outputs
  - `save_session` receives `grading_policy` as JSON of `GradingConfig` plus items.
  - `cmd_run` validates and compiles before running.
  - `cmd_summarize` gains `--tests` (required) and `--assignment`, re-scores, and prints the legacy notice.
  - CSV archive:
    - `submission_state` is written in snake_case serde form.
    - The sentinel row's status comes from `grade_state` and `withheld_reason`.
    - Add `grades_{stem}.csv` via `export::write_grades_csv`, one row per student: `student_id, student_name, canvas_user_id, grade_state, withheld_reason, score, max, final_grade, <item>_score, <item>_state...`.
    - Withheld cells are empty. A zero is written as `0`.
    - An unknown `--format` now bails instead of printing "Archived".
  - `cmd_grades_push`:
    - uses `export::grades_to_push`;
    - prints skip counts per reason.
- **`canvas/client.rs`**
  - `push_grades` takes `&BTreeMap<u64, f64>`.
- **`display.rs`**
  - `display_summary` columns: Student, Name, State, Reason, Score (`score/max` or `-`), Grade (`final_grade` or `-`), Pass (info).
  - Legacy reports show `LEGACY`.
  - `display_failures` labels `fault None` as "unknown fault" and prints the report-level withheld reason and the blocking item.
  - `display_stats` shows graded, zero and withheld-by-reason counts, with the average over graded students only.
- **`db/schema.rs` and `db/results.rs`**
  - The first migration is gated on `PRAGMA user_version < 1` and runs in one transaction:
    - `ALTER TABLE results ADD COLUMN grade_state TEXT`
    - `... withheld_reason TEXT`
    - `... score REAL`
    - `... max_score REAL`
    - set `user_version = 1`
  - `CREATE TABLE` includes the same columns.
  - `ResultRow` gains the four fields.
  - One shared row mapper serves `get_results` and `get_student_history`.
  - `pass_rate` becomes `Option`.
  - Order by `final_grade DESC NULLS LAST`.
  - `Session.avg_grade` becomes `Option`, NULL when nobody is graded.
- **`tui/ui.rs`**
  - Add State and Score columns in the list; withheld shows `—` plus its reason.
  - The detail pane gains a top state/reason line and a per-item `score/points` header.
- **`report_template.html`**
  - `_graded = grade_state === 'graded'`.
  - `grade_state` absent with `final_grade` set shows as legacy.
  - Add a state/reason badge and `score/max`.
  - Aggregates count graded students only, so withheld students are not counted as failures.
- **`scriptmark-py/src/lib.rs`**
  - `grade(..., assignment=None)` replaces `policy`, through the shared path.
  - New getters: `state`, `reason`, `score`, `max`.
- **`README.md:73-75`**
  - Guard the grade format against `None`.
- **`docs/test-bundles.md`**
  - Add a Scoring section: the schema, the withheld-reason table, and the zero-versus-withheld rules.
- **`examples/bundles/*/`**
  - Add example `assignment.toml` files.

## Commit sequence (the crate compiles after each)

1. **Additive model fields.** `Aggregation`, `GradingItem.points` and `.aggregation` (serde-defaulted), and all `StudentReport` fields (serde-defaulted). Legacy fixture tests still pass.
2. **Scorer.** Add `GradingConfig`, `ItemDecl`, `CompiledPolicy` and `grade_all`. Delete `GradingPolicy`, the flags and the blend. Fix all 6 call sites plus `test_load_course_config`. Flip `test_missing_student_gets_zero` and `test_formula_error_gives_zero`. Port the template tests.
3. **Assignment module.** Add `assignment.rs` with a `reconcile_items` that refuses, plus the validation table. Add the duplicate `meta.name` refusal. Wire `grade`, `run`, `summarize` and Python.
4. **Orchestrator and linter.** The `excused` flag, lint on the graded file, `run_lint -> Result`, and removal of `LintConfig.weight` (update the 5 linter tests).
5. **Export and push.** `export.rs` (`grades_to_push`, `write_grades_csv`), `cmd_grades_push`, the CSV archive changes, and `BTreeMap` in `push_grades`.
6. **DB.** Migration, row mapper, nullable average and the `grading_policy` column.
7. **Display, TUI, HTML, Python getters and README.**
8. **Docs and example `assignment.toml` files.** Add score assertions to `tests/examples.rs`.

## Regression tests mapped to acceptance items

**A1. More cases in one item do not change its max contribution.**
- `grading::more_cases_do_not_change_item_max`: 5 cases vs 50 cases, 2 points, next to a 1-point item. Max stays 3, and the score is equal at the same pass fraction.
- `integration::parametrized_count_does_not_change_item_max`: a real spec run with `count` 5 vs 50.
- `grading::all_or_nothing_aggregation`: 9/10 gives 0, 10/10 gives full points.

**A2. All correct and partial.**
- `grading::all_correct_and_partial`: all correct gives 100.0. With two 1-point items at 3/4 and 1/1, the score is 1.75/2, which is 87.5.
- `examples.rs`: alice gets full marks, bob gets a partial score.

**A3. Missing.**
- `grading::not_submitted_default_withheld` (the flipped `test_missing_student_gets_zero`; `StudentReport::default()` is legacy state, so it becomes NoEvidence, withheld).
- `grading::missing_zero_policy_gives_zero_without_curve`
- `grading::excused_always_withheld` (under `missing = "zero"`, and with an all-pass submission)
- `grading::received_unmatched_is_pending_review`
- `grading::missing_file_item_scores_zero_by_default_withheld_by_policy`
- `grading::all_nofile_items_reports_the_item`
- Port the call sites in `integration.rs:264-298` (dan is withheld NotSubmitted; the unmatched student is PendingReview).

**A4. Teacher formula error.**
- `grading::formula_that_fails_to_compile_is_refused` (the flipped `test_formula_error_gives_zero`): `compile` returns Err and no report is touched.
- `grading::formula_runtime_error_withholds_only_that_student`
- `grading::formula_non_finite_or_out_of_range_is_withheld_not_clamped` (`1.0/0.0`, `scale+1`, `-1`, `true`)
- `grading::teacher_fault_in_one_item_withholds_student_not_batch`, with `blocking_case` set.
- `integration::teacher_checker_crash_withholds_only_affected_student`

**A5. Environment startup failure.**
- `integration::env_startup_failure_withholds_all`: `/nonexistent/python3` gives every student Withheld(EnvironmentFault) with `final_grade` None.
- `grading::lint_tool_failure_withholds_when_lint_declared`
- `linter::spawn_failure_is_err_not_100`
- `grading::task_panic_is_grading_task_failed` (port of `db/mod.rs:317`)

**A6. Same evidence + policy gives the same result.**
- `grading::grade_all_is_idempotent_and_order_independent`: grading twice, and grading shuffled then sorted, give byte-identical JSON.
- `grading::rounding_is_half_away_and_recorded`: 2/3 gives 66.67, with the unrounded value in `basis`; 0 decimals gives 67.

**A7. Export distinguishes zero from ungraded.**
- `export::grades_csv_leaves_withheld_empty_and_writes_zero`
- `db::zero_and_withheld_round_trip_with_reason`
- `db::migration_adds_columns_to_pre_p677_db` (legacy rows read NULL state; `user_version` is 1)
- `db::session_average_null_when_nobody_graded`
- `input_equivalence::legacy_fixture_still_loads`: `final_grade == Some(95.0)`, `grade_state` None, no re-scoring on load. Replaces the `is_gradeable` assertion at `:350`.

**A8. Canvas sync distinguishes zero from ungraded.**
- `export::push_skips_withheld_and_excused_pushes_explicit_zero`
- `export::push_refuses_legacy_grade_without_state`
- `export::push_refuses_duplicate_canvas_user`

**Refuse-early validation.**
- `assignment::validate_assignment_refusals`, table-driven: duplicate id, orphan item, orphan spec, zero points, scale 0 or NaN, decimals 5, lower > upper, upper > scale, a bad formula, `lint_points` with no `[lint]`, `missing = "witheld"`, `pionts`, a missing `aggregation`.
- `spec_loader::duplicate_meta_name_is_refused`
- `spec_loader::empty_lint_command_is_refused`

**Unchanged.** `integration.rs:701-876` (the fault-owner tests), `harness.rs`, and `db/mod.rs:208-224,332-353`, whose intent is kept.

## Residual risks, accepted

- One teacher fault on a single item withholds the whole student. The per-item records show which item blocked.
- Re-scoring legacy JSON through `summarize` withholds every student with failures as UnknownFault. A regrade is required for those results.
- Removing the CLI flags and `LintConfig.weight` is a breaking change.
- The pushed scale is not checked against Canvas `points_possible`; that is P-680's.
