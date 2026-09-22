# P-670 — Canvas assignment / submission import and default normalisation

Linear: https://linear.app/acturea/issue/P-670
Parent: P-663 · Milestone: Canvas 与本地提交可统一导入

Revision 2 — Revision 1 was written against commit `541b259`, then put through a six-lens
adversarial review (60 findings, 28 survived refutation). Revision 1 asserted that
`normalize` was correct and would not be touched. That was wrong in four places, and the
sections below that carried the most weight — D6, D8, D9, D10 — were the ones that broke.

## Scope

P-669 built the pure half: `input::canvas::normalize(payload, roster, downloads, policy)`
turns Canvas payloads into an `AssignmentInput` and does no I/O. P-670 is the other half —
everything that gets those payloads and those downloads onto the machine:

fetch (paginated) → save the bundle → download attachments → expand zips → `normalize`.

**Not** in scope: teacher matching rules (P-673), per-item scoring and zero-vs-ungraded
policy (P-677), the grading result format (P-678), grade push (P-680, which owns
`push_grades` and its `eprintln`-and-continue loop), XLSX / column mapping (P-672).

## The boundary with P-669

Revision 1's "normalize is correct and stays untouched" is withdrawn. What is actually
true:

**Unchanged.** The payload structs' shape, `merged_roster`, `identity_for`, the two-axis
`SubmissionState` / `RosterMatch` model, and the owner decisions (Canvas decides
membership; 评分项 is a type).

**Changed, and why.** Four functions, each for a defect the review found:

| Function | Change | Because |
| -- | -- | -- |
| `attempts_of` | reads `Result`-valued downloads; expands zips; keyed diagnostics | a failed download is not the same fact as an un-attempted one, and neither names the student |
| `normalize` | retains placeholder rows; takes an `Assignment` | 免交 has nowhere to live; `assignment.toml` is silently dropped on the Canvas path |
| `not_submitted` | takes `source_status` | same |
| duplicate-row rule | keeps the row that has an attempt | keeping the first makes a real submitter 缺交 |

## What is already true, and what is broken

Verified against the Canvas docs and the `canvas-lms` serializer source:

- `attempts_of` prefers `submission_history` and falls back to the top-level row. Canvas's
  history *does* carry a per-entry `attempt` and its own `attachments`, and the top-level
  submission duplicates the last history entry — so both the preference and the fallback
  are right. **But history is opt-in** (see D1): without the include, the fallback is the
  only path that ever runs.
- Canvas **repeats carried-forward attachments in later attempts**: the same
  `attachment_id` appears under attempt 1 and attempt 2. `DownloadedAttachments` is keyed
  by id, so this de-duplicates for free — provided the downloader collects a *set* of ids
  rather than walking attempts.

Three defects the fetch layer must not inherit:

1. **`content-type` is hyphenated in Canvas JSON.** The serializer emits
   `"content-type" => attachment.content_type`. `CanvasAttachmentPayload.content_type` has
   no `rename`, so against a real payload it deserialises to `None` every time. P-669's
   fixtures use the underscore spelling and therefore pass. **D3.**
2. **Pagination uses `page=N` until an empty array** (`client.rs:78-120`). Canvas documents
   opaque bookmark cursors in a `Link` header and says the links "should be treated as
   opaque". **D1.**
3. **`pull_roster` loses the Canvas id** and writes unescaped CSV. **D9.**

## Decisions

### D1 — Every request's query string is pinned, and pagination follows `Link: rel="next"`

Revision 1 named no query parameters at all. The one that matters most:

```
GET /api/v1/courses/:c/assignments/:a/submissions?include[]=submission_history&per_page=100
```

`submission_history` is an **opt-in include** — `lib/api/v1/submission.rb` gates it on
`includes.include?("submission_history")`. Without it the payload carries only the current
attempt, `attempts_of` always takes its single-row fallback, `select_attempt` becomes a
no-op, and a course configured `attempt_policy = "earliest"` silently grades the latest
attempt. D4's de-duplication reasoning would also be unreachable in production while
passing every mock test. **The tests assert the outgoing query string, not just the parsed
result.**

