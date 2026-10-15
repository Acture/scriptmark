//! What a unit's processes may do, and the operating system's means of holding them to it.
//!
//! Every process a unit starts runs under the kernel's resource limits (`setrlimit`).
//! Sandboxed, each also runs confined to what its side needs: on macOS by Seatbelt, under
//! `sandbox-exec` with a profile written here; on Linux by Landlock (files, TCP, signals,
//! abstract sockets) and seccomp (other sockets, connecting, starting processes, tracing),
//! both applied between fork and exec. A confinement that cannot be applied fails the
//! spawn: nothing runs unconfined by accident. Windows has no confinement yet.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Resource limits for student code execution.
///
/// Applied via `setrlimit` in a `pre_exec` hook (between fork and exec),
/// so limits are kernel-enforced on the child process only.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
	/// CPU time limit in seconds (RLIMIT_CPU). Default: 30.
	pub cpu_secs: u64,
	/// Memory limit in MB: RLIMIT_AS on Linux, and the grader's watch on the student side
	/// everywhere. Default: 512.
	pub mem_mb: u64,
	/// Max file write size in MB (RLIMIT_FSIZE). Default: 10.
	pub fsize_mb: u64,
	/// Max open file descriptors (RLIMIT_NOFILE). Default: 64.
	pub nofile: u64,
	/// Max processes for this UID (RLIMIT_NPROC). Default: 64.
	/// It counts every process of the user — and on Linux every thread — so a low cap
	/// refuses even a thread on a busy machine. A sandboxed unit cannot start processes at
	/// all, so its cap only stops a runaway of threads, and sits far higher
	/// (`CONFINED_NPROC`); the harness itself starts none.
	pub nproc: Option<u64>,
}

/// The per-user process cap of a confined unit: room for a busy desktop's own threads, and
/// a stop to a student's thread bomb.
pub const CONFINED_NPROC: u64 = 4096;

impl Default for SandboxConfig {
	fn default() -> Self {
		Self {
			cpu_secs: 30,
			mem_mb: 512,
			fsize_mb: 10,
			nofile: 64,
			nproc: Some(64),
		}
	}
}

/// Apply resource limits to a `Command` via `pre_exec`.
///
/// This must be called before spawning the command. The closure runs
/// in the forked child process before exec, so it only affects the
/// student Python process.
#[cfg(unix)]
pub fn apply_sandbox(cmd: &mut tokio::process::Command, config: &SandboxConfig) {
	let cpu = config.cpu_secs;
	#[cfg(not(target_os = "macos"))]
	let mem = config.mem_mb * 1024 * 1024;
	let fsize = config.fsize_mb * 1024 * 1024;
	let nofile = config.nofile;
	let nproc = config.nproc;

	// SAFETY: setrlimit is async-signal-safe and we only call it
	// in the child process between fork() and exec().
	unsafe {
		cmd.pre_exec(move || {
			// Best-effort: some limits may not be supported on all platforms
			// (e.g. RLIMIT_AS on macOS, RLIMIT_NPROC when user has many processes)
			// Hard limit a second above the soft one: Linux sends SIGKILL at the hard limit and
			// SIGXCPU only at the soft one, and SIGXCPU is what says "ran out of CPU time".
			let _ = set_rlimits(libc::RLIMIT_CPU, cpu, cpu.saturating_add(1));
			let _ = set_rlimit(libc::RLIMIT_FSIZE, fsize);
			// A crash must not write the process's memory, teacher values included, to disk.
			let _ = set_rlimit(libc::RLIMIT_CORE, 0);
			let _ = set_rlimit(libc::RLIMIT_NOFILE, nofile);
			if let Some(nproc) = nproc {
				let _ = set_rlimit(libc::RLIMIT_NPROC, nproc);
			}
			// RLIMIT_AS: skip on macOS where it's unreliable
			#[cfg(not(target_os = "macos"))]
			let _ = set_rlimit(libc::RLIMIT_AS, mem);
			Ok(())
		});
	}
}

