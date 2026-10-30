use std::{
    fs::File,
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    process::{Child, Command, ExitStatus, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use signal_hook::{
    consts::signal::{SIGCHLD, SIGHUP, SIGINT, SIGKILL, SIGTERM},
    iterator::{Handle, Signals},
};

use crate::terminal::TerminalGuard;

const GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct Interrupted(pub i32);
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interrupted by signal {}", self.0)
    }
}
impl std::error::Error for Interrupted {}

#[derive(Default)]
struct SignalState {
    first: AtomicI32,
    count: AtomicUsize,
}

pub struct Supervisor {
    state: Arc<SignalState>,
    events: Mutex<Receiver<()>>,
    handle: Handle,
    worker: Option<JoinHandle<()>>,
}

impl Supervisor {
    pub fn new() -> Result<Arc<Self>> {
        // SAFETY: Linux subreaper status affects only this CLI and permits reaping its orphaned descendants.
        if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == -1 {
            return Err(io::Error::last_os_error()).context("enable child-process supervision");
        }
        let mut signals =
            Signals::new([SIGINT, SIGTERM, SIGHUP, SIGCHLD]).context("register process signals")?;
        let handle = signals.handle();
        let state = Arc::new(SignalState::default());
        let shared = Arc::clone(&state);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("commitfs-signals".into())
            .spawn(move || {
                for signal in signals.forever() {
                    if signal != SIGCHLD {
                        let _ = shared.first.compare_exchange(
                            0,
                            signal,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        );
                        shared.count.fetch_add(1, Ordering::SeqCst);
                    }
                    if sender.send(()).is_err() {
                        break;
                    }
                }
            })
            .context("start signal listener")?;
        Ok(Arc::new(Self {
            state,
            events: Mutex::new(receiver),
            handle,
            worker: Some(worker),
        }))
    }

    pub fn check(&self) -> Result<()> {
        let signal = self.state.first.load(Ordering::SeqCst);
        if signal == 0 {
            Ok(())
        } else {
            Err(Interrupted(signal).into())
        }
    }

    fn spawn(&self, command: &mut Command, foreground: bool) -> Result<ManagedChild> {
        self.check()?;
        let child = command
            .process_group(0)
            .spawn()
            .context("start managed process")?;
        let group = i32::try_from(child.id()).context("process-group ID exceeds PID range")?;
        let mut managed = ManagedChild {
            child,
            group,
            finished: false,
            terminal: TerminalGuard::default(),
        };
        if foreground {
            managed.terminal = TerminalGuard::give_to(group)?;
        }
        Ok(managed)
    }

    pub fn status(&self, command: &mut Command) -> Result<ExitStatus> {
        let mut child = self.spawn(command, true)?;
        self.wait(&mut child)
    }

    pub fn capture(
        &self,
        command: &mut Command,
        input: Option<Vec<u8>>,
        stdout: Option<File>,
    ) -> Result<Output> {
        command
            .stdout(stdout.map_or_else(Stdio::piped, Stdio::from))
            .stderr(Stdio::piped());
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = self.spawn(command, false)?;
        let writer = input.and_then(|data| {
            child
                .child
                .stdin
                .take()
                .map(|mut pipe| thread::spawn(move || pipe.write_all(&data)))
        });
        let out = child.child.stdout.take().map(reader);
        let err = child.child.stderr.take().map(reader);
        let status = self.wait(&mut child);
        let stdout = join_reader(out);
        let stderr = join_reader(err);
        let written = writer
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| anyhow!("process input writer panicked"))
            })
            .transpose()?;
        let status = status?;
        if status.success()
            && let Some(result) = written
        {
            result.context("write process input")?;
        }
        Ok(Output {
            status,
            stdout: stdout?,
            stderr: stderr?,
        })
    }

    fn wait(&self, child: &mut ManagedChild) -> Result<ExitStatus> {
        let mut interrupted = None;
        let mut deadline = None;
        let mut signals_seen = 0;
        loop {
            let count = self.state.count.load(Ordering::SeqCst);
            if count > signals_seen {
                let signal = self.state.first.load(Ordering::SeqCst);
                kill_group(
                    child.group,
                    if signals_seen == 0 { signal } else { SIGKILL },
                )?;
                interrupted = Some(signal);
                deadline = Some(Instant::now() + GRACE);
                signals_seen = count;
            }
            if let Some(status) = child.child.try_wait().context("wait for managed process")? {
                self.stop_group(child.group)?;
                child.finished = true;
                return interrupted.map_or(Ok(status), |signal| Err(Interrupted(signal).into()));
            }
            if deadline.is_some_and(|end| Instant::now() >= end) {
                kill_group(child.group, SIGKILL)?;
                deadline = None;
            }
            self.event(deadline.map(|end| end.saturating_duration_since(Instant::now())))?;
        }
    }

    fn stop_group(&self, group: i32) -> Result<()> {
        reap(group);
        if !group_exists(group) {
            return Ok(());
        }
        kill_group(group, SIGTERM)?;
        let deadline = Instant::now() + GRACE;
        let initial_count = self.state.count.load(Ordering::SeqCst);
        let mut killed = false;
        loop {
            reap(group);
            if !group_exists(group) {
                return Ok(());
            }
            if !killed
                && (Instant::now() >= deadline
                    || self.state.count.load(Ordering::SeqCst) > initial_count)
            {
                kill_group(group, SIGKILL)?;
                killed = true;
            }
            self.event(if killed {
                None
            } else {
                Some(deadline.saturating_duration_since(Instant::now()))
            })?;
        }
    }

    fn event(&self, timeout: Option<Duration>) -> Result<()> {
        let events = self
            .events
            .lock()
            .map_err(|_| anyhow!("process event receiver poisoned"))?;
        match timeout {
            Some(timeout) => {
                let _ = events.recv_timeout(timeout);
            }
            None => {
                let _ = events.recv();
            }
        }
        drop(events);
        Ok(())
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct ManagedChild {
    child: Child,
    group: i32,
    finished: bool,
    terminal: TerminalGuard,
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = kill_group(self.group, SIGKILL);
            let _ = self.child.wait();
            reap(self.group);
        }
    }
}

fn kill_group(group: i32, signal: i32) -> io::Result<()> {
    // SAFETY: The group ID is a positive PID allocated to a child spawned in its own group.
    if unsafe { libc::kill(-group, signal) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

fn group_exists(group: i32) -> bool {
    // SAFETY: Signal 0 only checks the managed process group's existence.
    unsafe { libc::kill(-group, 0) == 0 }
}

fn reap(group: i32) {
    // SAFETY: The main child has already been reaped; only adopted children in this managed group are collected.
    unsafe { while libc::waitpid(-group, std::ptr::null_mut(), libc::WNOHANG) > 0 {} }
}

fn reader(mut pipe: impl Read + Send + 'static) -> JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut data = Vec::new();
        pipe.read_to_end(&mut data)?;
        Ok(data)
    })
}

fn join_reader(worker: Option<JoinHandle<io::Result<Vec<u8>>>>) -> Result<Vec<u8>> {
    worker.map_or_else(
        || Ok(Vec::new()),
        |worker| {
            worker
                .join()
                .map_err(|_| anyhow!("process output reader panicked"))?
                .context("read process output")
        },
    )
}