`attachments` needs no include — it is in `SUBMISSION_OTHER_FIELDS` and is returned by
default. `grouped` is a valid parameter on this endpoint but is not wanted: it reshapes the
response around group submissions.

```
GET /api/v1/courses/:c/users?enrollment_type[]=student&enrollment_state[]=active&enrollment_state[]=completed&per_page=100
GET /api/v1/courses/:c/assignments/:a
GET /api/v1/courses?enrollment_type=teacher&per_page=100          # canvas courses
GET /api/v1/courses/:c/assignments?per_page=100                   # canvas assignments
```

`enrollment_state[]` is stated rather than inherited: the default omits `completed`, so a
student who dropped after submitting would be absent from `users` while present in
`submissions`, landing as `identity_for(user: None)` → a `CanvasUser` key and
`ReceivedUnmatched`. Visible, but it costs a teacher an investigation; including
`completed` makes them an ordinary enrollee.

Pagination: one generic `paginate::<T>(url) -> Result<Vec<T>, CanvasError>` issues the
request, parses `reqwest::header::LINK`, and follows the `rel="next"` target **verbatim**
because Canvas's cursors are opaque. Termination is "no `next` link", never "fewer than
`per_page` rows" — the docs say the `per_page` cap is unspecified. Two guards: a
`MAX_PAGES` cap (1000), and a hard error if `next` equals the URL just fetched.

### D2 — One set of Canvas types: the client deserialises into `input::canvas`

`client.rs::CanvasUser` is deleted — it is a second, thinner spelling of
`CanvasUserPayload`, and an unused public type mirroring a live one is exactly the drift
P-669 moved the payload types to prevent. `pull_roster` returns `Vec<CanvasUserPayload>`.

`CanvasSubmission` (the grade-push *response* shape) and `push_grades` are left alone —
P-680 owns them.

### D3 — `content-type`, with the underscore kept as an alias

```rust
#[serde(rename = "content-type", alias = "content_type")]
pub content_type: Option<String>,
```

`rename` makes the real spelling the one written back out, so a saved bundle round-trips
through the same struct; `alias` keeps every existing fixture deserialising.

### D4 — Downloads are collected as a set of attachments, then fetched once each

Walk every student → every attempt → every attachment into a
`BTreeMap<u64, &CanvasAttachmentPayload>`: one entry per id, ordered, so a file carried
forward across three attempts is fetched once, the order does not depend on `HashMap`
iteration, and the count shown to the teacher is the number of distinct files.

### D5 — The attachment `url` is used verbatim; timeouts and progress are explicit

Canvas builds the url with `download_frd=1` and `verifier=<attachment uuid>`, and redirects
to storage (S3). `reqwest` follows redirects but **strips `Authorization` across hosts** —
correctly, and harmlessly, because the verifier authorises the final hop. So: send bearer
auth (it serves the first, Canvas-host hop), follow redirects, do not re-attach the token.
An attachment with `url: None` is not a download failure but a payload that never offered
one.

**Timeouts.** `CanvasClient` is built with `reqwest::Client::new()`, which defaults every
timeout to `None`. A dead peer surfaces via TCP keepalive in about a minute; a
*live-but-stalled* hop — connection open, body never completing — never errors at all, and
at the default concurrency of 1 it blocks the entire import behind one file. The client
moves to `Client::builder().connect_timeout(10s).timeout(300s)`. 300s is for the body,
because a large attachment over a slow link is legitimate; the stall case is what the cap
catches. Without this, D8's `AttachmentUnavailable` boundary is unreachable for a stall —
there is no error to degrade.

**Progress.** CLAUDE.md requires progress display on long-running loops. The download loop
prints `i/n`, the filename and whether it was skipped by size; the import summary comes
after, not instead.

Downloads are **sequential by default**: Canvas throttles on a per-token cost bucket with
no published rate, and CLAUDE.md puts undocumented external APIs at sequential unless
asked. `--download-concurrency N` (default 1) raises it behind a `Semaphore`.

### D6 — The bundle is the fixture format, names are sanitised, and writes are atomic

```
<bundle>/
  canvas-payload.json          # exactly what was fetched, as CanvasPayload
  attachments.json             # id -> {path, size} | {error: reason}
  attachments/<id>/<name>      # one directory per attachment id
```