/// Limits for a teacher's tool — a checker script or a lint tool — run outside a unit: its
/// own process group, so it ends with everything it started, a CPU limit a second past
/// its deadline, and no file larger than `SandboxConfig`'s or any core dump.
#[cfg(unix)]
pub fn limit_tool(cmd: &mut std::process::Command, deadline_secs: u64) {
	use std::os::unix::process::CommandExt;
	let cpu = deadline_secs.saturating_add(1);
	let fsize = SandboxConfig::default().fsize_mb * 1024 * 1024;
	cmd.process_group(0);
	// SAFETY: setrlimit is async-signal-safe; this runs between fork and exec.
	unsafe {
		cmd.pre_exec(move || {
			let _ = set_rlimits(libc::RLIMIT_CPU, cpu, cpu.saturating_add(1));
			let _ = set_rlimit(libc::RLIMIT_FSIZE, fsize);
			let _ = set_rlimit(libc::RLIMIT_CORE, 0);
			Ok(())
		});
	}
}

#[cfg(target_os = "macos")]
type RlimitResource = libc::c_int;
#[cfg(target_os = "linux")]
type RlimitResource = libc::__rlimit_resource_t;

#[cfg(unix)]
fn set_rlimit(resource: RlimitResource, limit: u64) -> std::io::Result<()> {
	set_rlimits(resource, limit, limit)
}

#[cfg(unix)]
fn set_rlimits(resource: RlimitResource, soft: u64, hard: u64) -> std::io::Result<()> {
	let rlim = libc::rlimit {
		rlim_cur: soft as libc::rlim_t,
		rlim_max: hard as libc::rlim_t,
	};
	let ret = unsafe { libc::setrlimit(resource, &rlim) };
	if ret != 0 {
		return Err(std::io::Error::last_os_error());
	}
	Ok(())
}

/// Whether a run's units are confined by the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Isolation {
	/// Each side in its own sandbox; a unit whose sandbox cannot be applied is not graded.
	#[default]
	Sandboxed,
	/// `--no-sandbox`: resource limits only. Student code can reach whatever the grader's
	/// user can, and the results say so.
	Unisolated,
}

impl Isolation {
	pub fn is_sandboxed(self) -> bool {
		self == Isolation::Sandboxed
	}
}

/// What one confined process may touch. Paths must be canonical: the sandbox sees the paths
/// the kernel resolves, never the symlinks that lead to them.
#[derive(Debug, Clone, Default)]
pub struct Confinement {
	/// Directories and files it may read.
	pub read: Vec<PathBuf>,
	/// Directories it may read and write.
	pub write: Vec<PathBuf>,
	/// What it may execute in place of itself: the interpreter, which re-executes itself on
	/// some platforms, and its loader.
	pub exec: Vec<PathBuf>,
	/// It may start other processes, running any program it may read: lint tools. Never
	/// the student side.
	pub spawn: bool,
	/// Native calls (`ctypes`, `cffi`) are refused. On macOS they are how a process reads
	/// another same-user process's arguments and environment, which Seatbelt does not
	/// mediate.
	pub no_ffi: bool,
}

/// Whether this machine can confine a unit, and if not, why — in words a teacher can act on.
pub fn available() -> Result<(), String> {
	platform::available()
}

/// A command that runs `program` held to `confinement`. Its arguments are the caller's to add.
pub fn confined(
	program: &Path,
	confinement: &Confinement,
) -> Result<std::process::Command, String> {
	platform::command(program, confinement)
}

/// Where the operating system keeps the programs a teacher's tool may run, and what they
/// read: never a home directory.
pub fn system_programs() -> Vec<PathBuf> {
	#[cfg(target_os = "macos")]
	let places = [
		"/usr",
		"/bin",
		"/sbin",
		"/System",
		"/Library/Frameworks",
		"/private/etc",
		"/private/var/db/timezone",
	]
	.as_slice();
	#[cfg(not(target_os = "macos"))]
	let places = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"].as_slice();
	places
		.iter()
		.filter_map(|p| std::fs::canonicalize(p).ok())
		.collect()
}

