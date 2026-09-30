# P-675 — Expand samples and generation rules into a frozen, replayable test set

Linear: https://linear.app/acturea/issue/P-675
Parent: P-663 · Depends on: P-674, P-677 (merged) · Related: P-676 · Blocks: P-678

Revision 2. Revision 1 came out of a design workflow: three code maps (execution and
results, validation and tests, RNG and generator), three independent designs (minimal,
reproducibility-first, teacher-UX), and two judges (correctness, scope), both of which
picked the teacher-UX design. Revision 1 then went through a five-lens adversarial review
(acceptance, determinism, Rust feasibility, scope and UX, oracles), with a skeptic
refuting each lens. 32 of 38 findings survived. What changed, and why:

- **The spelling is the owner's (D-a, D-b).** `args` is an array of one-key tables in call
  order, samples are positional lists like fixed `args`, and the draws live in their own
  `[cases.parametrize.random]` table. Revision 1's `params` list beside a rules table was
  judged too much to remember.
- **An omitted seed is 0 (D-c).** `seed = "random"` asks for a drawn seed, which is
  recorded. Revision 1 drew a seed whenever none was declared.
- **A fresh run refuses to replace an artifact that holds other inputs (D-e).** Revision 1
  overwrote the only record of a drawn seed on any re-run, and wrote the artifact before
  `run_all`, so a Ctrl-C left a new artifact beside an old `results.json`. The artifact is
  now written after the run, next to the results, and replaced atomically.
- **Re-validation no longer refuses valid answers.** Revision 1 re-ran all of `validate` on
  the expanded spec. Its `contains_null(expect)` would refuse a reference answer such as
  `[1, None]`, which grades correctly today. Re-validation now skips that rule, and runs
  only once generation and the oracles have succeeded, so a failed oracle is not also
  reported as "declares nothing to judge".
- **The generator version travels with the inputs.** Revision 1 stamped the running
  binary's version into the artifact header, so replaying an old artifact under a newer
  build recorded the wrong version. Each template's entry now carries the version that
  drew it, and replay passes entries through untouched.
- **Replay checks the artifact against itself.** Row count, origin order, names, sample
  rows and `$` references are all checked, not only each row's length. The `format` is
  read before the strict parse, so a newer format is refused by name.
- **A seed is drawn only when something is random.** A template without `args`, or
  without `[random]`, records no seed.
- **Tests that could not fail were replaced.** There is now a golden test at the
  `generate` level (seed, streams, parameter order, names), a replay of a hand-edited
  artifact with values the rules cannot produce, and a sample in the fixture that fails
  alphabetical binding deterministically.
- **Corrected facts.** `blocking_case` names only a teacher or environment fault, so it is
  no reason to put samples first. The "zero-parameter templates are tested" citations were
  all refusal fixtures.

## Owner decisions

2026-09-30:

- **D-a** Generated parameters are an array in call order, one `{ name = rule }` table per
  parameter. The array order is the call order, and the key names the parameter for the
  Rhai oracle. It may be written inline or as one `[[cases.parametrize.args]]` block per
  parameter. The owner asked "why not split the parameters out"; this plan reads that as
  the block form, and uses it in the docs and the example.
- **D-b** The number of draws and the seed are kept apart from the parameters, in
  `[cases.parametrize.random]`. With no `[random]` table only the samples run.
- **D-c** An omitted seed is 0. `seed = N` fixes it. `seed = "random"` draws one, which is
  recorded and printed.
- **D-d** A seed belongs to one template. There is no run-level seed and no `--seed` flag.
- **D-e** A fresh `grade`/`run` that finds an artifact holding other inputs refuses before
  any student runs, unless `--replay FILE` or `--fresh` is given.

Adopted as recommended, not asked. Each can still be overturned:

- **D-f** The artifact freezes inputs only. Answers stay with the oracles and with P-676,
  which adds an `answer` slot and bumps `format`.
- **D-g** Replay requires the input settings to match. Oracles, checks and timeouts may
  change, and are re-run on the frozen inputs, so a teacher can fix a checker and regrade
  the same inputs.
- **D-h** Samples and `choice` elements are spelled like fixed `args`: `$$x` is the literal
  text `$x`, and a `$name` reference or `null` is refused. The artifact freezes values,
  not names, so a reference to a `[vars]` entry could not be replayed faithfully.
- **D-i** Caps: `count` ≤ 10 000; `list` nesting ≤ 8; each template's worst-case generated
  size ≤ 1 000 000 values or characters.
- **D-j** Under proportional scoring a boundary sample weighs `1 / cases`. This is
  documented, with the advice to use `all_or_nothing` or a separate item when samples must
  weigh more.
- **D-k** `setup.file` stays refused. Its message stops pointing at P-675, and generated
  setup data becomes a follow-up ticket.