The bundle exists so `grade --canvas <bundle>` re-grades offline: fixing a test spec is not
another 200 downloads, a bundle captured from a real course *is* a `CanvasPayload` fixture,
and the id directory keeps two students' `hw1.py` apart while **preserving the original
filename**, which the ticket requires (保留原始文件对应关系).

**`attachments.json` is not optional bookkeeping.** D10 splits `canvas fetch` from
`grade --canvas`, so a download that failed at fetch time has no other way to reach the
grade-time diagnostic list. Without the manifest the loader can only observe "not on disk",
which collapses two different facts: *attempted and failed* (`AttachmentUnavailable`) and
*never attempted*, e.g. an interrupted fetch (`PendingDownload`). The loader builds D7's
`Result`-valued map from this file.

**The on-disk name is sanitised, and the writer and loader share one function.**
`CanvasAttachmentPayload::name()` prefers `display_name`, which Canvas stores as raw user
input with only a truncation — Canvas itself refuses to put it on a filesystem unsanitised.
A student who renames their upload to `../../../../evil.py` gets that string in the
submissions JSON. So:

```rust
fn disk_name(payload: &CanvasAttachmentPayload) -> String
```

takes `Path::new(&payload.name()).file_name()`, requiring a single non-`..` component, and
falls back to `attachment-<id>` with a diagnostic otherwise. **Both** the fetcher and the
offline loader call it — sanitising only on write would leave the loader re-deriving the
location from the raw `display_name`, missing every file and turning the class into
`PendingDownload`. `Attachment.filename` keeps the raw payload name: that field is
provenance, and the teacher should see what Canvas actually reported.

(`filename` is *not* the risky field and is *not* percent-encoded — attachment_fu's
`sanitize_filename` already reduces it to `[\w.-]`.)

**Writes are atomic**: download to `<name>.part`, then rename. This removes the truncation
window rather than trying to detect it afterwards.

**Re-fetch skips** an attachment when the file exists and its size equals
`attachment.size`. When Canvas reports no size, the file is re-downloaded — guessing that
an existing byte count is complete is how a truncated file becomes a permanent 0.

### D7 — A zip attachment is expanded, and a failed download is a value, not an absence

Today `detect_language("zip")` is `None`, so a downloaded `.zip` becomes an `IgnoredFile`
and a student who uploaded one — the common case for a multi-file assignment, and what
Canvas itself produces for multi-file uploads — is `SubmittedEmpty`.

```rust
pub struct ExpandedEntry { pub entry: String, pub path: PathBuf }
pub struct DownloadedAttachment { pub path: PathBuf, pub expanded: Vec<ExpandedEntry> }
pub type DownloadedAttachments = HashMap<u64, Result<DownloadedAttachment, String>>;
```

The `Result` is what lets a failed download carry its reason to the one place a student's
identity is in scope. In `attempts_of`: `Ok` with a non-empty `expanded` yields one
candidate per entry; `Ok` with an empty one yields the attachment itself, as now;
`Err(reason)` yields a keyed `AttachmentUnavailable`; and `None` goes back to meaning what
it says — never attempted — as a keyed `PendingDownload`. One seam, no parallel failure
map, no double-fire.

```rust
FileOrigin::Attachment { attempt: u32, attachment_id: u64, entry: Option<String> }
```

`entry` is `#[serde(default)]`, so old results still load; it is `Some` exactly when the
file came out of a zip attachment. The archive's own path is recoverable from
`attachment_id`, so a fourth `FileOrigin` variant would only add a second way to say the
same thing. A zip that expands to nothing runnable leaves its owner `SubmittedEmpty` with
the per-entry diagnostics explaining why.

**The extraction code is reused, not rewritten.** `discovery::extract_archives` already
handles traversal (`enclosed_name`), name collision, the `MAX_FILE_SIZE` /
`MAX_TOTAL_SIZE` / `MAX_FILE_COUNT` zip-bomb guards, noise filtering, and rollback of the
claim and provenance together when a write fails. Its inner loop is lifted into

```rust
pub(crate) fn expand_archive(archive: &Path, target: &Path, diagnostics: &mut Vec<InputDiagnostic>) -> Vec<ExtractedFile>
```