/// The program the kernel loads to start `executable`: its ELF interpreter, which a
/// confined process must be allowed to run too.
#[cfg(not(target_os = "macos"))]
pub fn loader_of(executable: &Path) -> Option<PathBuf> {
	use std::io::{Read, Seek, SeekFrom};
	const PT_INTERP: u32 = 3;
	let mut file = std::fs::File::open(executable).ok()?;
	let mut header = [0u8; 64];
	file.read_exact(&mut header).ok()?;
	// 64-bit, little-endian ELF: what ScriptMark's Linux builds run on.
	if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
		return None;
	}
	let u16_at = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]) as u64;
	let phoff = u64::from_le_bytes(header[0x20..0x28].try_into().ok()?);
	let (size, count) = (u16_at(0x36), u16_at(0x38));
	for index in 0..count {
		let mut entry = [0u8; 56];
		file.seek(SeekFrom::Start(phoff + index * size)).ok()?;
		file.read_exact(&mut entry).ok()?;
		if u32::from_le_bytes(entry[..4].try_into().ok()?) != PT_INTERP {
			continue;
		}
		let offset = u64::from_le_bytes(entry[8..16].try_into().ok()?);
		let length = u64::from_le_bytes(entry[32..40].try_into().ok()?).min(4096) as usize;
		let mut path = vec![0u8; length];
		file.seek(SeekFrom::Start(offset)).ok()?;
		file.read_exact(&mut path).ok()?;
		let path = std::str::from_utf8(path.split(|b| *b == 0).next()?).ok()?;
		return std::fs::canonicalize(path).ok();
	}
	None
}

/// The shortest list covering `paths`: one inside another adds nothing.
pub fn covering(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
	paths.sort();
	paths.dedup();
	let mut kept: Vec<PathBuf> = Vec::new();
	for path in paths {
		if !kept.iter().any(|k| path.starts_with(k)) {
			kept.push(path);
		}
	}
	kept
}

#[cfg(target_os = "macos")]
mod platform {
	use super::*;

	const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

	pub fn available() -> Result<(), String> {
		let probe = std::process::Command::new(SANDBOX_EXEC)
			.args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
			.output()
			.map_err(|e| format!("{SANDBOX_EXEC} cannot run: {e}"))?;
		if probe.status.success() {
			return Ok(());
		}
		Err(format!(
			"{SANDBOX_EXEC} could not apply a sandbox ({}); a process already inside another sandbox cannot start one",
			String::from_utf8_lossy(&probe.stderr).trim()
		))
	}

	pub fn command(program: &Path, c: &Confinement) -> Result<std::process::Command, String> {
		let mut cmd = std::process::Command::new(SANDBOX_EXEC);
		cmd.arg("-p").arg(profile(c)?).arg(program);
		Ok(cmd)
	}

	/// An SBPL string literal.
	fn quote(path: &Path) -> Result<String, String> {
		let text = path
			.to_str()
			.ok_or_else(|| format!("'{}' is not valid UTF-8", path.display()))?;
		Ok(format!(
			"\"{}\"",
			text.replace('\\', "\\\\").replace('"', "\\\"")
		))
	}

	fn subpaths(paths: &[PathBuf]) -> Result<String, String> {
		paths
			.iter()
			.map(|p| quote(p).map(|q| format!("(subpath {q})")))
			.collect::<Result<Vec<_>, _>>()
			.map(|all| all.join(" "))
	}