- **D-l** There is no preview command. `grade` on one test submission shows the inputs.
- **D-m** Python `run()`/`grade()` refuse, before any student runs, a `seed = "random"`
  template when no `freeze=` path is given, so a drawn seed is never lost.
- **D-n** Values come from `rand_chacha::ChaCha8Rng` through our own mapping on
  `next_u64`, pinned by golden tests.

Settled before this plan:

- provenance goes on `Bundle`, not a parallel type (P-674 D12);
- a generation error is a preparation failure, not `null` (P-674 "left for their owners");
- an item's max is its declared points, never its number of cases (P-677 D1).

## Scope

In scope:

- the spelling (D1);
- a generator that parses rules into an AST and cannot fail once parsed;
- a portable RNG, and default, declared or drawn seeds;
- one naming rule for concrete cases;
- binding in call order for the student, the reference and the Rhai oracle;
- refusing every generation error before grading;
- the frozen-inputs artifact, `--replay` and `--fresh`;
- refusing duplicate spec names in `prepare`;
- the CLI and Python surfaces;
- docs, an example fixture, and regression tests.

Not in scope:

| Ticket | Owns |
|---|---|
| P-676 | answer sources, their provenance, priority and freezing; an `answer` slot on each frozen case; resolving references concurrently; property checks that see a case's inputs |
| P-678 | the versioned result contract; bundle and artifact digests; linking results or a DB session to an artifact; regrading from stored evidence |
| P-673 | student-file matching, and the harness's fuzzy lookup (which uses the number of arguments) |
| follow-up | generated setup data (`setup.file`, D-k); checking a reference's signature against `args`; showing a generated case's arguments in terminal failures |

## Facts checked against the code

**How arguments bind today**

- `expand_case` seeds `StdRng::seed_from_u64(param.seed.unwrap_or(0))`
  (`runner/expander.rs:13`). It builds `args` from `param.args.values()`, a `BTreeMap`,
  so arguments bind alphabetically (`:14-21`). A generator error becomes `Null` (`:21`).
  Names are `"{name} [{i}]"` (`:17`).
- `Parametrize` (`models/spec.rs:186-200`) is `deny_unknown_fields`, with a required
  `count: usize`, `seed: Option<u64>` and `args: BTreeMap<String, String>`.
- `prepare_one` takes `arg_names = param.args.keys()` (`runner/prepare.rs:158`) and
  resolves each generated case's oracle in turn (`:159-171`), pushing the case even when
  its oracle failed (`:172`). Then `spec.cases = cases` (`:178`), `check_names` (`:180`),
  and the units are planned once (`:186`).
- The Rhai oracle binds `arg_names` zipped with `case.args` (`runner/oracle.rs:78`). The
  reference is called positionally with the same vector (`:44`). The Rhai oracle's
  compile check also takes `param.args.keys()` (`spec_loader.rs:707-711`).
- The harness turns a `$name` string into the named value, and `$$` into a literal `$`,
  recursively (`runner/harness.py:416-427`). A sample `"$$x"` therefore reaches the
  student as `$x` but reaches the Rhai scope as `$$x` unless it is unescaped first.
- `toml` 0.8.23 deserialises `[[cases.parametrize.args]]` blocks and an inline array of
  one-key tables alike, in document order, and a second `[[cases]]` does not absorb the
  first's blocks (checked with a scratch program). The old table spelling fails with
  serde's "invalid type: map, expected a sequence", which names no fix.

**How the generator fails today**

- `random_range` gets unchecked bounds (`runner/generator.rs:19`, `:24`, `:32`); rand
  panics on `min > max` and on a non-finite float bound.
- `bool(x)` ignores its argument (`:27`); `choice` accepts any JSON array, `null` included
  (`:46-52`); `list` parses its inner rule only when it draws at least one item (`:57-60`);
  `parse_two_nums` splits on every `,` (`:89-94`); `parse_list_args` counts brackets inside
  JSON strings (`:111-118`).

**Validation**

- `validate` registers the expanded names `"{name} [{i}]"` for a parametrized case
  (`spec_loader.rs:243-251`), duplicating the format in `expander.rs:17`.
- `count ≥ 1` is enforced (`:641`), and so is "no case `args` beside parametrize" (`:644`).
- `contains_null(args)` (`:389`) and `contains_null(expect)` (`:422-429`) run on every
  case. The second exists because TOML has no null, so a null in a parsed `expect` is a
  lost `inf`/`nan`. After an oracle, a null is Python's `None`: `oracle.rs:55-60` refuses
  only a top-level `None`, and a nested one grades correctly.
- The approx, set_eq and text shape checks run once `expect` is set (`:605-622`).
- `setup.file` is refused with a message pointing at P-675 (`:328-333`).
- No test loads a zero-parameter template successfully; the fixtures at `:1097`, `:1134`
  and `:1227-1243` are all refusals for other reasons.