and `extract_archives` becomes the directory-scanning caller. Those guards have **zero test
coverage today**, and this lift puts them on a path fed directly by student-uploaded
archives, so a characterization test is written against the current `extract_archives`
*before* the move. Expanded entries land in `attachments/<id>/<archive stem>/`.

### D8 — The failure boundary, and three rules that were wrong

| Failure | Result |
| -- | -- |
| users / assignment / submissions request fails, or any page of one | hard `Err`; the existing bundle is left intact |
| attachment download fails (HTTP, IO, timeout, no url) | `AttachmentUnavailable { key, .. }`; recorded in `attachments.json` |
| attachment never attempted | `PendingDownload { key, .. }` |
| submission type we cannot run | `UnsupportedSubmissionType { key, submission_type }` |

A partial *listing* is fatal because the cohort is the thing being established: a
submissions page that silently went missing becomes a class of phantom 缺交 with no way for
the teacher to see it. A single attachment is the opposite — 单个附件失败不伪装成学生缺交 —
so it degrades to a diagnostic and the student keeps whatever else they handed in. The
fetch writes to a temp directory and renames into place, so a failed re-fetch cannot
clobber a good bundle.

**Diagnostics name the student.** `AttachmentUnavailable` and `PendingDownload` both carry
`key: String`, matching `UnsupportedSubmissionType`. Without it the failure list the ticket
demands (提供导入汇总与失败项) is a flat list of attachment ids that no teacher can act on.
Neither carries `attempt`: `normalize`'s `diagnostics.dedup()` folds the N identical pushes
a carried-forward attachment generates into one line, and `attempt` would un-fold them.

**Unsupported types are a default arm, not a whitelist.** Revision 1 enumerated five types,
which leaves any other — `basic_lti_launch` is reachable on exactly the assignments
ScriptMark grades — with no diagnostic at all. Inverted: anything that is not
`online_upload` (or `online_text_entry`, which keeps its more specific `TextEntryOnly`)
emits `UnsupportedSubmissionType` carrying whatever string Canvas sent. A null type on a
row with no attempt is already `NotSubmitted` and is not reported.

**A duplicate submission row keeps the row that has an attempt.** Canvas's
`submissions#index` is offset-paginated over a relation recomputed per request, so an
enrollment landing mid-walk shifts the window and re-reads a row. Revision 1 kept the
first, which is the older snapshot: if the student submitted between the two page fetches,
the kept copy is the placeholder, and a real submitter is reported 缺交 behind a warning
that says "kept the first". It stays a warning — escalating to an Error would refuse to
grade the whole class over transient skew a re-fetch resolves.

```rust
#[error("attachment {attachment_id} ('{filename}') could not be downloaded for '{key}': {reason}")]
AttachmentUnavailable { key: String, attachment_id: u64, filename: String, reason: String },
#[error("'{key}' submitted via '{submission_type}', which cannot be graded automatically")]
UnsupportedSubmissionType { key: String, submission_type: String },
```

### D9 — `roster-pull` round-trips the Canvas id, through a real CSV writer

`pull_roster` returns `Vec<CanvasUserPayload>`; the CSV gains a fourth column:

```
name,class,student_id,canvas_id
Alice,,2024010001,12345
Bob,,,12346
```

**`save_roster_csv` is rewritten on `csv::Writer`.** It currently writes
`writeln!(f, "{},,{}", name, student_id)` with no quoting, while `load_roster` reads with
`csv::ReaderBuilder`. Adding a fourth positional column to an unescaped writer converts a
loud failure into a silent mis-identification: a name containing a comma emits
`Wu, Alice,,2024010001,101`, which parses as five fields; `record.get(2)` is empty, and the
new index-3 read takes `2024010001` as the **canvas_id**, keying the student
`CanvasUser(2024010001)` — their own 学号 misread as a Canvas user id. They then match no
submission, and `push_grades` PUTs to a nonexistent user whose 404 is swallowed by
`eprintln!`. Today that row is merely dropped with an `UnusableRosterRow`.

The name column takes `CanvasUserPayload.name`, **never `sortable_name`**, which is
"Last, First" by construction and would shift every row.