	/// The Seatbelt profile for `c`. Everything is denied but what `system.sb` grants every
	/// process (the dyld cache, system frameworks, `/dev/null`, `/dev/urandom`), and:
	pub(super) fn profile(c: &Confinement) -> Result<String, String> {
		let mut rules = vec![
			"(version 1)".to_string(),
			"(deny default)".to_string(),
			"(import \"system.sb\")".to_string(),
		];
		let readable = covering(c.read.iter().chain(&c.write).cloned().collect());
		rules.push(format!("(allow file-read* {})", subpaths(&readable)?));
		// Resolving a path, and finding the working directory, stats every directory above it.
		let ancestors: Vec<PathBuf> = readable
			.iter()
			.flat_map(|p| p.ancestors().skip(1).map(Path::to_path_buf))
			.collect::<std::collections::BTreeSet<_>>()
			.into_iter()
			.collect();
		if !ancestors.is_empty() {
			let literals = ancestors
				.iter()
				.map(|p| quote(p).map(|q| format!("(literal {q})")))
				.collect::<Result<Vec<_>, _>>()?;
			rules.push(format!("(allow file-read-metadata {})", literals.join(" ")));
		}
		if !c.write.is_empty() {
			rules.push(format!("(allow file-write* {})", subpaths(&c.write)?));
		}
		if !c.exec.is_empty() {
			rules.push(format!("(allow process-exec {})", subpaths(&c.exec)?));
		}
		rules.push("(allow signal (target self))".into());
		if c.spawn {
			rules.push("(allow process-fork)".into());
			rules.push("(allow process-exec)".into());
			rules.push("(allow signal (target children))".into());
		}
		if c.no_ffi {
			rules.push(
				r#"(deny file-read* (regex #"/_ctypes[^/]*\.so$") (regex #"/_cffi_backend[^/]*\.so$"))"#
					.into(),
			);
			// Nor by another name: no hard link to the library, and no native code loaded
			// from anywhere it can write.
			rules.push("(deny file-link)".into());
			if !c.write.is_empty() {
				rules.push(format!(
					"(deny file-map-executable {})",
					subpaths(&c.write)?
				));
			}
		}
		Ok(rules.join("\n"))
	}
}

#[cfg(target_os = "linux")]
mod platform {
	use super::*;
	use landlock::{
		ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
		RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
	};
	use seccompiler::{
		BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
		SeccompRule, TargetArch,
	};
	use std::collections::BTreeMap;

	/// The Landlock ABI the running kernel offers: 0 when it has none.
	fn kernel_abi() -> i64 {
		const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
		// SAFETY: with this flag the call only reports a number.
		unsafe {
			libc::syscall(
				libc::SYS_landlock_create_ruleset,
				std::ptr::null::<u8>(),
				0usize,
				LANDLOCK_CREATE_RULESET_VERSION,
			)
		}
	}

	fn abi() -> Result<ABI, String> {
		Ok(match kernel_abi() {
			n if n <= 0 => {
				return Err(
					"this kernel has no Landlock (Linux 5.13 or later, with Landlock enabled at boot, is needed)"
						.into(),
				);
			}
			1 => ABI::V1,
			2 => ABI::V2,
			3 => ABI::V3,
			4 => ABI::V4,
			5 => ABI::V5,
			6 => ABI::V6,
			7 => ABI::V7,
			8 => ABI::V8,
			_ => ABI::V9,
		})
	}

	pub fn available() -> Result<(), String> {
		abi().map(|_| ())
	}

	pub fn command(program: &Path, c: &Confinement) -> Result<std::process::Command, String> {
		use std::os::unix::process::CommandExt;
		let abi = abi()?;
		let ruleset = landlock_ruleset(abi, c).map_err(|e| format!("Landlock: {e}"))?;
		// Signals and abstract sockets are Landlock's from ABI 6; before it, seccomp's.
		let filters = filters(c, abi < ABI::V6, abi < ABI::V3)?;
		let mut cmd = std::process::Command::new(program);
		let mut ruleset = Some(ruleset);
		// SAFETY: between fork and exec this only makes the syscalls that apply a prepared
		// ruleset and prepared filters; nothing is allocated unless one fails.
		unsafe {
			cmd.pre_exec(move || {
				if let Some(ruleset) = ruleset.take() {
					let status = ruleset.restrict_self().map_err(std::io::Error::other)?;
					if status.ruleset != RulesetStatus::FullyEnforced {
						return Err(std::io::Error::other("Landlock was not fully enforced"));
					}
				}
				for filter in &filters {
					seccompiler::apply_filter(filter).map_err(std::io::Error::other)?;
				}
				Ok(())
			});
		}
		Ok(cmd)
	}