**Duplicate spec names**

- `assignment::settle` refuses a duplicate `[meta] name` (`assignment.rs:79`), but the
  Python `run()` never calls `settle` (`scriptmark-py/src/lib.rs:263-264`).
- Results are keyed by that name: `TestResult.item_id = meta.name` (`orchestrator.rs:198`).

**Order and concurrency**

- Specs are prepared concurrently and re-sorted by index (`prepare.rs:92-115`). The
  orchestrator only clones a planned unit and sets its file (`orchestrator.rs:157-158`), and
  places results by slot (`:122-201`). Every student already gets the same generated
  inputs; what is missing is the record of them.

**Evidence**

- `CaseInput.args` records each case's arguments (`models/result.rs:77-88`), copied from
  `case.args` (`runner/judge.rs:418-428`), so results.json already shows every input.
- `StudentReport` is `deny_unknown_fields` (`result.rs:179`); this plan adds nothing to it.

**The CLI**

- `run_bundles` prepares (`main.rs:498`) and runs (`:512-513`), and gets no output path.
- `grade` and `run` call it at `:576` and `:747`. Ctrl-C bails at `:516`, and
  `grade_all(...)?` can fail at `:586`, both before results are written (`:599-604`,
  `:756-761`). The output directory is created only then (`:599-601`, `:756-758`).

**Scoring**

- An item's max is `item.points`; the number of cases enters only the proportional
  fraction (`grading.rs:354`, `:401-405`).
- `blocking_case` names the first case with a teacher or environment fault (`:375-385`); a
  graded item sets none.
- `grading::test_more_cases_do_not_change_an_items_max` (`grading.rs:528`) and
  `integration::test_more_generated_cases_do_not_change_an_items_worth`
  (`integration.rs:412-477`) exist. The latter uses derived 1-point items and a symmetric
  oracle, so it detects neither declared points nor binding order.

**Crates**

- `rand` 0.9.5 and 0.10.3, `rand_chacha` 0.9.0, `rand_core` 0.9.5, `serde_json` 1.0.151 and
  `toml` 0.8.23 are all in `Cargo.lock`.
- rand_chacha documents its generators as "deterministic and portable"
  (`rand_chacha-0.9.0/src/lib.rs:20`) and has `set_stream` (`chacha.rs:232`). rand_core
  calls a change to `seed_from_u64` "value-breaking" (`rand_core-0.9.5/src/lib.rs:464-465`).
- `OsRng` needs rand's default `os_rng` feature (`rand-0.9.5/Cargo.toml:66-72`), and
  `TryRngCore::try_next_u64` is fallible (`rand_core-0.9.5/src/os.rs:92`).
- serde_json's `float_roundtrip` is off by default (`serde_json-1.0.151/Cargo.toml:69`).
  The harness records are parsed with serde_json too (`runner/records.rs:65`).

## Decisions

**D1. Spelling (D-a, D-b, D-c).**

```toml
[[cases]]
name = "clamp"

[[cases.parametrize.args]]      # call order: clamp(value, low, high)
value = "choice([-100, -75, -25, 0, 25, 75, 100])"
[[cases.parametrize.args]]
low = "int(-49, -26)"
[[cases.parametrize.args]]
high = "int(26, 49)"

[cases.parametrize]
samples = [[-30, -30, 30], [30, -30, 30], [-100, -30, 30]]   # like fixed args

[cases.parametrize.random]      # optional: random draws from the rules above
count = 12
seed = 7                        # omit -> 0; "random" -> drawn and recorded

[cases.parametrize.oracle]
rhai = "if value < low { low } else if value > high { high } else { value }"
```

- `args` declares every parameter: its name, its place in the call and its rule. It is
  required whenever the function takes arguments, whether the inputs are samples, draws
  or both. One rule, so there is nothing to infer. The inline form
  `args = [{ value = "..." }, { low = "..." }]` is the same thing.
- `samples` are inputs only: each is a list of values in call order, exactly like a fixed
  case's `args`. Nothing in a sample is an answer.
- `[random]` holds `count` (at least 1) and `seed`. Without it, only the samples run.
- A template with no `args` is a function without arguments. `[random] count = 5` then
  calls it five times: a property check on a nondeterministic function. Such a template
  has nothing random to seed, so a declared seed is refused.

**D2. Model.**

```rust
// models/spec.rs — every struct deny_unknown_fields
pub struct Parametrize {
	#[serde(default, deserialize_with = "...")] pub args: Vec<Param>, // call order
	#[serde(default)] pub samples: Vec<Vec<Value>>,
	#[serde(default)] pub random: Option<Random>,
	#[serde(default)] pub oracle: Oracle,
}
pub struct Param { pub name: String, pub rule: String } // (de)serialised as { name = rule }
pub struct Random { pub count: usize, #[serde(default)] pub seed: Seed }
pub enum Seed { Fixed(u64), Random }                     // default Fixed(0)
```

