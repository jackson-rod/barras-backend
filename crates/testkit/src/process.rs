//! Starting a workspace binary from its tests: with a clean environment, stopped with
//! SIGTERM, with everything it printed.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The variable through which cargo-llvm-cov tells an instrumented binary where to write its
/// coverage profile.
const PROFILE_FILE: &str = "LLVM_PROFILE_FILE";

/// Short, so that a wait ends soon after the binary is ready or has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// On loopback a connect succeeds or is refused at once; only a listener whose backlog is
/// full leaves it waiting.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);

/// `127.0.0.1` with a port the OS has just handed out, which nothing listens on any more.
/// Another process may take the port before the binary binds it: a test that starts a binary
/// on it starts over with a new address when the binary exits before it listens.
pub fn free_local_addr() -> SocketAddr {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap_or_else(|error| panic!("cannot bind 127.0.0.1:0: {error}"));
    listener.local_addr().unwrap()
}

/// A binary to start from a test. Its environment holds only the variables given with
/// [`env`](Self::env), plus `LLVM_PROFILE_FILE` when the tests run under cargo-llvm-cov, so
/// that the binary's coverage joins theirs. Its standard input is empty, and what it prints
/// is captured. A step that fails panics, and so fails the test that took it.
#[derive(Debug)]
pub struct BinaryCommand {
    program: PathBuf,
    args: Vec<OsString>,
    variables: BTreeMap<OsString, OsString>,
}

impl BinaryCommand {
    /// The binary at `program`, with no argument and an empty environment.
    pub fn new(program: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            variables: BTreeMap::new(),
        }
    }

    /// Adds an argument.
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Adds a variable to the environment. The last value given for a name wins.
    pub fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.variables
            .insert(name.as_ref().to_owned(), value.as_ref().to_owned());
        self
    }

    /// Runs the binary to its end and returns its exit status and everything it printed. Like
    /// [`Command::output`], it waits as long as the binary runs.
    pub fn run(self) -> Output {
        let mut running = self.spawn();
        let status = running.child.wait().unwrap_or_else(|error| {
            panic!("cannot wait for process {}: {error}", running.child.id())
        });
        running.collect_output(status)
    }

    /// Starts the binary and returns at once. Its output is read from that moment on, so a
    /// binary that prints more than a pipe holds never blocks.
    pub fn spawn(self) -> RunningBinary {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .env_clear()
            .envs(child_environment(self.variables, parent_profile_file()))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("cannot start {}: {error}", self.program.display()));
        let stdout = child.stdout.take().expect("standard output is piped");
        let stderr = child.stderr.take().expect("standard error is piped");
        RunningBinary {
            child,
            stdout: Some(read_in_background(stdout)),
            stderr: Some(read_in_background(stderr)),
        }
    }
}