	fn landlock_ruleset(
		abi: ABI,
		c: &Confinement,
	) -> Result<landlock::RulesetCreated, landlock::RulesetError> {
		let all = AccessFs::from_all(abi);
		let read = AccessFs::ReadFile | AccessFs::ReadDir;
		// Writing never makes devices, pipes or sockets in the filesystem, and never runs
		// what it wrote.
		let write = all
			& !(AccessFs::Execute
				| AccessFs::MakeChar
				| AccessFs::MakeBlock
				| AccessFs::MakeFifo
				| AccessFs::MakeSock);
		let readable = if c.spawn {
			read | AccessFs::Execute
		} else {
			read
		};
		let mut ruleset = Ruleset::default()
			.set_compatibility(CompatLevel::HardRequirement)
			.handle_access(all)?;
		if abi >= ABI::V4 {
			ruleset = ruleset.handle_access(AccessNet::from_all(abi))?;
		}
		if abi >= ABI::V6 {
			ruleset = ruleset.scope(Scope::from_all(abi))?;
		}
		let rule = |path: &PathBuf, access: landlock::BitFlags<AccessFs>| {
			// A path that does not exist grants nothing.
			PathFd::new(path)
				.ok()
				.map(|fd| PathBeneath::new(fd, access & AccessFs::from_file(abi)))
		};
		let dir_rule = |path: &PathBuf, access: landlock::BitFlags<AccessFs>| {
			PathFd::new(path).ok().map(|fd| {
				let access = if path.is_dir() {
					access
				} else {
					access & AccessFs::from_file(abi)
				};
				PathBeneath::new(fd, access)
			})
		};
		let devices = [
			("/dev/null", read | AccessFs::WriteFile),
			("/dev/zero", read),
			("/dev/urandom", read),
			("/dev/random", read),
		]
		.into_iter()
		.filter_map(|(path, access)| rule(&PathBuf::from(path), access));
		// `no_new_privs` is set with the ruleset: a setuid program cannot shed it.
		ruleset
			.create()?
			.add_rules(
				c.read
					.iter()
					.filter_map(|p| dir_rule(p, readable))
					.map(Ok::<_, landlock::RulesetError>),
			)?
			.add_rules(
				c.write
					.iter()
					.filter_map(|p| dir_rule(p, write))
					.map(Ok::<_, landlock::RulesetError>),
			)?
			.add_rules(
				c.exec
					.iter()
					.filter_map(|p| rule(p, read | AccessFs::Execute))
					.map(Ok::<_, landlock::RulesetError>),
			)?
			.add_rules(devices.map(Ok::<_, landlock::RulesetError>))
	}

	fn arch() -> Result<TargetArch, String> {
		if cfg!(target_arch = "x86_64") {
			Ok(TargetArch::x86_64)
		} else if cfg!(target_arch = "aarch64") {
			Ok(TargetArch::aarch64)
		} else if cfg!(target_arch = "riscv64") {
			Ok(TargetArch::riscv64)
		} else {
			Err("seccomp filters are not built for this architecture".into())
		}
	}

	fn condition(arg: u8, op: SeccompCmpOp, value: u64) -> Result<SeccompRule, String> {
		SeccompCondition::new(arg, SeccompCmpArgLen::Qword, op, value)
			.and_then(|c| SeccompRule::new(vec![c]))
			.map_err(|e| e.to_string())
	}