- `Param`, `Seed` and the `args` field get hand-written `Deserialize` impls, as
  `CheckMethod` already has (`spec.rs:48-71`), so errors name the fix rather than
  "did not match any variant". The old table spelling of `args` gets: "args is now a list
  in call order, one `{ name = rule }` per parameter — write a `[[cases.parametrize.args]]`
  block for each of a, b, in the order the function takes them". No ready-to-paste array
  is offered, because its order would be a guess.
- `Parametrize::inputs()` returns an `Inputs { args, samples, random }` value, everything
  that decides the inputs and nothing else. It is a separate struct because serde cannot
  combine `flatten` with `deny_unknown_fields`.
- `runner/generation.rs` replaces `expander.rs`:

  ```rust
  pub enum Origin { Sample(usize), Draw(usize) }      // {"sample": j} | {"draw": i}
  pub enum SeedSource { Default, Declared, Drawn }
  pub struct Concrete { pub name: String, pub origin: Origin, pub args: Vec<Value> }
  pub struct Generated {
  	pub generator: u32,                // GENERATOR_VERSION that drew these inputs
  	pub inputs: Inputs,                // the settings, as the spec wrote them
  	pub seed: Option<u64>,             // the seed used; None when nothing was drawn
  	pub seed_source: Option<SeedSource>,
  	pub cases: Vec<Concrete>,          // samples first, then draws
  }
  pub fn generate(case: &str, inputs: &Inputs, seed: Option<u64>) -> Result<Generated, Vec<String>>;
  ```

- `Bundle` gains `pub generated: BTreeMap<String, Generated>`, keyed by source case name.
- `TestCase` is unchanged. Concrete cases join results by name.

**D3. Rules and values (D-i, D-n).**

- `generator.rs` parses a rule into `Rule` (`Int`, `Float`, `Bool`, `Str`, `Choice`,
  `List`) once. `Rule::draw(&self, &mut ChaCha8Rng) -> Value` cannot fail.
  `pub const GENERATOR_VERSION: u32 = 1`.
- The mapping is our own, on `next_u64`, with no rand distribution involved:
  - `int(a, b)`: rejection sampling over the span as `u128`, so the full `i64` range works;
  - `float(a, b)`: `a + (b − a)·u`, `u = (x >> 11)·2⁻⁵³`, clamped to `[a, b]`;
  - `bool()`: the top bit;
  - `choice`, and the lengths of `str` and `list`, through the int mapping;
  - `str`: 36 lowercase letters and digits, as today.
- Streams: the template's RNG is `ChaCha8Rng::seed_from_u64(seed)` with `set_stream(i)`
  for draw `i`, and the parameters are drawn in call order. Growing `count` keeps earlier
  draws; editing one rule changes only the later parameters of each draw; templates share
  no RNG, so preparing specs concurrently changes nothing.
- The promise: the same seed and the same `GENERATOR_VERSION` give the same inputs on
  every platform. Golden tests pin it at both layers (Tests). Changing a golden value
  means bumping the version.
- `rand_chacha = "0.9"` becomes a direct dependency; it is already locked, so nothing new
  is built. `rand` stays for `OsRng`. serde_json's `float_roundtrip` is turned on in the
  same commit, so a `choice` float and every student float parse exactly. This can flip a
  rare comparison that was one ULP off, which is a fix.
- Every generated value changes once, even under a declared seed.

**D4. Seeds (D-c, D-d).**

- A seed is used only when `[random]` exists and `args` is not empty. Otherwise `seed` and
  `seed_source` are `None`.
