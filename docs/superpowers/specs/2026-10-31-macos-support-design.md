# macOS support and platform boundaries

The user selected the hybrid approach and authorized implementation on 2026-10-31:
small internal interfaces backed by standard APIs and focused crates where they
preserve the existing behavior. This supplements the original staged-command
design; its snapshot, isolation, exit-code, and lifetime contracts still apply.

## Intent and scope

Support native Linux and macOS execution with Git >= 2.43. Preserve partial
staging, alternate indexes, byte-valued Unix paths/arguments, private Git storage,
checkout conversions, ignored imports, retention, terminal input, and signals.
No root, mounts, FUSE, writeback, or shared source cache is introduced. Windows
and BSD are future ports, not claimed supported platforms in this change.

## Approaches

- Project-owned implementations: precise control, but all syscall safety and
  platform maintenance remain ours.
- Broad copying/process crates: existing platform support, but their path APIs
  and lifecycle semantics still require adaptation to our contract.
- Selected hybrid: project-owned capability interfaces using standard APIs and
  rustix's safe descriptor/process operations. Keep product policy independent
  of the primitive implementation; add no speculative backend trait hierarchy.

The concrete crate choice is an implementation decision: rustix already supplies
Linux ioctl_ficlone and Apple fclonefileat, so source paths need not be reopened
by a cloning crate. process-wrap is unnecessary for creating Unix groups (std
already does it) and does not replace our cancellation/terminal/cleanup policy.

## Platform boundaries

`src/platform/` owns filesystem OS-string conversion, symlink creation,
permission repair, safe source opening, clone primitives, managed process-group
operations, supervision initialization, and terminal ownership. Linux and macOS
specializations live behind compile-time selection. Existing capture,
materialization, ignored selection, copy policy, and orchestration stay separate.
Future ports can implement these capabilities without exposing group IDs or
cloning syscalls to the app; a Windows port will still need its own path encoding,
process/terminal semantics and tests.

`copy.rs` owns auto/copy/reflink policy and error precedence. Source handles use
no-follow/nonblocking open and must be regular files. Destinations are created
exclusively. Linux clones between descriptors; Apple clones from the validated
source descriptor into a unique sibling temporary file, then replaces the owned
destination and updates its handle. tempfile owns intermediate paths. Preserve
permissions and independently writable storage. Unsupported/cross-device errors
permit auto fallback; permission, I/O and storage failures propagate. Strict
reflink never silently copies. Existing destination paths must survive failure.

`process.rs` owns cancellation, status and the two-second TERM-to-KILL grace
period; platform code owns group spawn/signaling/existence/reaping. Linux retains
subreaper adoption and SIGCHLD-driven waits. macOS leaves orphan reaping to the OS;
while draining a group it distinguishes running/exiting/zombie states and uses a sleeping bounded event wait to recheck completion
because orphaned grandchildren do not send SIGCHLD to this wrapper. Main-child
waits remain signal-driven. A macOS direct-exec adapter precomputes argv, environment
and PATH candidates before fork and uses execve from std's pre_exec hook. Apple's
execvp/posix_spawnp otherwise interpret ENOEXEC through a shell, violating the
existing direct-argv/exit-126 contract. No allocation runs in the child callback. Never delete a workspace while live or exiting group members remain; zombies
cannot execute and are reaped by the OS on macOS.
Detached processes, SIGKILL and machine failure remain outside the guarantee.
The periodic macOS wake is a completion-observation implementation decision, not
an application busy loop or a new cleanup timeout that permits surviving workers.

## Build, hooks and compatibility

Cargo outside mise defaults to the native host. The mise environment selects
x86_64-unknown-linux-musl on x86-64 Linux and the native target on macOS, respecting
an explicit CARGO_BUILD_TARGET. Keep Rust/tool version pins in mise.toml and
Linux static ELF CI checks. Recipes, installation and hook bootstrap must agree
on target/output paths, without GNU-only realpath options. macOS system Bash 3.2
is sufficient. Keep Git >= 2.43; require Xcode Command Line Tools for native builds.

Test the same product contract on macOS and Linux. Replace GNU stat in assertions
with Rust metadata, adapt PTY signatures and test cleanup discovery, and normalize
fixture paths where macOS aliases /var to /private/var. Filesystem limitations
(case sensitivity, normalization, clone support) remain those of the local disk.
Support Apple Silicon with direct validation and configure macOS CI. Document
Intel macOS separately if it cannot be executed in the available environment.

## Validation

Run formatter/fix pipeline and read-only checks with mise. Use the entire real-Git
suite on Linux and on the SSH Mac, including partial commits, alternate indexes,
clone modes, filters, signal escalation, leftover descendants, and foreground PTY
input/restoration. Exercise native debug/release help and temporary-root install,
plus the real staged hook. Verify the Linux release remains fully static. Add
regressions for clone handle stability, destination collisions and cleanup after
strict failures. Validate remote changes in an isolated temporary checkout; leave
the user's Mac clone and its preexisting mise.lock edits intact.