`load_roster` reads index 3 as `canvas_id` for **all** rows, independently of the key
decision. It stays positional — column *mapping* is P-672's, and a half-mapping here is one
more thing for P-672 to tear out. The blank-`student_id` ⇒ `CanvasUser` fallback is gated
on the row width matching the header width, so a shifted row fails loudly instead of
guessing.

Bob's blank `student_id` is what forces this design: a Canvas enrollee with no SIS id is
keyed `StudentKey::CanvasUser(12346)`, which renders `canvas:12346` — and `load_roster`
**rejects** a reserved prefix as unusable. Writing the rendered key into `student_id` would
drop exactly the students P-669's reversed D9 was meant to protect.

**The chain this actually completes** is `load_roster` → `discovery.rs` (both the matched
and roster-only branches) → `orchestrator` → `results.json` → `cmd_grades_push`, which
reads `report.canvas_user_id`. `db::import_roster` also stores `canvas_id`, but
`cmd_grades_push` opens no database — Revision 1 named the wrong path.

**A `roster.csv` written by the old `roster-pull` must be re-pulled.** Its `student_id` for
a SIS-less enrollee is a fabricated value (the `login_id`, or the Canvas id as a string),
which keys as `Number(...)` with no `canvas_user_id`, never matches the `CanvasUser(id)`
row enrollment produces, and so appears twice — once graded, once 缺交. A stale row is
deliberately not merged; it surfaces as `NotEnrolled`.

### D10 — CLI: fetching is its own step, and `assignment.toml` still wins where it should

```
scriptmark canvas courses                                   # 直接选择
scriptmark canvas assignments --course-id N
scriptmark canvas fetch --course-id N --assignment-id M -o canvas/hw1
scriptmark grade --canvas canvas/hw1 -t tests/              # offline, repeatable
scriptmark run   --canvas canvas/hw1 -t tests/
```

`--canvas-url` falls back to `CANVAS_URL`, as `CANVAS_TOKEN` already works.
`--course-id` / `--assignment-id` default to `assignment.toml`'s `canvas_course_id` /
`canvas_assignment_id`, which P-669 added and nothing currently reads.

Splitting fetch from grade is the point: re-running a spec must not re-download a class's
work, and the bundle is what makes a run reproducible.

**`grade --canvas` merges `assignment.toml`, and Revision 1 dropped it on the floor.**
`normalize` requires an `AttemptPolicy`; the bundle has no policy field and Revision 1 added
no flag, so the only source is `load_assignment` — which returns the policy and the
`Assignment` in one tuple. Writing `let (_assignment, policy) = …` compiles clean, passes
`clippy -D warnings`, and silently discards the declared items; passing
`AttemptPolicy::default()` instead ignores a teacher's `attempt_policy = "earliest"` and
grades every student on their latest attempt. So:

- `normalize` takes the loaded `Assignment`, keeping the payload's Canvas ids as a fallback
  for fields the toml leaves unset;
- toml supplies `name`, `items` and `attempt_policy`; the bundle supplies
  `canvas_course_id` / `canvas_assignment_id`;
- when both carry ids and they **differ**, refuse with an error naming both — grading one
  assignment's submissions against another's declaration is not recoverable;
- `reconcile_items` runs on the merged result, so P-669's declared-item reporting works on
  this path too.

`--canvas` and the positional submission dirs are mutually exclusive. `submissions` is
`#[arg(required = true)]` today, so this needs `required_unless_present("canvas")` plus
`conflicts_with` — a clap restructure, not a one-line addition.

`grade --canvas` reuses `build_local_input`'s shape: report the import summary, then refuse
to grade if any diagnostic is an `Error`.

### D11 — 免交 has somewhere to live

The ticket requires 保留缺交、迟交、免交等来源状态. `SourceStatus` lives only on
`SubmissionAttempt`. A Canvas placeholder row has `attempt: null`, so `attempts_of` skips
every row and `normalize` hits `if attempts.is_empty() { continue }` before `source_status()`
is ever called; the student is re-created by the roster merge through `not_submitted()`,
which sets `attempts: Vec::new()`.