- Omitted: 0, recorded as `Default`. `seed = N`: `N`, recorded as `Declared`, for
  `0 ≤ N ≤ i64::MAX` (TOML's integers). `seed = "random"`: drawn and recorded as `Drawn`.
- The draw is `OsRng.try_next_u64()` masked to `2⁵³ − 1`, so it fits TOML and stays exact
  for JSON readers. A failed draw is a `PrepareError`.
- For each drawn seed, stderr gets: "case 'clamp' in 'clamp': drew seed 81234. Write
  `seed = 81234` to keep these inputs, or grade with --replay output/results.cases.json".
- Two templates with the same seed and rules draw the same inputs; the docs say so.

**D5. Names and uniqueness.**

- `generation::concrete_name(case, origin)` gives `"{name} [sample {j}]"` and
  `"{name} [{i}]"`. It replaces both `expander.rs:17` and `spec_loader.rs:246`.
- Samples come first, then draws.
- `validate` registers every source `[[cases]]` name and every concrete name, so a fixed
  case named `x [sample 0]` beside template `x` is refused, as are two templates named `x`.
- `prepare` refuses two specs with one `[meta] name`: results and the artifact are keyed
  by it. The CLI already refuses this in `settle`; the Python `run()` did not.

**D6. The artifact (D-f).** JSON, never `.toml`, because the loader reads every `*.toml`
in the tests directory.

```json
{ "format": 1, "scriptmark": "0.3.0",
  "specs": { "clamp": { "clamp": {
    "generator": 1,
    "inputs": { "args": [{"value": "choice([...])"}, {"low": "int(-49, -26)"}, {"high": "int(26, 49)"}],
                "samples": [[-30, -30, 30], [30, -30, 30], [-100, -30, 30]],
                "random": { "count": 12, "seed": 7 } },
    "seed": 7, "seed_source": "declared",
    "cases": [ { "name": "clamp [sample 0]", "origin": {"sample": 0}, "args": [-30, -30, 30] },
               { "name": "clamp [0]", "origin": {"draw": 0}, "args": [75, -31, 40] } ] } } } }
```

- `Frozen { format, scriptmark, specs: BTreeMap<spec, BTreeMap<case, Generated>> }` comes
  from `Frozen::of(&[Bundle])`. `scriptmark` names the build that wrote the file. Every
  entry is a `BTreeMap` or a `Vec` in declaration order, so the file does not depend on
  student order or concurrency.
- Loading reads `format` from a lenient header first and refuses one it does not know,
  then parses the strict `Frozen`. A newer file is refused by name, not with "unknown
  field `answer`".
- It holds inputs only and no paths. Digests and links from results to an artifact are
  P-678's.
- `frozen::beside(output)` names it: `output/results.json` → `output/results.cases.json`.
  `grade --archive` also writes `cases_{stem}.json` beside `archive_{stem}` and
  `grades_{stem}.csv`.
- It is written with a temporary file and a rename, so it is never half-written.

**D7. Replay (D-g).**

- `prepare(specs, &Generation, executor, timeout)`, with
  `enum Generation { Fresh(DrawSeed), Replay(Frozen) }` in `runner/frozen.rs`. `DrawSeed`
  is a `fn() -> Result<u64, String>`; the CLI and the bindings use
  `Generation::fresh()`, which draws from the OS, and tests pass a failing one.
- Under `Replay`, each template's entry is taken from the artifact as it is — generator
  version and seed included — and its cases are never regenerated.
- Everything else runs as it does for fresh inputs: validation, inspecting the teacher
  modules, the oracles, `check_names`, the Rhai dry run and re-validation.
- Replay refuses, each as a `PrepareError` naming the case:
  - a template the specs have and the artifact lacks, or the reverse;
  - `args` (names, order or rules), `samples` or `count` that differ from the spec,
    naming the field: "case 'clamp': the spec says count = 30; the frozen inputs used
    count = 12. Restore it, or grade with --fresh to draw new inputs";
  - a declared seed other than the recorded one. A spec that now declares the seed a
    `"random"` run drew matches; a spec that now says `"random"` matches any recorded seed;
  - an entry that disagrees with itself: rows not exactly samples `0..n` then draws
    `0..count`, in order; a name that is not `concrete_name`; a sample row unequal to its
    sample; a row whose length is not the number of parameters; a `null` or a `$name`
    anywhere in a row.
- Oracles, checks and timeouts may change; the oracles are re-run on the frozen inputs.

**D8. Errors.**

Static, in `Validator::parametrize`, at load and again in `prepare`, all collected:

1. **args.** An entry with no key or several keys; a name that is not an identifier
   (`[A-Za-z_][A-Za-z0-9_]*`, so the Rhai oracle can name it); a name used twice; the old
   table spelling (D2).
2. **random.** `count = 0` ("drop [random] to run only the samples"); `count` above the
   cap; a declared seed on a template with no `args`; a seed below 0 or not an integer
   or `"random"`.
3. **Nothing to run.** No samples and no `[random]`.
4. **samples.** A sample whose length is not the number of parameters; a `null`
   (`contains_null`); a `$name` reference (`refs()`).
5. **rules.** Every rule must parse, whether or not it is drawn:
   - an unknown function; the wrong number of arguments; a number that does not parse;
   - `min > max`; a non-finite float bound, or `max − min` not finite; a negative length;
   - `choice`: invalid JSON (the message gives an example), not an array, empty, or
     containing `null`, a `$name` reference, or an integer outside `i64`;
   - `bool` with an argument;
   - `list`: an invalid inner rule even when its minimum length is 0; nesting deeper than
     the cap. The bracket scanner learns to skip JSON strings;
   - the template's worst-case size (count × the largest draw) above the cap.
6. **Answers.** A sample can hold no answer, since it is only a list of values. The
   message for a stray `expect` in `[cases.parametrize]` is serde's unknown-field error,
   which lists the allowed keys.

In `prepare`, as `PrepareError`s: a failed seed draw; a replay mismatch (D7); two specs
sharing a name (D5); the existing oracle failures; then, **only if all of those are
empty**, re-validation of the expanded spec (below).

**Re-validation.** After `spec.cases = cases`, `validate` runs again with one rule
switched off: `contains_null(expect)`, because a null from an oracle is a real `None`.
The expanded cases have `parametrize: None`, their `args` set and their `expect` resolved,
so the shape checks (`spec_loader.rs:605-622`) now catch an oracle answer that cannot fit
its checker, such as a list under `approx`, before any student runs.

`generate` returns every problem it finds. The `unwrap_or(Null)` (`expander.rs:21`) goes,
and no generation path is left that can panic or produce `null`.

**D9. Oracles (D-h).**

- The parameter names come from `args` in call order at all three sites, changed together:
  `arg_names` (`prepare.rs:158`), the Rhai scope (`oracle.rs:78`) and the Rhai compile check
  (`spec_loader.rs:708`).
- One value everywhere. A new helper `literal(&Value) -> Value` strips one `$` from each
  `$$` string, recursively, mirroring `harness.py:418-419`. It is applied only where the
  Rhai oracle binds. The student and the reference get the same value through the
  harness. `str()` draws only letters and digits, so no other generated string starts
  with `$`.
- Answers come from the rules at `spec_loader.rs:647-720`, which do not change: a fixed
  `expect` or `expect_error` beside the template, a case-level `check`, the reference, the
  Rhai oracle, or `oracle.check`. `oracle.check` is `sorted` alone, because it must need
  no expectation, and no checker sees the case's inputs; input-aware properties are
  P-676's.
- A reference oracle is called with the same positional vector as the student
  (`oracle.rs:44`), so it cannot catch a binding mistake. A Rhai oracle can, which is why
  the fixture and the binding test use one.
- Samples, draws, a seed and a reference can each be absent. Only "nothing to run" (D8)
  and "nothing to judge" (`spec_loader.rs:456-471`) are refused.

**D10. CLI (D-e).**

- `grade` and `run` gain `--replay <FILE>` and `--fresh`, which conflict.
- `run_bundles` takes the `Generation` and returns the reports and the `Frozen`.
- Right after `prepare`, before any student runs, when the bundles have templates and
  `--replay` was not given: if `beside(output)` exists, it is loaded. Unless `--fresh` is
  given, a file that fails to load, or whose inputs differ from the new ones, is refused:
  "output/results.cases.json holds other inputs (case 'clamp' in 'clamp' differs): grade
  with --replay output/results.cases.json to use them again, or --fresh to replace them".
  Identical inputs, which is the usual case with a fixed or default seed, go ahead.
- After the run, `results.json` is written first, then the artifact beside it, atomically.
  An interrupted or failed run replaces neither. A drawn seed is still printed at prepare
  time (D4).
- results.json, `StudentReport` and the database are unchanged.

**D11. Python bindings (D-m).**

- `run()` and `grade()` gain `freeze: str | None` (where to write the artifact) and
  `replay: str | None` (an artifact to reuse).
- A `seed = "random"` template with `freeze=None` and no `replay` → `ValueError` before any
  student runs: "case 'x' in 'y' draws a random seed: pass freeze='cases.json' to keep it,
  or declare seed = N".
- An invalid artifact → `ValueError`; a missing one → `FileNotFoundError`, following
  P-674's convention for the bindings.
- The bindings write where they are told: an explicit `freeze=` path is the caller's
  choice, so D10's refusal does not apply.
- `PyTestSpec.num_cases` keeps counting source cases.

## Migration

| Spec shape | After P-675 |
|---|---|
| `[cases.parametrize.args]` table | refused; the message asks for `[[cases.parametrize.args]]` blocks in call order |
| `count`, `seed` directly in `[cases.parametrize]` | refused as unknown fields; they move to `[cases.parametrize.random]` |
| explicit `seed` | same meaning; the values change once (D3) |
| no `seed` | still 0, now recorded; the values change once |
| a bad rule | refused at load; was `null` args |
| `min > max`, a NaN or infinite bound | refused at load; was a panic |
| `bool(x)`; `choice` with `null`, `$name` or an out-of-range integer | refused |
| two specs sharing `[meta] name` in Python `run()` | refused in `prepare` |
| a fixed case named like a concrete case | refused |
| Python `run()`/`grade()` with `seed = "random"` and no `freeze=` | `ValueError` |
| `setup.file` | still refused; the message changes (D-k) |

Every spec in the repository with a parametrized case moves to the new spelling:
`README.md:122-128`, `examples/python/test_larger_number.toml`, and the fixtures in
`spec_loader.rs` and `tests/integration.rs`. The docs' "Migrating an older spec" table
gets the same rows.

## Tests

**A fixed seed reproduces the inputs; a drawn seed is recorded and replays.**

- `generator::golden_values_v1`: seed 42, literal values for every rule kind, asserting
  `GENERATOR_VERSION == 1`.
- `generation::golden_generate_v1`: a template with three parameters, two samples and
  `count = 3` (so streams 0–2 are covered) under a fixed seed. Asserts the literal names,
  origins and args, so a change to streams, parameter order or naming fails it.
- `generator::int_full_range_and_degenerate_spans`, `float_stays_within_bounds`,
  `parse_refusals` (table-driven over D8.5, including `list(choice(["a)","b"]), 1, 3)`).
- `generation::same_seed_same_cases`, `drawn_seed_below_2_53`.
- `integration::pasting_a_drawn_seed_reproduces_the_inputs`: `seed = "random"`, then the
  recorded seed written back as `seed = N`; the cases are identical.
- `frozen::floats_round_trip_bit_exact`: 10 000 drawn floats and known hard doubles,
  through the artifact, compared with `to_bits`.

**Changing `count` does not change the item's points.**

- `examples::test_generated_count_does_not_change_item_points`: through
  `assignment::settle` with `points = 4`, `count = 12` and `count = 52`. `max` is 4 both
  times; alice scores 4 both times.
- `generation::growing_count_keeps_earlier_draws`, `adding_samples_keeps_draws`.
- The existing grading and integration tests stay; the latter moves to the new spelling.

**Samples, rules, reference and seed combine, and each may be absent.**

- `generation::samples_only_records_no_seed`, `random_only`, `samples_then_draws_in_order`,
  `no_args_template_records_no_seed`.
- `integration::template_matrix`, where each cell passes a correct student and fails a
  wrong one: samples only with a reference; draws only with Rhai and the default seed;
  samples and draws with `oracle.check`; a fixed case beside a template in one spec.

**The frozen artifact is reused directly, whatever the order or concurrency.**

- `integration::replay_uses_the_frozen_rows`: hand-edit an artifact so its draw rows hold
  values the rules cannot produce and its `generator` is 999. Replay it: those exact values
  reach every student's `CaseInput.args`, and the rewritten artifact keeps them and 999.
- `integration::replay_accepts_a_changed_oracle`: a corrected Rhai oracle regrades the
  same inputs.
- `integration::replay_refusals`: an unknown `format` with an extra field; a missing and an
  extra template; a changed `count`, rule, parameter order and sample; a conflicting
  seed; a truncated `cases`; a renamed row; an edited sample row; a `$name` in a row.
- `integration::inputs_do_not_depend_on_order_or_concurrency`: prepare specs `[A, B]` and
  `[B, A]`, and a spec with its templates reordered, with declared seeds; each template's
  cases are identical. Run the students at concurrency 1 and 8, in reverse order; the
  artifact and each (student, case)'s `CaseInput.args` and status are identical.
- `frozen::beside_names_the_file`, `frozen::write_is_atomic_and_creates_the_directory`,
  `frozen::a_newer_format_is_refused_by_name`.
- A small library function decides D10's refusal from (existing file, new inputs,
  `--fresh`); it is unit-tested for identical, different, unreadable and `--fresh`.

