use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_CAPTURE_BYTES: usize = 1024 * 1024;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const TRUNCATION_MARKER: &[u8] = b"\n...[output truncated]\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessFailure {
    Spawn {
        program: PathBuf,
        message: String,
    },
    Capture {
        program: PathBuf,
        message: String,
    },
    Timeout {
        program: PathBuf,
        timeout: Duration,
    },
    NonZero {
        program: PathBuf,
        code: Option<i32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

impl fmt::Display for ProcessFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { program, message } => {
                write!(
                    formatter,
                    "failed to start {}: {message}",
                    program.display()
                )
            }
            Self::Capture { program, message } => write!(
                formatter,
                "failed to collect output from {}: {message}",
                program.display()
            ),
            Self::Timeout { program, timeout } => write!(
                formatter,
                "{} timed out after {} ms",
                program.display(),
                timeout.as_millis()
            ),
            Self::NonZero { program, code, .. } => write!(
                formatter,
                "{} exited with status {}",
                program.display(),
                code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
            ),
        }
    }
}

impl std::error::Error for ProcessFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlledProcessFailure {
    Process(ProcessFailure),
    Cancelled,
}

pub fn execute(command: &CommandSpec, timeout: Duration) -> Result<ProcessOutput, ProcessFailure> {
    match execute_cancellable(command, timeout, || false) {
        Ok(output) => Ok(output),
        Err(ControlledProcessFailure::Process(error)) => Err(error),
        Err(ControlledProcessFailure::Cancelled) => Err(ProcessFailure::Capture {
            program: command.program.clone(),
            message: "process was unexpectedly cancelled".to_string(),
        }),
    }
}

pub fn execute_cancellable(
    command: &CommandSpec,
    timeout: Duration,
    is_cancelled: impl Fn() -> bool,
) -> Result<ProcessOutput, ControlledProcessFailure> {
    if is_cancelled() {
        return Err(ControlledProcessFailure::Cancelled);
    }

    let mut process = Command::new(&command.program);
    process
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in &command.env_remove {
        process.env_remove(key);
    }
    for (key, value) in &command.env {
        process.env(key, value);
    }
    configure_process_group(&mut process);

    let mut child = process.spawn().map_err(|error| {
        ControlledProcessFailure::Process(ProcessFailure::Spawn {
            program: command.program.clone(),
            message: error.to_string(),
        })
    })?;
    let mut process_tree = ProcessTree::attach(&child).map_err(|message| {
        let _ = child.kill();
        let _ = child.wait();
        ControlledProcessFailure::Process(ProcessFailure::Spawn {
            program: command.program.clone(),
            message,
        })
    })?;

    let stdout = take_stdout(&mut child, &mut process_tree, &command.program)?;
    let stderr = take_stderr(&mut child, &mut process_tree, &command.program)?;
    let stdout_reader = thread::spawn(move || read_pipe_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_pipe_bounded(stderr));
    let started = Instant::now();

    let outcome = loop {
        if is_cancelled() {
            break WaitOutcome::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => break WaitOutcome::Exited(status),
            Ok(None) if started.elapsed() < timeout => thread::sleep(PROCESS_POLL_INTERVAL),
            Ok(None) => break WaitOutcome::TimedOut,
            Err(error) => break WaitOutcome::WaitFailed(error.to_string()),
        }
    };

    // Always terminate the whole tree before joining pipe readers. A renderer
    // can otherwise exit after spawning a background child that inherits the
    // pipe handles, leaving the readers and the caller blocked indefinitely.
    process_tree.terminate();
    if !matches!(&outcome, WaitOutcome::Exited(_)) {
        let _ = child.kill();
    }
    let _ = child.wait();

    match outcome {
        WaitOutcome::Cancelled => {
            let _ = collect_pipe(stdout_reader, &command.program);
            let _ = collect_pipe(stderr_reader, &command.program);
            Err(ControlledProcessFailure::Cancelled)
        }
        WaitOutcome::TimedOut => {
            let _ = collect_pipe(stdout_reader, &command.program);
            let _ = collect_pipe(stderr_reader, &command.program);
            Err(ControlledProcessFailure::Process(ProcessFailure::Timeout {
                program: command.program.clone(),
                timeout,
            }))
        }
        WaitOutcome::WaitFailed(message) => {
            let _ = collect_pipe(stdout_reader, &command.program);
            let _ = collect_pipe(stderr_reader, &command.program);
            Err(ControlledProcessFailure::Process(ProcessFailure::Capture {
                program: command.program.clone(),
                message,
            }))
        }
        WaitOutcome::Exited(status) => {
            let stdout = collect_pipe(stdout_reader, &command.program)
                .map_err(ControlledProcessFailure::Process)?;
            let stderr = collect_pipe(stderr_reader, &command.program)
                .map_err(ControlledProcessFailure::Process)?;
            if !status.success() {
                return Err(ControlledProcessFailure::Process(ProcessFailure::NonZero {
                    program: command.program.clone(),
                    code: status.code(),
                    stdout,
                    stderr,
                }));
            }
            Ok(ProcessOutput { stdout, stderr })
        }
    }
}