缺交 itself is fine — `SubmissionOutcome::NotSubmitted` *is* that fact, and Canvas's
`missing: true` on a never-submitted row is redundant with it. What is unrepresentable is
**免交**: a teacher excuses a student who never submitted, so `attempt` stays null, and the
student is emitted indistinguishable from someone who simply forgot.

`normalize` retains the skipped placeholder rows in a `BTreeMap<u64, &CanvasSubmissionPayload>`
and `not_submitted(identity, roster_index, source_status)` carries the row's status through.
This works because `merged_roster` enrols every `payload.users` entry, so every placeholder
row's owner is recreated in the roster-merge loop — that is the load-bearing check.

To keep the fact in one place rather than two, `StudentSubmission::source_status()` reads
the selected attempt's when there is one and the placeholder's otherwise, with a
`debug_assert!` that the stored field is `None` whenever `attempts` is non-empty.
`report_input` counts excused separately, or the status is preserved and still invisible.

No `Excused` variant is added to `SubmissionOutcome`: this is provenance, and
zero-vs-ungraded is P-677's. Nothing here changes a grade today — P-669's D6 already leaves
every non-`Executable` report at `final_grade: None`.

### D12 — The import writes `input.json`, which is where 明确使用哪次提交 lands

The ticket asks the import to 输出统一输入模型 and to 明确使用哪次提交. Revision 1 did
neither durably: the selected attempt existed only in memory and reached no teacher-visible
surface.

`grade --canvas` and `canvas fetch` write the normalised `AssignmentInput` to
`<bundle>/input.json`. `StudentSubmission.selected` **is** the durable record of which
attempt was graded, per student, with its `submitted_at` and its files' provenance beside
it. The import summary additionally prints how many students are graded on an attempt later
than their first.