**Binding order.**

- `generation::binds_in_call_order`: three parameters with disjoint ranges.
- `spec_loader::the_old_args_table_is_refused_with_the_fix`.
- `integration::rhai_oracle_and_student_agree_on_order`: `clamp` with parameters in
  reverse-alphabetical order and the sample `[-100, -30, 30]`. The correct student passes
  every case; a student reading the arguments alphabetically fails that sample, whatever
  the draws.
- `oracle::a_dollar_literal_reaches_rhai_and_student_alike`.

**Generation failures are preparation errors.**

- `spec_loader::parametrize_refusals`: table-driven over D8.1–D8.4, the caps and D5.
- `prepare::a_hand_built_bad_rule_is_a_prepare_error`: a `PrepareError`, not `null`.
- `prepare::a_failed_seed_draw_is_refused`: a failing `SeedSource`.
- `prepare::duplicate_spec_names_are_refused`.
- `prepare::an_oracle_answer_of_the_wrong_shape_is_refused`: a reference returning a list
  under `approx`.
- `prepare::a_reference_answer_with_nested_none_prepares`.
- `prepare::a_failed_oracle_is_not_also_nothing_to_judge`.

**Fixture.** `examples::test_the_generated_cases_example` pins `failures()` and
`graded()` (below).

## Docs and example fixture