enum WaitOutcome {
    Exited(ExitStatus),
    TimedOut,
    Cancelled,
    WaitFailed(String),
}

fn take_stdout(
    child: &mut Child,
    process_tree: &mut ProcessTree,
    program: &Path,
) -> Result<std::process::ChildStdout, ControlledProcessFailure> {
    child.stdout.take().ok_or_else(|| {
        process_tree.terminate();
        let _ = child.kill();
        let _ = child.wait();
        ControlledProcessFailure::Process(ProcessFailure::Capture {
            program: program.to_path_buf(),
            message: "stdout pipe unavailable".to_string(),
        })
    })
}

fn take_stderr(
    child: &mut Child,
    process_tree: &mut ProcessTree,
    program: &Path,
) -> Result<std::process::ChildStderr, ControlledProcessFailure> {
    child.stderr.take().ok_or_else(|| {
        process_tree.terminate();
        let _ = child.kill();
        let _ = child.wait();
        ControlledProcessFailure::Process(ProcessFailure::Capture {
            program: program.to_path_buf(),
            message: "stderr pipe unavailable".to_string(),
        })
    })
}

fn read_pipe_bounded(mut pipe: impl Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::with_capacity(MAX_CAPTURE_BYTES.min(64 * 1024));
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let read = pipe.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = MAX_CAPTURE_BYTES.saturating_sub(output.len());
        let retained = remaining.min(read);
        output.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    if truncated {
        let retained = MAX_CAPTURE_BYTES.saturating_sub(TRUNCATION_MARKER.len());
        output.truncate(retained);
        output.extend_from_slice(TRUNCATION_MARKER);
    }
    Ok(output)
}

fn collect_pipe(
    reader: thread::JoinHandle<std::io::Result<Vec<u8>>>,
    program: &Path,
) -> Result<Vec<u8>, ProcessFailure> {
    reader
        .join()
        .map_err(|_| ProcessFailure::Capture {
            program: program.to_path_buf(),
            message: "output reader thread panicked".to_string(),
        })?
        .map_err(|error| ProcessFailure::Capture {
            program: program.to_path_buf(),
            message: error.to_string(),
        })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_group(_command: &mut Command) {}

#[cfg(not(any(unix, windows)))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
struct ProcessTree {
    process_group: i32,
    terminated: bool,
}

#[cfg(unix)]
impl ProcessTree {
    fn attach(child: &Child) -> Result<Self, String> {
        let process_group = i32::try_from(child.id())
            .map_err(|_| "child process id does not fit a Unix process group id".to_string())?;
        Ok(Self {
            process_group,
            terminated: false,
        })
    }

    fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        // The child was spawned as its own process-group leader, so a negative
        // pid targets the renderer and every descendant in that group.
        unsafe {
            libc::kill(-self.process_group, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(windows)]
struct ProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
    terminated: bool,
}

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(format!(
                "failed to create Windows renderer job object: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            let error = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(job);
            }
            return Err(format!(
                "failed to configure Windows renderer job object: {error}"
            ));
        }
        let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle().cast()) };
        if assigned == 0 {
            let error = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(job);
            }
            return Err(format!(
                "failed to assign renderer to Windows job object: {error}"
            ));
        }
        Ok(Self {
            job,
            terminated: false,
        })
    }

    fn terminate(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        self.terminate();
        unsafe {
            CloseHandle(self.job);
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct ProcessTree {
    terminated: bool,
}

#[cfg(not(any(unix, windows)))]
impl ProcessTree {
    fn attach(_child: &Child) -> Result<Self, String> {
        Ok(Self { terminated: false })
    }

    fn terminate(&mut self) {
        self.terminated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_drains_but_never_retains_more_than_the_limit() {
        let input = vec![b'x'; MAX_CAPTURE_BYTES + 4096];
        let output = read_pipe_bounded(input.as_slice()).unwrap();

        assert_eq!(output.len(), MAX_CAPTURE_BYTES);
        assert!(output.ends_with(TRUNCATION_MARKER));
    }
}
