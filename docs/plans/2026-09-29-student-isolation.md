# P-745 — Isolating student code: research and design

Linear: https://linear.app/acturea/issue/P-745
Parent: P-663 · Related: P-674, P-744, P-678

This records the research done while PR #4 (P-674) was in review, and the decisions the
owner made on it. It is the starting point for P-745, not a finished plan.

## Why

After P-674 a unit is one Python process. The harness, the teacher's code (modules,
function checkers, reference implementations) and the student's module share it, and it
runs as the grader's user. A student can therefore:

- read the bundle, expected values included, from disk;
- import past the allowlist with `importlib`;
- forge the records of their own calls, including a passing function checker's verdict
  (`run_calls` looks up `run_check` in `__main__` on every call);
- and teacher and student imports share one namespace, so a file on one side can stand in
  for a module the other side imports.

**Threat model (owner's decision).** The courses are introductory. Isolation must contain
accidents (loops, deleted or overwritten files, full disks, reading the wrong file) and
naive cheating (reading answers, forging results, touching other students' work). It does
not aim to stop a skilled student who sets out to attack the grader.

## Architecture (owner's decisions)

```
ScriptMark (Rust, not sandboxed): spawns both processes, joins them, receives the result
 ├─ teacher sandbox: teacher modules, function and Python-script checkers, reference
 │    implementations, lint tools; drives the calls and writes the records
 │    reads the bundle and a read-only copy of this student's source; no network;
 │    cannot see other submissions or the grader's environment (Canvas token)
 └─ student sandbox: the student's module only
      reads the interpreter (or P-744's environment) and its work dir; writes the work
      dir and its own HOME/TMPDIR; no network, no exec, no signals to other processes;
      cannot see the bundle, other students or $HOME
```

- The two sandboxes talk over one pipe, directly; ScriptMark does not relay.
- Teacher code gets a sandbox of its own, not ScriptMark's privileges: it handles data the
  student produced, and ScriptMark holds the Canvas token and every student's files.
- Fail closed: a unit whose sandbox cannot be applied is not graded. `--no-sandbox` opts
  out explicitly, and the results say so.
- `==` and other operations on student-produced values are the teacher code's to guard.
- The import allowlist stays as a teaching rule; the OS is the boundary.

### What crosses between the sandboxes

The owner chose values over remote proxies. What a checker receives:

| The student's value | Crosses as | The checker can |
|--|--|--|
| plain data: numbers, strings, lists, dicts | JSON, as today | everything |
| an instance of a class from a **shared module** | the class's name and the instance's state; rebuilt in the teacher sandbox from the teacher's own copy of the class, without `__init__` | everything: methods, properties and `isinstance` run the teacher's code |
| an instance of a class the student wrote | a snapshot: instance attributes, plus public property values computed in the student sandbox | read attributes; the student's methods are tested through scenario steps (method and attribute targets exist since P-674) |

- **Shared modules.** `[meta] shared = ["common.py"]` names teacher-provided definitions.
  They are staged into the student's work dir and importable there without an
  `allowed_imports` entry, and the teacher sandbox imports its own copy from the bundle.
  Changing one copy does not change the other.
- **Identity survives.** Object graphs keep shared references and cycles (a linked list
  with a cycle, a tree with a shared node), so the serialiser needs ids and references;
  today's `to_json` writes `"<cycle>"`.
- A student subclass of a shared class is rebuilt as the shared base.
- An exception or timeout while snapshotting a student object is the student's.
- Arguments the teacher passes to student calls cross by value only.

**Rejected for this:**

- *pickle*: rebuilding a student object needs the student's class, which means importing
  the student's module in the teacher sandbox: the split undone.
- *RPyC* (MIT, 6.0.2): the closest off-the-shelf fit, but symmetric (a list passed to a
  student method arrives as a reference back into the caller), it depends on `plumbum` in
  every environment that runs the harness, and its request loop would need our call timer
  fitted into it. CVE-2024-27758 (CVSS 8.4, fixed in 6.0.0) was a peer gaining code
  execution through `np.array(netref)`: the direction that matters here. Not decisive for
  introductory courses, but the dependency and the fit are.
- *Pyro5*: remote access needs `@expose` (applicable at runtime), plain instance
  attributes cannot be exposed at all, only registered objects are auto-proxied, and
  dunder operations are not forwarded. Adapting it would take more code than the value
  rule above.

## Sandboxes

### macOS: Seatbelt, self-applied

Each process calls `sandbox_init` through `ctypes` after the interpreter starts and before
untrusted code loads. Verified on macOS 26.6.2 with Python 3.14.7, under a lock:

- reads outside the allowed roots are denied, including the bundle under `$HOME`, sibling
  units' directories, listings of `$TMPDIR`, `/Users` and `/private/tmp`; so are `stat`s;
- writes outside the work dir, `subprocess`, re-running python and `os.fork` are denied;
- `connect` to the network is denied; AF_UNIX socketpairs still work (`asyncio.run` does);
- signals reach only the process itself; `setitimer`/SIGALRM still fires;
- inherited descriptors (the record pipe) still work;
- a second `sandbox_init` from student code fails: the lock cannot be undone.

**Overhead** (median of 40 runs):

| Run | Median |
|--|--|
| `python3 -I -c pass` | 21.8 ms |
| with `import ctypes` | 25.1 ms |
| self-locked | 38.0 ms (+16 ms; `sandbox_init` itself 11.4–12.3 ms, mostly compiling `system.sb`) |
| `sandbox-exec` wrapper | 34.6 ms against 21.1 ms (+13.5 ms) |

**Rules the profile needs, from the research and its critique:**

- The read root is what the interpreter needs, **not the whole Homebrew prefix**:
  `/opt/homebrew/var` holds database data and `/opt/homebrew/etc` holds ssh, gnupg and
  service configs. With only the Python keg readable, `lzma`, `ssl` and `sqlite3` fail to
  import and `decimal` falls back to `_pydecimal`, so compute the dependency closure or
  curate the list. P-744's environment directory joins the read roots.
- Deny `sysctl` reads of `kern.procargs*`: a same-user process can read another's
  arguments and environment, the grader's `CANVAS_TOKEN` included (mechanism checked, not
  probed: nested sandboxes fail).
- Check whether `(version 1)` in the profile re-enables blanket XPC lookup through
  `system.sb`; prefer a later version if the denials hold.
- It cannot nest: under another Seatbelt sandbox (Claude Code's own) `sandbox_init` fails
  with EPERM. Fail closed, and run the sandbox tests outside such a sandbox, as the Canvas
  tests already are.
- Self-check: ScriptMark writes a canary outside the work dir; a unit that can still open
  it after locking refuses to continue.
- SBPL is undocumented and `sandbox-exec` is marked deprecated (it printed no warning on
  26.6.2). Chromium, Bazel, Codex CLI and Claude Code rely on the same engine. A macOS CI
  job asserts every denial above.
- Seatbelt has no memory limit and `RLIMIT_AS` is unreliable on macOS: ScriptMark watches
  each unit's memory and ends a unit past its limit.

### Linux: Landlock and seccomp (unverified)

Nothing here ran on Linux; it comes from kernel and crate documentation. The first step is
a probe suite in ubuntu-24.04 CI.

- Filesystem: a Landlock ruleset (`landlock` 0.4.7), read and execute on the interpreter's
  prefix and system libraries, read and write on the work dir; required ABI enforced.
- Syscalls: a seccomp filter (`seccompiler` 0.5.0) denying network sockets, `execve`,
  `ptrace`, `mount`, `unshare`, `bpf`, `io_uring_*` and process creation other than
  threads.
- **Unix sockets:** Landlock governs pathname sockets only from ABI 9, so deny `connect`
  outright below it (socketpairs need none).
- **Signals:** Landlock scopes them only from ABI 6 (kernel 6.12). Below that, deny the
  kill family (breaking `os.kill(os.getpid(), …)`, not `setitimer`), or compile the filter
  per unit with the worker's pid. For introductory courses, denying is enough.
- No privileges and no user namespaces, so Ubuntu 24.04's AppArmor restriction does not
  apply. `/proc` must stay out of the read roots (it would expose other processes'
  environments).

**Rejected sandboxes:** bubblewrap, nsjail and hakoniwa need user namespaces, which stock
Ubuntu 24.04 restricts (useful as optional hardening); isolate needs root; nono was four
months old with 78 minor releases; birdcage is archived and GPL; Anthropic's
sandbox-runtime needs Node and allows exec by design; Apple `container` and WASI are heavy
per unit (WASI is unmeasured) and stay a possible hardened mode. A dedicated grading UID,
as contest judges use, would remove the same-user signal and `procargs` classes of
problem; it needs one-time setup, not per-run privilege.

Windows has no counterpart: it keeps the nonce-framed stdout channel and no isolation.

## The record channel

Today Rust buffers the whole stream up to a cap sized from the plan (P-674's CodeRabbit
round) and parses it after the process exits. The cap protects the grader's memory, but
its size is a guess.

- Frame each record as a length and JSON; read frames as they arrive.
- Bound each record by the largest record legal at that point of the plan; ScriptMark
  passes the limits in the payload so they live in one place.
- The number of records is already fixed by the plan (one `ready`, one `done`, one per call
  and per check), so the total follows without a guess.
- A frame past its bound ends the stream there, `Flooded`, and earlier records stand.
- Frames from the student sandbox to the teacher sandbox are checked against the pending
  request's bound before they are read; JSON nesting depth and `$bigint` digits are
  bounded.

## Also in scope

- Per-unit timer ownership: the student side keeps its per-call timer; the teacher side
  sets a deadline on each pending request, and on expiry the student process is ended and
  the remaining calls are `not_run`, the student's.
- `[lint]` runs in the teacher sandbox on the read-only copy; its command is no longer split
  on whitespace (a file name with a space breaks it), and every graded file is linted, not
  only the first.
- A reserved checker parameter `source`, the student's source text, for `ast`-based rules.
- Python-script checkers run in the teacher sandbox; today ScriptMark starts them with its
  whole environment.
- `prepare` refuses a teacher export that is a path into the bundle but not staged through
  `data_files`: under a sandbox the student cannot read it. One private spec needs this
  migration.
- A total disk quota for the work dir (`RLIMIT_FSIZE` bounds one file only).

## Cost

- Seatbelt: about 16 ms per sandboxed process (measured above).
- A second Python process for units with teacher code: about 22 ms to start, 36 ms with
  the harness's imports (the architecture report's measurement), in parallel with the
  student process, so CPU rather than latency.
- Size: the architecture report estimated the split at 1.2–1.5k lines including tests; the
  value rule replaces its proxies and should come in under that.

## Open decisions

- When Linux is supported: macOS first is recommended.
- Windows: run only with `--no-sandbox`, or refuse.

## Provenance

- Research ran as four surveys (macOS, Linux, cross-platform crates, architecture in this
  repo), a synthesis and a completeness critique, on 2026-09-28.
- Crate versions from crates.io that day: landlock 0.4.7 (2026-07-27), seccompiler 0.5.0
  (2025-03-07), nono 0.78.0, hakoniwa 1.8.0, birdcage 0.8.1 (archived). RPyC 6.0.2 and its
  `plumbum` dependency from PyPI; Pyro5's exposure rules from its server documentation.
- Claims about the private course specs (which use `teacher =`, how their checkers read
  student objects) rest on files outside the repository.