/// A binary started by [`BinaryCommand::spawn`]. Dropping it kills the process and reaps it,
/// also while a failed test unwinds, so no test leaves a process behind.
#[derive(Debug)]
#[must_use = "dropping it kills the binary"]
pub struct RunningBinary {
    child: Child,
    // In an `Option` only so that `collect_output` can take them: a type with `Drop` cannot
    // be taken apart.
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl RunningBinary {
    /// Waits until `addr` accepts a connection, and returns `true`. Returns `false` as soon as
    /// the binary has exited, or once `timeout` has passed. Each try opens a connection and
    /// closes it without sending anything.
    pub fn wait_until_listening(&mut self, addr: SocketAddr, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            // Before the connect: when another process took the port, that process is the one
            // that answers, and the binary has exited.
            if self.exit_status().is_some() {
                return false;
            }
            if TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Sends SIGTERM, waits up to `timeout` for the binary to exit, kills it if it has not,
    /// and returns its exit status and everything it printed. A binary that has already
    /// exited gets no signal.
    pub fn terminate(mut self, timeout: Duration) -> Output {
        // Once the binary is reaped, its pid may belong to another process.
        if self.exit_status().is_none() {
            self.send_sigterm();
        }
        let status = self
            .wait_for_exit(timeout)
            .unwrap_or_else(|| self.kill_and_reap());
        self.collect_output(status)
    }

    /// The exit status once the binary has exited. Looking reaps it.
    fn exit_status(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().unwrap_or_else(|error| {
            panic!(
                "cannot check whether process {} exited: {error}",
                self.child.id()
            )
        })
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            let status = self.exit_status();
            if status.is_some() || Instant::now() >= deadline {
                return status;
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Through the `kill` program: no `unsafe`, and no dependency for one signal.
    fn send_sigterm(&self) {
        let pid = self.child.id().to_string();
        let kill = Command::new("kill")
            .args(["-s", "TERM", &pid])
            .output()
            .unwrap_or_else(|error| panic!("cannot run kill: {error}"));
        assert!(
            kill.status.success(),
            "kill -s TERM {pid}: {}",
            String::from_utf8_lossy(&kill.stderr)
        );
    }

    fn kill_and_reap(&mut self) -> ExitStatus {
        let pid = self.child.id();
        self.child
            .kill()
            .unwrap_or_else(|error| panic!("cannot kill process {pid}: {error}"));
        self.child
            .wait()
            .unwrap_or_else(|error| panic!("cannot wait for process {pid}: {error}"))
    }

    fn collect_output(&mut self, status: ExitStatus) -> Output {
        Output {
            status,
            stdout: join_reader(self.stdout.take()),
            stderr: join_reader(self.stderr.take()),
        }
    }
}

impl Drop for RunningBinary {
    fn drop(&mut self) {
        // Once the binary has been reaped, `kill` sends nothing, so it cannot reach a process
        // that took the pid since, and `wait` returns the status it kept.
        let reaped = self.child.kill().and_then(|()| self.child.wait());
        // While a failed test unwinds, a second panic would abort the whole test process.
        if let Err(error) = reaped
            && !thread::panicking()
        {
            panic!("cannot kill process {}: {error}", self.child.id());
        }
    }
}

/// Reads `pipe` to its end on a thread of its own: a binary that fills a pipe nobody reads
/// stops until someone does.
fn read_in_background(mut pipe: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)
            .unwrap_or_else(|error| panic!("cannot read what the binary printed: {error}"));
        bytes
    })
}

/// What the reader read. A panic of its thread carries on in the caller's.
fn join_reader(reader: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .expect("the output is collected once")
        .join()
        .unwrap_or_else(|payload| panic::resume_unwind(payload))
}

/// The environment a binary starts with: the variables given, plus the parent's
/// `LLVM_PROFILE_FILE` unless one was given. Without it an instrumented binary is measured as
/// never run, and writes a stray `default.profraw` into its working folder.
fn child_environment(
    mut given: BTreeMap<OsString, OsString>,
    parent_profile_file: Option<OsString>,
) -> BTreeMap<OsString, OsString> {
    if let Some(profile_file) = parent_profile_file {
        given
            .entry(OsString::from(PROFILE_FILE))
            .or_insert(profile_file);
    }
    given
}

#[allow(clippy::disallowed_methods)] // test support, and the only variable it reads: where cargo-llvm-cov wants the coverage written
fn parent_profile_file() -> Option<OsString> {
    std::env::var_os(PROFILE_FILE)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::IpAddr;
    use std::os::unix::process::ExitStatusExt;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    use super::*;

    /// 1 MiB on each stream, far more than a pipe holds: the shell blocks on whichever
    /// stream nobody reads.
    const LARGE_OUTPUT: &str = r#"line=$(printf '%01023d' 0); i=0; while [ "$i" -lt 1024 ]; do echo "$line"; echo "$line" >&2; i=$((i + 1)); done"#;
    const MIB: usize = 1024 * 1024;
    const SIGKILL: i32 = 9;

    fn text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).unwrap()
    }

    fn os_string(text: &str) -> OsString {
        OsString::from(text)
    }

    /// Whether a process with this id exists, a zombie included.
    fn is_alive(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// Runs `work` on a thread of its own, and fails the test when it is still running after
    /// ten seconds: a deadlock then fails the test instead of hanging the run.
    fn within_ten_seconds<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || sender.send(work()));
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("still running after ten seconds")
    }

    /// A file the shell creates once its trap is set: a SIGTERM sent before that would end
    /// the shell instead of running the trap.
    struct ReadyFile(PathBuf);

    impl ReadyFile {
        fn new(test: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("barras-testkit-{}-{test}", std::process::id()));
            // An earlier run whose process had the same id may have left it behind.
            let _ = fs::remove_file(&path);
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn wait(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !self.0.exists() {
                assert!(
                    Instant::now() < deadline,
                    "the shell was not ready after ten seconds"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    impl Drop for ReadyFile {
        fn drop(&mut self) {
            // Absent when the shell never got that far.
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn the_parents_profile_file_joins_the_variables_given() {
        let given = BTreeMap::from([(os_string("APP_ENV"), os_string("local"))]);

        let environment = child_environment(given, Some(os_string("/cover/barras-%p-%m.profraw")));

        assert_eq!(
            environment,
            BTreeMap::from([
                (os_string("APP_ENV"), os_string("local")),
                (
                    os_string("LLVM_PROFILE_FILE"),
                    os_string("/cover/barras-%p-%m.profraw")
                ),
            ])
        );
    }

    #[test]
    fn without_a_profile_file_in_the_parent_the_binary_gets_only_the_variables_given() {
        let given = BTreeMap::from([(os_string("APP_ENV"), os_string("local"))]);

        assert_eq!(child_environment(given.clone(), None), given);
    }

    #[test]
    fn a_profile_file_given_to_the_binary_wins_over_the_parents() {
        let given = BTreeMap::from([(os_string("LLVM_PROFILE_FILE"), os_string("/mine.profraw"))]);

        let environment = child_environment(given.clone(), Some(os_string("/cover/%p.profraw")));

        assert_eq!(environment, given);
    }

    #[test]
    fn the_binary_sees_only_the_variables_it_was_given() {
        // `env` itself: a shell would add PWD, SHLVL and `_` of its own.
        let output = BinaryCommand::new("/usr/bin/env")
            .env("FIRST", "1")
            .env("SECOND", "two words")
            .run();

        let mut expected = vec!["FIRST=1".to_owned(), "SECOND=two words".to_owned()];
        // Under cargo-llvm-cov the tests have it, so the binary has it too.
        if let Some(profile_file) = parent_profile_file() {
            expected.push(format!("{PROFILE_FILE}={}", profile_file.to_str().unwrap()));
        }
        expected.sort();
        let mut printed: Vec<&str> = text(&output.stdout).lines().collect();
        printed.sort_unstable();
        assert_eq!(printed, expected);
    }

    #[test]
    fn run_returns_the_exit_status_and_what_the_binary_printed() {
        let output = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("echo out; echo err >&2; exit 3")
            .run();

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(text(&output.stdout), "out\n");
        assert_eq!(text(&output.stderr), "err\n");
    }

    #[test]
    fn run_does_not_block_a_binary_that_prints_more_than_a_pipe_holds() {
        let command = BinaryCommand::new("/bin/sh").arg("-c").arg(LARGE_OUTPUT);

        let output = within_ten_seconds(move || command.run());

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), MIB);
        assert_eq!(output.stderr.len(), MIB);
    }

    #[test]
    fn terminate_delivers_sigterm() {
        let ready = ReadyFile::new("terminate-delivers-sigterm");
        let running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(r#"trap 'echo got-term; exit 0' TERM; : > "$READY"; while :; do /bin/sleep 0.1; done"#)
            .env("READY", ready.path())
            .spawn();
        ready.wait();

        let output = running.terminate(Duration::from_secs(10));

        assert_eq!(output.status.code(), Some(0));
        assert_eq!(text(&output.stdout), "got-term\n");
    }

    #[test]
    fn a_binary_that_ignores_sigterm_is_killed_when_the_timeout_ends() {
        let ready = ReadyFile::new("ignores-sigterm");
        // `sleep` keeps the ignored disposition across `exec`.
        let running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(r#"trap '' TERM; : > "$READY"; exec /bin/sleep 60"#)
            .env("READY", ready.path())
            .spawn();
        ready.wait();
        let timeout = Duration::from_millis(300);

        let started = Instant::now();
        let output = running.terminate(timeout);

        assert!(started.elapsed() >= timeout);
        assert_eq!(output.status.signal(), Some(SIGKILL));
    }

    #[test]
    fn terminate_returns_the_status_and_output_of_a_binary_that_already_exited() {
        let mut running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("echo out; echo err >&2; exit 3")
            .spawn();
        assert!(running.wait_for_exit(Duration::from_secs(10)).is_some());

        let output = running.terminate(Duration::from_secs(10));

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(text(&output.stdout), "out\n");
        assert_eq!(text(&output.stderr), "err\n");
    }

    #[test]
    fn terminate_returns_a_large_output_read_while_the_binary_ran() {
        let ready = ReadyFile::new("large-output");
        // Never ready unless both streams are read while it runs.
        let running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                r#"trap 'exit 0' TERM; {LARGE_OUTPUT}; : > "$READY"; while :; do /bin/sleep 0.1; done"#
            ))
            .env("READY", ready.path())
            .spawn();
        ready.wait();

        let output = running.terminate(Duration::from_secs(10));

        assert_eq!(output.status.code(), Some(0));
        assert_eq!(output.stdout.len(), MIB);
        assert_eq!(output.stderr.len(), MIB);
    }

    #[test]
    fn a_dropped_binary_leaves_no_process_behind() {
        let running = BinaryCommand::new("/bin/sleep").arg("60").spawn();
        let pid = running.child.id();
        assert!(is_alive(pid));

        let started = Instant::now();
        drop(running);

        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the drop waited for the process instead of killing it"
        );
        assert!(!is_alive(pid));
    }

    #[test]
    fn a_binary_dropped_while_a_test_panics_leaves_no_process_behind() {
        let pid = AtomicU32::new(0);

        let outcome = panic::catch_unwind(|| {
            let running = BinaryCommand::new("/bin/sleep").arg("60").spawn();
            pid.store(running.child.id(), Ordering::SeqCst);
            panic!("the test fails while the binary runs");
        });

        assert!(outcome.is_err());
        assert!(!is_alive(pid.load(Ordering::SeqCst)));
    }

    #[test]
    fn wait_until_listening_is_true_once_the_address_accepts_connections() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut running = BinaryCommand::new("/bin/sleep").arg("60").spawn();

        let listening =
            running.wait_until_listening(listener.local_addr().unwrap(), Duration::from_secs(10));

        assert!(listening);
    }

    #[test]
    fn wait_until_listening_is_false_at_once_for_a_binary_that_has_exited() {
        // Another process listens on the address: the binary exited because it had the port.
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let mut running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("exit 1")
            .spawn();
        assert!(running.wait_for_exit(Duration::from_secs(10)).is_some());

        let started = Instant::now();
        let listening =
            running.wait_until_listening(taken.local_addr().unwrap(), Duration::from_secs(60));

        assert!(!listening);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn wait_until_listening_is_false_as_soon_as_the_binary_exits() {
        let mut running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("/bin/sleep 0.2; exit 1")
            .spawn();

        let started = Instant::now();
        let listening = running.wait_until_listening(free_local_addr(), Duration::from_secs(60));

        assert!(!listening);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn wait_until_listening_is_false_when_the_timeout_ends() {
        let mut running = BinaryCommand::new("/bin/sleep").arg("60").spawn();
        let timeout = Duration::from_millis(200);

        let started = Instant::now();
        let listening = running.wait_until_listening(free_local_addr(), timeout);

        assert!(!listening);
        assert!(started.elapsed() >= timeout);
    }

    #[test]
    fn free_local_addr_is_a_loopback_address_that_nothing_listens_on() {
        let addr = free_local_addr();

        assert_eq!(addr.ip(), IpAddr::from(Ipv4Addr::LOCALHOST));
        assert_ne!(addr.port(), 0);
        assert!(TcpListener::bind(addr).is_ok());
    }
}
