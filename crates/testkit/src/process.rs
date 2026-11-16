//! Starting a workspace binary from its tests: with a clean environment, stopped with
//! SIGTERM, with everything it printed, and never waited for without a deadline.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::panic;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The variable through which cargo-llvm-cov tells an instrumented binary where to write its
/// coverage profile.
const PROFILE_FILE: &str = "LLVM_PROFILE_FILE";

/// Short, so that a wait ends soon after the binary has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// A binary to start from a test. Its environment holds only the variables given with
/// [`env`](Self::env), plus `LLVM_PROFILE_FILE` when the tests run under cargo-llvm-cov, so
/// that the binary's coverage joins theirs. Its standard input is empty, and what it prints
/// is captured unless sent elsewhere. A step that fails panics, and so fails the test that
/// took it.
#[derive(Debug)]
pub struct BinaryCommand {
    program: PathBuf,
    args: Vec<OsString>,
    variables: BTreeMap<OsString, OsString>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
}

impl BinaryCommand {
    /// The binary at `program`, with no argument and an empty environment.
    pub fn new(program: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            variables: BTreeMap::new(),
            stdout: None,
            stderr: None,
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

    /// Sends standard output to `destination` instead of capturing it: a pipe nobody reads, for
    /// a test of a binary that cannot write its output, say.
    pub fn stdout(mut self, destination: Stdio) -> Self {
        self.stdout = Some(destination);
        self
    }

    /// Sends standard error to `destination` instead of capturing it.
    pub fn stderr(mut self, destination: Stdio) -> Self {
        self.stderr = Some(destination);
        self
    }

    /// Runs the binary to its end and returns its exit status and everything it printed. A
    /// binary still running once `timeout` has passed is killed, and the test fails with what it
    /// printed: a binary that should have stopped, and serves instead, fails its test rather
    /// than hang it.
    pub fn run_within(self, timeout: Duration) -> Output {
        let mut running = self.spawn();
        let exited = running.wait_for_exit(timeout);
        let status = exited.unwrap_or_else(|| running.kill_and_reap());
        let output = running.collect_output(status);
        assert!(
            exited.is_some(),
            "the binary was still running after {timeout:?}, so it was killed: {}",
            describe(&output)
        );
        output
    }

    /// Starts the binary and returns at once. Its output is read from that moment on, so a
    /// binary that prints more than a pipe holds never blocks.
    pub fn spawn(self) -> RunningBinary {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .env_clear()
            .envs(child_environment(self.variables, parent_profile_file()))
            .stdin(Stdio::null())
            .stdout(self.stdout.unwrap_or_else(Stdio::piped))
            .stderr(self.stderr.unwrap_or_else(Stdio::piped))
            .spawn()
            .unwrap_or_else(|error| panic!("cannot start {}: {error}", self.program.display()));
        RunningBinary {
            stdout: Captured::start(child.stdout.take()),
            stderr: Captured::start(child.stderr.take()),
            child,
        }
    }
}

/// A binary started by [`BinaryCommand::spawn`]. Dropping it kills the process and reaps it,
/// also while a failed test unwinds, so no test leaves a process behind.
#[derive(Debug)]
#[must_use = "dropping it kills the binary"]
pub struct RunningBinary {
    child: Child,
    stdout: Captured,
    stderr: Captured,
}

impl RunningBinary {
    /// The first whole line of standard output, one that ends with a newline, for which
    /// `matches` holds, whether it was printed before the call or is printed during it. `None`
    /// as soon as the binary has closed its output without printing one, or once `timeout` has
    /// passed. The lines stay in the output that [`terminate`](Self::terminate) returns.
    pub fn wait_for_line(
        &self,
        matches: impl Fn(&str) -> bool,
        timeout: Duration,
    ) -> Option<String> {
        let deadline = Instant::now() + timeout;
        let mut stream = self.stdout.stream();
        let mut looked_at = 0;
        loop {
            while let Some(length) = stream.bytes[looked_at..]
                .iter()
                .position(|&byte| byte == b'\n')
            {
                let line = &stream.bytes[looked_at..looked_at + length];
                let line = String::from_utf8_lossy(line).into_owned();
                looked_at += length + 1;
                if matches(&line) {
                    return Some(line);
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if stream.ended || left.is_zero() {
                return None;
            }
            stream = self.stdout.wait_for_more(stream, left);
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
            stdout: self.stdout.take_all(),
            stderr: self.stderr.take_all(),
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

/// One stream of the binary, read to its end on a thread of its own, since a binary that fills
/// a pipe nobody reads stops until someone does. What was read is shared, so that a test can
/// look at it while the binary runs.
#[derive(Debug)]
struct Captured {
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
}

#[derive(Debug, Default)]
struct Shared {
    stream: Mutex<Stream>,
    /// Signalled whenever bytes arrive, and when the stream ends.
    grew: Condvar,
}

#[derive(Debug, Default)]
struct Stream {
    bytes: Vec<u8>,
    ended: bool,
}

impl Captured {
    /// Reads `pipe` from now on; with no pipe, the stream went elsewhere and is empty.
    fn start(pipe: Option<impl Read + Send + 'static>) -> Self {
        let shared = Arc::new(Shared::default());
        let reader = match pipe {
            Some(pipe) => {
                let shared = Arc::clone(&shared);
                Some(thread::spawn(move || read_to_end(pipe, &shared)))
            }
            None => {
                lock(&shared.stream).ended = true;
                None
            }
        };
        Self { shared, reader }
    }

    fn stream(&self) -> MutexGuard<'_, Stream> {
        lock(&self.shared.stream)
    }

    /// Waits until more bytes arrive, the stream ends, or `timeout` passes.
    fn wait_for_more<'a>(
        &'a self,
        stream: MutexGuard<'a, Stream>,
        timeout: Duration,
    ) -> MutexGuard<'a, Stream> {
        let (stream, _) = self
            .shared
            .grew
            .wait_timeout(stream, timeout)
            .unwrap_or_else(PoisonError::into_inner);
        stream
    }

    /// Everything read, once the stream has ended. A panic of the reader carries on in the
    /// caller's.
    fn take_all(&mut self) -> Vec<u8> {
        if let Some(reader) = self.reader.take() {
            reader
                .join()
                .unwrap_or_else(|payload| panic::resume_unwind(payload));
        }
        std::mem::take(&mut self.stream().bytes)
    }
}

fn read_to_end(mut pipe: impl Read, shared: &Shared) {
    let mut chunk = [0; 8192];
    let failure = loop {
        match pipe.read(&mut chunk) {
            Ok(0) => break None,
            Ok(count) => {
                lock(&shared.stream)
                    .bytes
                    .extend_from_slice(&chunk[..count]);
                shared.grew.notify_all();
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Some(error),
        }
    };
    // Ended either way, so that nobody waits for bytes that cannot come.
    lock(&shared.stream).ended = true;
    shared.grew.notify_all();
    if let Some(error) = failure {
        panic!("cannot read what the binary printed: {error}");
    }
}

/// A lock whose holder panicked still guards bytes that are whole: each change is one append
/// or one flag.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The exit status and what the binary printed, for the message of a failed test.
fn describe(output: &Output) -> String {
    format!(
        "{}\nstandard output:\n{}\nstandard error:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
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
    use std::os::unix::process::ExitStatusExt;
    use std::sync::atomic::{AtomicU32, Ordering};

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
            .run_within(Duration::from_secs(10));

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
    fn run_within_returns_the_exit_status_and_what_the_binary_printed() {
        let output = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("echo out; echo err >&2; exit 3")
            .run_within(Duration::from_secs(10));

        assert_eq!(output.status.code(), Some(3));
        assert_eq!(text(&output.stdout), "out\n");
        assert_eq!(text(&output.stderr), "err\n");
    }

    #[test]
    fn run_within_does_not_block_a_binary_that_prints_more_than_a_pipe_holds() {
        let output = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(LARGE_OUTPUT)
            .run_within(Duration::from_secs(10));

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), MIB);
        assert_eq!(output.stderr.len(), MIB);
    }

    #[test]
    fn run_within_kills_a_binary_still_running_at_the_deadline_and_fails_with_its_output() {
        // `exec` keeps the shell's process id, so the one printed is that of the sleeper.
        let command = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("echo $$; exec /bin/sleep 60");
        let started = Instant::now();

        let failure = panic::catch_unwind(|| command.run_within(Duration::from_millis(300)));

        assert!(started.elapsed() < Duration::from_secs(10));
        let message = failure.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("still running after 300ms"), "{message}");
        let pid = message
            .lines()
            .find_map(|line| line.parse::<u32>().ok())
            .unwrap();
        assert!(!is_alive(pid), "{message}");
    }

    #[test]
    fn a_stream_sent_elsewhere_is_not_captured() {
        let script = "echo out; echo err >&2";

        let quiet_stdout = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdout(Stdio::null())
            .run_within(Duration::from_secs(10));
        let quiet_stderr = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stderr(Stdio::null())
            .run_within(Duration::from_secs(10));

        assert_eq!(text(&quiet_stdout.stdout), "");
        assert_eq!(text(&quiet_stdout.stderr), "err\n");
        assert_eq!(text(&quiet_stderr.stdout), "out\n");
        assert_eq!(text(&quiet_stderr.stderr), "");
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
    fn wait_for_line_returns_the_first_whole_line_that_matches_while_the_binary_runs() {
        // The line arrives in two writes; its first half alone would match too.
        let running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(
                "echo starting; printf 'listen'; /bin/sleep 0.2; printf 'ing on 1\\n'; \
                 echo listening on 2; exec /bin/sleep 60",
            )
            .spawn();

        let line =
            running.wait_for_line(|line| line.starts_with("listen"), Duration::from_secs(10));
        let output = running.terminate(Duration::from_secs(10));

        assert_eq!(line.as_deref(), Some("listening on 1"));
        // Looking at the lines took none of them away.
        assert_eq!(
            text(&output.stdout),
            "starting\nlistening on 1\nlistening on 2\n"
        );
    }

    #[test]
    fn wait_for_line_finds_a_line_printed_before_it_was_called() {
        let ready = ReadyFile::new("line-printed-before");
        let running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg(r#"echo ready; : > "$READY"; exec /bin/sleep 60"#)
            .env("READY", ready.path())
            .spawn();
        ready.wait();

        let line = running.wait_for_line(|line| line == "ready", Duration::from_secs(10));
        running.terminate(Duration::from_secs(10));

        assert_eq!(line.as_deref(), Some("ready"));
    }

    #[test]
    fn wait_for_line_is_none_as_soon_as_the_binary_exits_without_printing_it() {
        let running = BinaryCommand::new("/bin/sh")
            .arg("-c")
            .arg("echo something else; /bin/sleep 0.2; exit 1")
            .spawn();

        let started = Instant::now();
        let line = running.wait_for_line(|line| line == "listening", Duration::from_secs(60));
        let output = running.terminate(Duration::from_secs(10));

        assert_eq!(line, None);
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(output.status.code(), Some(1));
    }

    #[test]
    fn wait_for_line_is_none_when_the_timeout_ends() {
        let running = BinaryCommand::new("/bin/sleep").arg("60").spawn();
        let timeout = Duration::from_millis(200);

        let started = Instant::now();
        let line = running.wait_for_line(|_| true, timeout);
        running.terminate(Duration::from_secs(10));

        assert_eq!(line, None);
        assert!(started.elapsed() >= timeout);
    }
}