- **`docs/test-bundles.md`**: a "Generated cases" section after "Checkers": `args` in call
  order, samples as inputs only, the rule table with bounds and caps, `[random]` and
  seeds, the names of concrete cases, `$$` in samples and `choice`, the artifact,
  `--replay` and `--fresh`, what replay accepts, and the weight of samples (D-j). New rows
  in "Refused before grading" and "Migrating an older spec". README's parametrize example
  and Python section follow.
- **`examples/bundles/generated_cases/`**:
  - `tests/test_clamp.toml`: the D1 example, plus a fixed case `args = [5, 0, 10]`,
    `expect = 5`. The draws take `value` from a `choice` that never equals a drawn bound,
    and two samples sit exactly on the bounds.
  - `assignment.toml`: item `clamp`, `points = 4`, `proportional`.
  - `submissions/alice_clamp.py`: correct. Reading the arguments alphabetically would
    fail her on `[-100, -30, 30]`.
  - `submissions/carol_clamp.py`: returns `high` when `value == low` and `low` when
    `value == high`. She fails exactly `clamp [sample 0]` and `clamp [sample 1]`, whatever
    is drawn: 14 of 16 cases, 3.5 points, grade 87.5.

## Commits (the crate builds and the tests pass after each)

1. **Plan.** This document.
2. **Generator.** `Rule`, `parse`/`draw`, ChaCha8 with our own mapping, `GENERATOR_VERSION`,
   the value goldens, `rand_chacha`, `float_roundtrip`.