Deliberately **not** done: adding `attempt` / `submitted_at` to `StudentReport`.
`discovery.rs` synthesises `SubmissionAttempt::new(1)` for every local submission, so that
field would stamp `Some(1)` on every locally-graded student — the source-neutral false
default P-669 rejected in so many words ("`late: false` on a local submission is a false
statement, not a neutral default"). The grading result format is P-678's.

## Touch list

| File | Change |
| -- | -- |
| `canvas/client.rs` | `Link`-header `paginate`; timeouts on the builder; `pull_roster -> Vec<CanvasUserPayload>`; `fetch_assignment`, `fetch_submissions`, `list_courses`, `list_assignments`, `download_attachment`; delete `CanvasUser`; `save_roster_csv` on `csv::Writer` + `canvas_id` |
| `canvas/bundle.rs` *(new)* | write/read `canvas-payload.json`, `attachments.json`, `attachments/<id>/`; `disk_name`; atomic write; skip-by-size; zip expansion; `input.json` |
| `canvas/mod.rs` | re-export the bundle loader/fetcher |
| `input/canvas.rs` | `content-type` rename; `Result`-valued `DownloadedAttachments`; `attempts_of` reads `expanded` + keyed diagnostics; retained placeholder rows; `normalize` takes an `Assignment`; duplicate-row rule; `UnsupportedSubmissionType` default arm |
| `models/submission.rs` | `FileOrigin::Attachment` += `entry`; `StudentSubmission` += `source_status` + accessor; `not_submitted` signature; `key` on `AttachmentUnavailable` / `PendingDownload`; 2 new `DiagnosticKind` variants |
| `discovery.rs` | extract `expand_archive`; `not_submitted` call site |
| `roster.rs` | `load_roster` reads column 3 as `canvas_id`; width-gated blank-id ⇒ `CanvasUser` |
| `main.rs` | `canvas` subcommand group; `--canvas` on grade/run with `required_unless_present` + `conflicts_with`; toml merge; `report_input` gains excused + later-attempt lines; `cmd_roster_pull` |
| `tests/input_equivalence.rs` | 2 `DownloadedAttachments` literals + the `values()` `is_file` loop |
| `Cargo.toml` | dev-dep `wiremock` |

**Release note.** `pull_roster`'s return type, `DownloadedAttachments`,
`FileOrigin::Attachment`'s shape, `not_submitted`'s and `normalize`'s signatures and the new
`DiagnosticKind` variants are all crate-public. The last shipped tag is `v0.2.0` and the
workspace is already at an unreleased `0.3.0` (bumped by P-669), so these fold into 0.3.0 —
**no further version bump**.

## Tests

`wiremock` drives the real `reqwest` client, so the `Link` parser, redirect handling and
skip-by-size are exercised rather than mocked away.

Against a mock server:

1. two pages of users joined by a `Link: …; rel="next"` header, and no third request;
2. a `next` link pointing at the URL just fetched ⇒ error, not a spin;
3. **the submissions request carries `include[]=submission_history`** — asserted on the
   outgoing query string;
4. a submission with two attachments, both downloaded;
5. two students whose attachments share the filename `hw1.py` ⇒ two files, two directories,
   both still named `hw1.py`;
6. the same `attachment_id` on attempts 1 and 2 ⇒ exactly one GET;
7. two submission rows for one `user_id`, one a placeholder ⇒ the one with an attempt wins;
8. an `unsubmitted` placeholder row ⇒ `NotSubmitted`, and no download attempted;
9. a download returning 500 ⇒ `AttachmentUnavailable` **naming the student**, the student
   keeps their other file and is **not** `NotSubmitted`;
10. a submissions page returning 500 ⇒ hard `Err`, and a pre-existing bundle is untouched;
11. re-fetch, in three parts: (a) on-disk size == `attachment.size` ⇒ that mock `.expect(0)`;
    (b) a file truncated below `attachment.size` ⇒ `.expect(1)` and the file is whole
    afterwards; (c) `"size": null` ⇒ `.expect(1)`. Only (b) and (c) discriminate — Revision
    1's single "zero GETs" assertion passes an `if path.exists() { continue }`
    implementation, which is the bug D6 exists to prevent;
12. an `online_url` submission ⇒ `UnsupportedSubmissionType` naming the type, and an
    unlisted type (`basic_lti_launch`) ⇒ the same, via the default arm.

Without a server:

13. a zip attachment expanding to two `.py` files with `FileOrigin::Attachment { entry: Some(..) }`;
14. a characterization test for the zip-bomb guards written **before** `expand_archive` is
    lifted: a zip with one zero-filled 6 MB entry (a few KB deflated) plus a real `.py` ⇒
    `ArchiveEntrySkipped` naming the byte limit, and the `.py` still extracts;
15. `"content-type"` and `"content_type"` both deserialising, and a bundle we wrote
    reloading with `content_type` populated;
16. roster round-trip through `save_roster_csv` → `load_roster` with an enrollee name
    containing **a comma and a double quote**, asserting key, name and canvas id all
    survive — plus, on the same file, `Alice,,2024010001,12345` ⇒
    `key == Number("2024010001")` **and** `canvas_user_id == Some(12345)`, which is the
    load-bearing half;
17. an attachment whose `display_name` is `../../../evil.py` ⇒ written inside the bundle
    under a sanitised name, with a diagnostic;
18. an excused placeholder row (`attempt: null, excused: true`) ⇒ `NotSubmitted` with
    `source_status().excused == true`;
19. a bundle round-trip: fetch → `attachments.json` → offline load ⇒ the same
    `AssignmentInput`, including a recorded download failure coming back as
    `AttachmentUnavailable` rather than `PendingDownload`;
20. bundle Canvas ids disagreeing with `assignment.toml`'s ⇒ refused, naming both;
21. a legacy `results.json` with no `entry` field still loading.

Any committed bundle fixture is checked with `git add -n` before it is relied on —
`.gitignore` has broad `*.csv` / `submissions` rules and P-669 needed an explicit
`!crates/scriptmark/tests/fixtures/**` exception.

## Verification

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Locally `scriptmark-py` needs `PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1` (machine Python is
3.14, above pyo3 0.24's ceiling), and cargo needs the copied `CARGO_HOME` under `$TMPDIR`.

Real-course verification is **not** in this ticket's automated suite — P-663 asks that mock,
repo-fixture and real-Canvas runs be recorded separately, and only the first two can run in
CI. No Canvas credentials are configured in this environment, so the third is not attempted
here; the bundle format is what makes it reproducible when a test course exists.