	/// The seccomp filters for `c`. Landlock governs paths; these govern what it cannot:
	/// sockets other than a pair the process makes for itself, connecting or binding
	/// anything, sending to an address, starting processes, tracing, changing another
	/// process's limits or scheduling, changing any file's mode, owner or extended
	/// attributes, and the kernel's other ways out. `signals` adds the kill family, when
	/// Landlock cannot scope signals; `truncate` adds truncating by path, before Landlock
	/// governs it (ABI 3).
	fn filters(c: &Confinement, signals: bool, truncate: bool) -> Result<Vec<BpfProgram>, String> {
		let mut denied: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
		for call in [
			libc::SYS_connect,
			libc::SYS_bind,
			libc::SYS_listen,
			libc::SYS_accept,
			libc::SYS_accept4,
			libc::SYS_sendmsg,
			libc::SYS_sendmmsg,
			libc::SYS_ptrace,
			libc::SYS_process_vm_readv,
			libc::SYS_process_vm_writev,
			libc::SYS_mount,
			libc::SYS_umount2,
			libc::SYS_pivot_root,
			libc::SYS_chroot,
			libc::SYS_unshare,
			libc::SYS_setns,
			libc::SYS_bpf,
			libc::SYS_perf_event_open,
			libc::SYS_userfaultfd,
			libc::SYS_keyctl,
			libc::SYS_add_key,
			libc::SYS_request_key,
			libc::SYS_io_uring_setup,
			libc::SYS_io_uring_enter,
			libc::SYS_io_uring_register,
			libc::SYS_kexec_load,
			libc::SYS_init_module,
			libc::SYS_finit_module,
			libc::SYS_delete_module,
			libc::SYS_pidfd_getfd,
			libc::SYS_open_by_handle_at,
			libc::SYS_name_to_handle_at,
		] {
			denied.insert(call, Vec::new());
		}
		// Only the local pairs `socketpair` makes, and `socket(AF_UNIX)` for asyncio's.
		denied.insert(
			libc::SYS_socket,
			vec![condition(0, SeccompCmpOp::Ne, libc::AF_UNIX as u64)?],
		);
		denied.insert(libc::SYS_sendto, vec![condition(4, SeccompCmpOp::Ne, 0)?]);
		if !c.spawn {
			// A thread shares its process; anything else is a new process.
			denied.insert(
				libc::SYS_clone,
				vec![condition(
					0,
					SeccompCmpOp::MaskedEq(libc::CLONE_THREAD as u64),
					0,
				)?],
			);
			#[cfg(target_arch = "x86_64")]
			for call in [libc::SYS_fork, libc::SYS_vfork] {
				denied.insert(call, Vec::new());
			}
		}
		// Its own limits, priority, scheduling and memory stay its own — pid 0, or the
		// calling process by kind and id — and another process's are not. Rules for one call
		// refuse it when any of them matches.
		const PRIO_PROCESS: u64 = 0;
		const IOPRIO_WHO_PROCESS: u64 = 1;
		for (call, rules) in [
			(libc::SYS_prlimit64, vec![(0, 0)]),
			(libc::SYS_setpriority, vec![(0, PRIO_PROCESS), (1, 0)]),
			(libc::SYS_ioprio_set, vec![(0, IOPRIO_WHO_PROCESS), (1, 0)]),
			(libc::SYS_sched_setaffinity, vec![(0, 0)]),
			(libc::SYS_sched_setscheduler, vec![(0, 0)]),
			(libc::SYS_sched_setparam, vec![(0, 0)]),
			(libc::SYS_sched_setattr, vec![(0, 0)]),
			(libc::SYS_migrate_pages, vec![(0, 0)]),
			(libc::SYS_move_pages, vec![(0, 0)]),
		] {
			let rules = rules
				.into_iter()
				.map(|(arg, own)| condition(arg, SeccompCmpOp::Ne, own))
				.collect::<Result<Vec<_>, _>>()?;
			denied.insert(call, rules);
		}
		// Landlock has no right for mode, owner or extended attributes: a student could
		// otherwise lock another file of the user's away, even one it cannot read.
		const SYS_FCHMODAT2: i64 = 452;
		for call in [
			libc::SYS_fchmod,
			libc::SYS_fchmodat,
			SYS_FCHMODAT2,
			libc::SYS_fchown,
			libc::SYS_fchownat,
			libc::SYS_setxattr,
			libc::SYS_lsetxattr,
			libc::SYS_fsetxattr,
			libc::SYS_removexattr,
			libc::SYS_lremovexattr,
			libc::SYS_fremovexattr,
		] {
			denied.insert(call, Vec::new());
		}
		#[cfg(target_arch = "x86_64")]
		for call in [libc::SYS_chmod, libc::SYS_chown, libc::SYS_lchown] {
			denied.insert(call, Vec::new());
		}
		if truncate {
			denied.insert(libc::SYS_truncate, Vec::new());
		}
		if signals {
			for call in [
				libc::SYS_kill,
				libc::SYS_tkill,
				libc::SYS_tgkill,
				libc::SYS_rt_sigqueueinfo,
				libc::SYS_rt_tgsigqueueinfo,
				libc::SYS_pidfd_send_signal,
			] {
				denied.insert(call, Vec::new());
			}
		}
		let arch = arch()?;
		let build = |rules, errno: i32| -> Result<BpfProgram, String> {
			SeccompFilter::new(
				rules,
				SeccompAction::Allow,
				SeccompAction::Errno(errno as u32),
				arch,
			)
			.and_then(BpfProgram::try_from)
			.map_err(|e| e.to_string())
		};
		// `clone3` takes its flags by pointer, where a filter cannot look: refuse it as
		// unknown, and the C library falls back to `clone`.
		let clone3 = BTreeMap::from([(libc::SYS_clone3, Vec::new())]);
		let filters = vec![
			build(denied, libc::EPERM)?,
			build(clone3, libc::ENOSYS)?,
			unknown_calls(),
		];
		Ok(filters)
	}

