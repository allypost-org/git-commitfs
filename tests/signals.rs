mod common;

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::{fd::FromRawFd, unix::process::CommandExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use common::Repo;

struct Managed {
    child: Child,
    groups: Vec<i32>,
    paths: Vec<PathBuf>,
}

impl Managed {
    fn spawn(command: &mut Command) -> Self {
        #[cfg(target_os = "linux")]
        rustix::process::set_child_subreaper(Some(rustix::process::Pid::INIT))
            .expect("fixture subreaper");
        let child = command
            .process_group(0)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn supervised fixture");
        Self {
            child,
            groups: Vec::new(),
            paths: Vec::new(),
        }
    }

    fn lines(&mut self) -> Receiver<String> {
        lines(self.child.stdout.take().expect("fixture stdout"))
    }

    fn track(&mut self, pid: i32) {
        // SAFETY: pid was reported by this test's own live child.
        let group = unsafe { libc::getpgid(pid) };
        if group > 0 {
            self.groups.push(group);
        }
    }

    fn signal(&self, signal: i32) {
        // SAFETY: The PID belongs to this test's live wrapper process.
        assert_eq!(
            unsafe { libc::kill(i32::try_from(self.child.id()).expect("pid"), signal) },
            0
        );
    }

    fn wait(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            if let Some(status) = self.child.try_wait().expect("wrapper status") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "wrapper failed to stop before watchdog deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Managed {
    fn drop(&mut self) {
        let pid = i32::try_from(self.child.id()).expect("owned PID");
        collect_groups(pid, &mut self.groups);
        self.groups.push(pid);
        // SAFETY: All group IDs came from our isolated fixture process tree.
        let own_group = unsafe { libc::getpgrp() };
        for &group in &self.groups {
            if group > 0 && group != own_group {
                // SAFETY: Never target the test harness's process group.
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
            }
        }
        for path in &self.paths {
            if let Some(root) = path.parent().filter(|root| {
                root.file_name()
                    .is_some_and(|name| name.as_encoded_bytes().starts_with(b"git-commitfs-"))
            }) {
                let _ = fs::remove_dir_all(root);
            }
        }
        let _ = self.child.wait();
        for &group in &self.groups {
            if group > 0 && group != own_group {
                // SAFETY: Reap only fixture children in a fixture-owned process group.
                unsafe { while libc::waitpid(-group, std::ptr::null_mut(), libc::WNOHANG) > 0 {} }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn collect_groups(pid: i32, groups: &mut Vec<i32>) {
    if let Ok(children) = fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
        for child in children
            .split_whitespace()
            .filter_map(|s| s.parse::<i32>().ok())
        {
            collect_groups(child, groups);
            // SAFETY: The PID is a descendant of this test's owned child.
            let group = unsafe { libc::getpgid(child) };
            if group > 0 {
                groups.push(group);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn collect_groups(pid: i32, groups: &mut Vec<i32>) {
    use libproc::processes::{ProcFilter, pids_by_type};
    let Ok(parent) = u32::try_from(pid) else {
        return;
    };
    if let Ok(children) = pids_by_type(ProcFilter::ByParentProcess { ppid: parent }) {
        for child in children
            .into_iter()
            .filter_map(|pid| i32::try_from(pid).ok())
        {
            collect_groups(child, groups);
            // SAFETY: The child was found under this test's fixture process.
            let group = unsafe { libc::getpgid(child) };
            if group > 0 {
                groups.push(group);
            }
        }
    }
}

fn lines(input: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(input).lines() {
            let Ok(line) = line else {
                break;
            };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

fn ready(child: &mut Managed, receiver: &Receiver<String>) -> (i32, PathBuf) {
    let line = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("fixture ready marker");
    let mut fields = line.splitn(3, ' ');
    let pid: i32 = fields
        .next()
        .expect("command PID")
        .parse()
        .expect("command PID number");
    let worker: i32 = fields
        .next()
        .expect("worker PID")
        .parse()
        .expect("worker PID number");
    let path = PathBuf::from(fields.next().expect("workspace"));
    child.paths.push(path.clone());
    child.track(pid);
    child.track(worker);
    (worker, path)
}

fn alive(pid: i32) -> bool {
    // SAFETY: Signal 0 only queries whether the fixture PID exists.
    let present = unsafe { libc::kill(pid, 0) == 0 };
    #[cfg(target_os = "macos")]
    if present {
        // launchd reaps orphaned zombies asynchronously; they cannot execute.
        if let Ok(info) = libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(pid, 1) {
            return info.pbi_status != libc::SZOMB;
        }
    }
    present
}

#[test]
fn termination_signals_stop_descendants_and_clean_the_workspace() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let repo = Repo::new();
        let mut child = Managed::spawn(repo.run_command().args([
            "run",
            if signal == libc::SIGHUP {
                "--keep=on-failure"
            } else {
                "--keep=never"
            },
            "--",
            "sh",
            "-c",
            "trap 'exit 0' INT TERM HUP; sleep 1000 >/dev/null 2>&1 & printf '%s %s %s\\n' \"$$\" \
             \"$!\" \"$PWD\"; wait",
        ]));
        let receiver = child.lines();
        let (worker, path) = ready(&mut child, &receiver);
        child.signal(signal);
        assert_eq!(child.wait().code(), Some(128 + signal));
        assert!(!alive(worker), "worker survived signal forwarding");
        assert_eq!(
            path.exists(),
            signal == libc::SIGHUP,
            "interrupted retention policy"
        );
    }
}

#[test]
fn background_workers_are_reaped_even_after_the_main_command_exits() {
    let repo = Repo::new();
    let mut child = Managed::spawn(repo.run_command().args([
        "run",
        "--",
        "sh",
        "-c",
        "sleep 1000 >/dev/null 2>&1 & printf '%s %s %s\\n' \"$$\" \"$!\" \"$PWD\"; exit 17",
    ]));
    let receiver = child.lines();
    let (worker, path) = ready(&mut child, &receiver);
    assert_eq!(child.wait().code(), Some(17));
    assert!(!alive(worker), "background worker was left alive");
    assert!(!path.exists());
}

#[test]
fn ignored_termination_is_escalated_after_a_bounded_grace_period() {
    let repo = Repo::new();
    let mut child = Managed::spawn(repo.run_command().args([
        "run",
        "--",
        "sh",
        "-c",
        "trap '' TERM; sleep 1000 >/dev/null 2>&1 & printf '%s %s %s\\n' \"$$\" \"$!\" \"$PWD\"; \
         wait",
    ]));
    let receiver = child.lines();
    let (worker, path) = ready(&mut child, &receiver);
    let start = Instant::now();
    child.signal(libc::SIGTERM);
    assert_eq!(child.wait().code(), Some(143));
    assert!(start.elapsed() < Duration::from_secs(8));
    assert!(!alive(worker));
    assert!(!path.exists());
}

#[test]
fn interruption_during_a_checkout_filter_stops_helpers_and_never_runs_the_command() {
    let repo = Repo::new();
    repo.write(".gitattributes", b"file filter=demo\n");
    repo.write("file", b"content\n");
    repo.git(&["add", "."]);
    repo.git(&["config", "filter.demo.required", "true"]);
    repo.git(&[
        "config",
        "filter.demo.smudge",
        "printf '%s %s\\n' \"$$\" \"$PWD\" > \"$READY_FILE\"; sleep 1000; cat",
    ]);
    let fifo = repo.storage.path().join("filter-ready");
    assert!(
        repo.command("mkfifo")
            .arg(&fifo)
            .status()
            .expect("ready FIFO")
            .success()
    );
    let (sender, receiver) = mpsc::channel();
    let reader = fifo.clone();
    std::thread::spawn(move || {
        let mut line = String::new();
        let mut reader = BufReader::new(fs::File::open(reader).expect("filter ready FIFO"));
        reader.read_line(&mut line).expect("filter marker");
        let _ = sender.send(line);
    });
    let mut child = Managed::spawn(repo.run_command().env("READY_FILE", fifo).args([
        "run",
        "--",
        "printf",
        "must not run",
    ]));
    let user_output = child.lines();
    let marker = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("filter started");
    let (pid, path) = marker
        .trim_end()
        .split_once(' ')
        .expect("filter PID and workspace");
    let pid = pid.parse().expect("filter PID");
    let path = PathBuf::from(path);
    child.track(pid);
    child.paths.push(path.clone());
    child.signal(libc::SIGTERM);
    assert_eq!(child.wait().code(), Some(143));
    assert!(!alive(pid));
    assert!(!path.exists());
    assert!(user_output.try_iter().next().is_none());
}

#[test]
fn a_second_signal_escalates_without_waiting_for_the_grace_period() {
    let repo = Repo::new();
    let mut child = Managed::spawn(repo.run_command().args([
        "run",
        "--",
        "sh",
        "-c",
        "trap 'printf ack\\n' TERM; sleep 1000 >/dev/null 2>&1 & printf '%s %s %s\\n' \"$$\" \
         \"$!\" \"$PWD\"; while :; do wait; sleep 1000 >/dev/null 2>&1 & done",
    ]));
    let receiver = child.lines();
    let (_, path) = ready(&mut child, &receiver);
    child.signal(libc::SIGTERM);
    let acknowledgement = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("first signal forwarded");
    assert!(acknowledgement.starts_with("ack"));
    let start = Instant::now();
    child.signal(libc::SIGTERM);
    assert_eq!(child.wait().code(), Some(143));
    assert!(start.elapsed() < Duration::from_millis(1800));
    assert!(!path.exists());
}

#[test]
#[allow(
    clippy::useless_conversion,
    reason = "ioctl request types differ between Linux and macOS"
)]
fn interactive_input_ctrl_c_and_terminal_restoration_work() {
    let repo = Repo::new();
    let (mut master_fd, mut slave_fd) = (-1, -1);
    // SAFETY: openpty initializes both descriptors; no name or termios buffers are supplied.
    assert_eq!(
        unsafe {
            libc::openpty(
                &raw mut master_fd,
                &raw mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    // SAFETY: Both descriptors were returned successfully and have no other Rust owner.
    let (mut master, slave) = unsafe {
        (
            fs::File::from_raw_fd(master_fd),
            fs::File::from_raw_fd(slave_fd),
        )
    };
    let receiver = lines(master.try_clone().expect("PTY reader"));
    let controller = r#""$BIN" run -- sh -c 'printf "ready:%s\n" "$PWD"; IFS= read -r value; printf "read:%s\n" "$value"; IFS= read -r wait'; status=$?; printf "status:%s\n" "$status"; IFS= read -r ack; printf "restored:%s\n" "$ack""#;
    let mut command = repo.command("sh");
    command
        .args(["-c", controller])
        .env("BIN", env!("CARGO_BIN_EXE_git-commitfs"))
        .stdin(slave.try_clone().expect("PTY stdin"))
        .stdout(slave.try_clone().expect("PTY stdout"))
        .stderr(slave.try_clone().expect("PTY stderr"));
    // SAFETY: Only async-signal-safe syscalls run between fork and exec, using the inherited PTY stdin.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1
                || libc::ioctl(0, libc::TIOCSCTTY.into(), 0) == -1
                || libc::tcsetpgrp(0, libc::getpgrp()) == -1
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Managed {
        child: command.spawn().expect("PTY controller"),
        groups: Vec::new(),
        paths: Vec::new(),
    };
    drop(command);
    drop(slave);
    let marker = until(&receiver, "ready:");
    child.paths.push(PathBuf::from(
        marker
            .trim_end()
            .strip_prefix("ready:")
            .expect("PTY workspace"),
    ));
    master.write_all(b"hello\n").expect("terminal input");
    until(&receiver, "read:hello");
    master.write_all(&[3]).expect("terminal Ctrl-C");
    until(&receiver, "status:130");
    master
        .write_all(b"acknowledged\n")
        .expect("restored terminal input");
    until(&receiver, "restored:acknowledged");
    assert_eq!(child.wait().code(), Some(0));
}

fn until(receiver: &Receiver<String>, marker: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let line = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("PTY watchdog deadline");
        if line.contains(marker) {
            return line;
        }
    }
}