3. **Spelling, validation and generation.** `Param`, `Random`, `Seed`, their
   deserialisers, the D8 static rules, and `generation.rs` in place of `expander.rs`:
   `Bundle.generated`, seeds, call-order binding, `literal()`, re-validation, duplicate
   spec names, and the repository's specs moved to the new spelling. Planned as two
   commits; one, because the model change and the expander that read it cannot build
   apart.
4. **Freeze and replay.** `frozen.rs`, `Generation`, the `prepare` call sites, `--replay`,
   `--fresh`, the artifact, and the Python `freeze`/`replay`.
5. **Docs, fixture, and the examples tests.**

## Residual risks, accepted

- Every generated value changes once on upgrade, even under a declared seed.
- With `seed = "random"`, one case name holds different inputs in different sessions. The
  artifact and `CaseInput.args` show which inputs each session used; the database history
  still compares them by name (P-678).
- Two templates with the same seed and rules draw the same inputs.
- The artifact holds inputs only, so a replay after the teacher edits the reference
  changes the expectations (P-676).
- A boundary sample weighs less as `count` grows (D-j).
- A crash between writing `results.json` and the artifact leaves the old artifact beside
  the new results. The next fresh run then sees other inputs and refuses, so the
  mismatch is not silent.

## Left for their owners

- **P-676:** answers in the artifact and their provenance and priority; resolving
  references concurrently (`prepare.rs:159-171` runs them one at a time); one failure per
  template instead of `count` near-identical ones; property checks that see the inputs.
- **P-678:** artifact and bundle digests; linking a result or database session to its
  artifact; seeds in the sessions JSON (`main.rs:718-721`); regrading from evidence.
- **P-673:** the harness's fuzzy lookup by number of arguments.
- **Follow-up ticket:** generated setup data (`setup.file`); checking a reference's
  signature against `args`; showing a generated case's arguments in terminal failures.

## Review of the implementation

A six-lens adversarial review of the branch (acceptance, determinism, errors, wiring,
quality, tests and docs), each lens checked by a skeptic, left 32 of 38 findings, which
came to these changes (`c5c08f9`):

- **D10 compares rows.** "Other inputs" meant the whole recorded entry, so writing a drawn
  seed back as `seed = N`, as the note says, was refused as other inputs. Found by five of
  the six lenses. Now only the concrete rows count; how the seed was spelled or chosen,
  and the generator version, do not.
- **A batch without templates removes a stale file.** It left an earlier batch's inputs
  beside results they had nothing to do with. It now passes the same check, and the file
  goes.
- **The drawn-seed note comes after the check.** Printed before it, a refused run offered
  `--replay` of a file that held the previous seed's inputs.
- **The count cap holds in validation.** `validate` named every draw before the cap
  refused them, so a mistyped `count = 1000000000` never finished loading.
- **`choice` sizes and integers.** A `choice` counts as its largest value under the size
  cap, and an integer literal past 64 bits is refused instead of drawn as a float.
- **The file's mode follows the umask**, like `results.json`, rather than tempfile's
  owner-only default.
- **Python probes `freeze=`** before any student runs, and `grade()` writes it only after
  scoring succeeded. With no templates it writes a file holding none: the caller named
  the path, and the file says what was graded on.
- Messages: `count = 0` names a fix that works; an oracle's answer that fails its check
  is not called `expect`. Format 1 is pinned by a literal file, and the golden generate
  test asserts origins.
- One null walk, one `plural`, and `Inputs::draws`/`Inputs::seeded` replace copies.

Kept as it is: the refused `count` and `seed` fields on `Parametrize` also appear in
serde's list of expected keys. They follow `copy_refs`, `compile` and `setup.file`, and
give the message that names the fix.