	/// A list of what to refuse lets through whatever the kernel adds after it was written
	/// — `setxattrat` and `removexattrat` came in 6.13. Every syscall numbered from the
	/// first this list does not know is refused as unknown (`ENOSYS`), which the C library
	/// and Python treat as an older kernel. They share one numbering on every architecture;
	/// x32's numbering on x86-64, which starts at `0x40000000`, is refused with them.
	fn unknown_calls() -> BpfProgram {
		use seccompiler::sock_filter;
		const FIRST_UNKNOWN: u32 = 463; // setxattrat
		const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
		const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
		const LOAD_NR: u16 = 0x20; // BPF_LD | BPF_W | BPF_ABS, at seccomp_data.nr
		const JUMP_GE: u16 = 0x35; // BPF_JMP | BPF_JGE | BPF_K
		const RETURN: u16 = 0x06; // BPF_RET | BPF_K
		let op = |code, jt, jf, k| sock_filter { code, jt, jf, k };
		vec![
			op(LOAD_NR, 0, 0, 0),
			op(JUMP_GE, 0, 1, FIRST_UNKNOWN),
			op(RETURN, 0, 0, SECCOMP_RET_ERRNO | libc::ENOSYS as u32),
			op(RETURN, 0, 0, SECCOMP_RET_ALLOW),
		]
	}
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
	use super::*;

	pub fn available() -> Result<(), String> {
		Err("ScriptMark cannot sandbox student code on this platform yet".into())
	}

	pub fn command(_program: &Path, _c: &Confinement) -> Result<std::process::Command, String> {
		Err(available().unwrap_err())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_default_config() {
		let config = SandboxConfig::default();
		assert_eq!(config.cpu_secs, 30);
		assert_eq!(config.mem_mb, 512);
		assert_eq!(config.fsize_mb, 10);
		assert_eq!(config.nofile, 64);
		assert_eq!(config.nproc, Some(64));
	}

	#[cfg(unix)]
	#[tokio::test]
	async fn test_sandbox_applies_to_command() {
		let config = SandboxConfig::default();
		let mut cmd = tokio::process::Command::new("echo");
		cmd.arg("hello");
		apply_sandbox(&mut cmd, &config);
		let output = cmd.output().await.unwrap();
		assert!(output.status.success());
		assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
	}

	#[test]
	fn test_covering_keeps_only_the_outermost_paths() {
		let paths = ["/a/b", "/a", "/c", "/a/b/c", "/ab"]
			.map(PathBuf::from)
			.to_vec();
		assert_eq!(
			covering(paths),
			["/a", "/ab", "/c"].map(PathBuf::from).to_vec()
		);
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn test_a_profile_quotes_paths_and_grants_only_what_it_lists() {
		let c = Confinement {
			read: vec![PathBuf::from("/opt/py")],
			write: vec![PathBuf::from("/tmp/unit \"x\"/work")],
			exec: vec![PathBuf::from("/opt/py/bin")],
			spawn: false,
			no_ffi: true,
		};
		let profile = platform::profile(&c).unwrap();
		assert!(profile.contains("(deny default)"));
		assert!(
			profile.contains(r#"(subpath "/tmp/unit \"x\"/work")"#),
			"{profile}"
		);
		assert!(
			profile.contains(r#"(literal "/tmp/unit \"x\"")"#),
			"{profile}"
		);
		assert!(profile.contains("_ctypes"));
		assert!(!profile.contains("process-fork"));
	}
}
