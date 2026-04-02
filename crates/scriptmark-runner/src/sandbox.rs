/// Resource limits for student code execution.
///
/// Applied via `setrlimit` in a `pre_exec` hook (between fork and exec),
/// so limits are kernel-enforced on the child process only.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
	/// CPU time limit in seconds (RLIMIT_CPU). Default: 30.
	pub cpu_secs: u64,
	/// Virtual memory limit in MB (RLIMIT_AS). Default: 512.
	pub mem_mb: u64,
	/// Max file write size in MB (RLIMIT_FSIZE). Default: 10.
	pub fsize_mb: u64,
	/// Max open file descriptors (RLIMIT_NOFILE). Default: 64.
	pub nofile: u64,
	/// Max processes for this UID (RLIMIT_NPROC). Default: 64.
	/// Note: on macOS this is per-UID, not per-process.
	pub nproc: u64,
}

impl Default for SandboxConfig {
	fn default() -> Self {
		Self {
			cpu_secs: 30,
			mem_mb: 512,
			fsize_mb: 10,
			nofile: 64,
			nproc: 64,
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
			let _ = set_rlimit(libc::RLIMIT_CPU, cpu);
			let _ = set_rlimit(libc::RLIMIT_FSIZE, fsize);
			let _ = set_rlimit(libc::RLIMIT_NOFILE, nofile);
			let _ = set_rlimit(libc::RLIMIT_NPROC, nproc);
			// RLIMIT_AS: skip on macOS where it's unreliable
			#[cfg(not(target_os = "macos"))]
			let _ = set_rlimit(libc::RLIMIT_AS, mem);
			Ok(())
		});
	}
}

#[cfg(unix)]
fn set_rlimit(resource: libc::c_int, limit: u64) -> std::io::Result<()> {
	let rlim = libc::rlimit {
		rlim_cur: limit as libc::rlim_t,
		rlim_max: limit as libc::rlim_t,
	};
	let ret = unsafe { libc::setrlimit(resource, &rlim) };
	if ret != 0 {
		return Err(std::io::Error::last_os_error());
	}
	Ok(())
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
		assert_eq!(config.nproc, 64);
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
}
