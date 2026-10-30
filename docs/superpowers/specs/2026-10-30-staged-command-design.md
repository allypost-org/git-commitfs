# git-commitfs: staged command execution

Status: approved by the user on 2026-10-30. Approval covers the design and
writing the implementation plan, not starting product implementation.

## Intent and decisions from the discussion

Run an arbitrary command against what the current Git index would commit,
without temporarily changing the user's checkout. The motivating use is
pre-commit checks that must handle partially staged files correctly.

The user has chosen Linux as the initial platform, with macOS desirable later
and Windows a distant consideration. Ease of implementation now takes priority
over avoiding every full checkout. Preparation and execution should have
separate responsibilities so optimizations can be selected later.

All writes inside the disposable workspace, including writes to included
ignored files, are disposable. Persisting command results to the original
checkout is a desirable future option, not required initially. Until then,
users run a separate ordinary command when they want persistent changes.

The remaining concrete defaults below are proposed engineering decisions,
not preferences explicitly supplied by the user.

## First-release contract

- Implementation: the existing Rust package, edition 2024, calling the installed
  Git CLI for Git operations rather than implementing checkout with a Git library.
- Supported platform: Linux, without requiring root, FUSE, user namespaces, or
  mount permissions. macOS is a later validation target, not a v1 guarantee.
- Proposed Git minimum: 2.43, with an actionable check before preparation.
- The tracked path set, contents, symlink targets, and executable bits come
  exclusively from a captured index. Unchanged tracked files are included too.
- Unstaged tracked edits and ordinary untracked files are absent.
- Staged additions, deletions, renames, type changes, empty trees, and repositories
  without an existing commit work. Rename detection is unnecessary for checkout.
- The effective index is honored, including `GIT_INDEX_FILE` supplied by a hook
  or a partial/path-limited commit. Do not assume `.git/index` is the candidate.
- Commands may mutate their workspace and its private Git metadata. No tracked
  changes are automatically copied back or staged in the source repository.
- A successful formatter that edits files still succeeds: no implicit
  fail-on-change policy. Users can run formatter check modes or inspect the
  workspace's Git diff themselves.
- Working tree, index, refs, configuration, hooks, and object files in the source
  repository are not written by git-commitfs's own preparation or execution logic.

This is a local checkout with the same effective checkout settings, not a
hermetic reproduction of another computer. Toolchains, environment variables,
and explicitly included ignored files remain local inputs.

## CLI and observable behavior

```sh
git commitfs run -- cargo test
git commitfs run --include-ignored -- cargo build
git commitfs run --include-ignored --copy-mode=copy -- npm test
git commitfs run --keep=on-failure -- cargo clippy -- -D warnings
git commitfs run -- sh -c 'cargo fmt --check && cargo test'
```

Install a binary named `git-commitfs`, usable directly or as `git commitfs`.
Arguments after `--` are passed directly to the executable; there is no implicit
shell. Commands start at the snapshot repository root, regardless of the caller's
subdirectory. Relative script and argument paths consequently refer to that root.

Initial options:

- `--include-ignored`: import eligible ignored files, otherwise import none.
- `--copy-mode=auto|copy|reflink`: controls copying imported ignored regular files.
  `auto` tries reflinks and falls back to independent copies when unsupported;
  `copy` always copies; `reflink` requires reflink support. Default: `auto`.
  This does not make Git's direct tracked-file checkout lazy or reflink-based.
- `--keep=never|on-failure|always`: retain the disposable directory for inspection.
  Default: `never`. Print the retained absolute path to stderr. Retention does
  not publish changes to the original checkout.

No backend-selection flag with only one implemented workspace backend. Add
selectors when another complete strategy exists, rather than promise arbitrary
combinations that cannot work together.

Inherit stdin, stdout, and stderr. Keep wrapper diagnostics on stderr. Preserve
ordinary child exit codes, including 125. Wrapper/setup/cleanup errors use 125;
usage errors use 2; executable-not-found uses 127; non-executable uses 126;
signal termination uses 128 plus the signal number. Diagnostics distinguish
wrapper failures from a command that itself exits 125. Cleanup failure after
a successful command returns 125; cleanup failure must not mask a child failure.

## Architecture and alternatives

Considered approaches:

1. Immutable materialized checkout plus a fresh OverlayFS writable layer. Good
   reuse and conventional filesystem behavior, but requires mounts and cache
   lifecycle machinery, and still needs initial materialization.
2. Lazy Git-backed FUSE plus disposable writes. Avoids eager expansion but adds
   deployment restrictions, filesystem implementation, and checkout-filter work.
3. Direct materialization into a disposable directory. Simplest correctness and
   deployment story, at the cost of checkout time and temporary disk space.

Selected direction from the discussion: start with the simpler materialized
approach and separate stages; defer mandatory mounts and large-repository
optimizations. The specific v1 strategy proposed here is option 3.

The responsibilities are:

1. **Discovery and capture:** resolve the source repository and effective index;
   produce one immutable commit-candidate description.
2. **Workspace allocation:** own a unique temporary directory and its lifetime.
3. **Materialization:** populate tracked files and a private Git repository, then
   optionally import ignored files. Never execute a command against a partially
   prepared workspace.
4. **Execution:** run the user's argv, supervise its process group, preserve the
   result, and apply the retention/cleanup policy.

Capture and allocation can be arranged to share a single per-run storage owner;
these are responsibility boundaries, not a requirement for four trait hierarchies
or separate subprocesses. Initial tracked files are checked out directly into
the final disposable workspace, with no redundant intermediate full checkout.

Use focused Rust modules for CLI, repository/index capture, workspace checkout,
ignored-file copying, and process execution. Introduce interfaces around actual
exchange data and resource ownership; do not build a generic backend framework
before there is a second backend.

## Capturing the index and preparing Git state

Resolve source paths using Git, including linked worktrees and paths containing
spaces or non-UTF-8 bytes. Treat object IDs as opaque Git-format identifiers;
do not assume SHA-1 length. Use NUL-delimited path output and native OS strings.

Read a stable copy of the effective index without modifying it, and ask Git to
produce the candidate tree using private index/object storage. Split indexes
must be resolved while their source shared-index data is available; subsequent
stages use a self-contained private index. Do not turn a concurrent replacement
of the source index into a mixture of two candidates. With no index, the
candidate is empty. Later staging never changes an already captured run.

Reject unmerged index entries and intent-to-add entries before command execution,
with an explanation of how to stage or resolve them. Intent-to-add support can
be improved later; do not silently treat an uncommitted new file as an empty file.
Populate the full staged tree even when the source uses sparse checkout.

Give the workspace its own `.git` directory, index, refs, configuration, and
object database. Import the tree and blobs needed for the candidate into its
object database; source objects can be read during preparation but the completed
workspace must not depend on the source object database or alternate-index files.
This costs additional storage, accepted in v1 in exchange for simpler ownership
and an inspectable retained workspace. Missing locally available objects fail
setup clearly; do not silently fetch them into the original repository.

Use Git checkout plumbing with attributes taken from the captured tree.
`git archive` is unsuitable because export attributes can change its output.
Preserve effective source checkout settings needed for conversion, including
`filter.*`, `core.autocrlf`, `core.eol`, `core.safecrlf`,
`core.checkRoundtripEncoding`, `core.symlinks`, `core.fileMode`, and
`core.attributesFile`, plus applicable source `info/attributes`. Resolve external
attribute-file inputs for preparation consistently. Do not copy the entire
source config, hooks, remotes, worktree pointers, fsmonitor, or maintenance setup.
Global/system configuration remains a local input, with explicit private
overrides where needed for predictable repository ownership.

Required checkout filters must run successfully; do not substitute raw Git LFS
pointers or raw blobs after a filter failure. Available Git LFS/filter support
is a compatibility target, not a promise to initialize external tools or download
missing content automatically. Verify the clean baseline with the same filter
configuration so conversion-induced changes do not appear as user modifications.

Set detached HEAD to an unsigned synthetic **root commit** whose tree equals
the candidate tree. Use an internal git-commitfs author/committer identity;
do not require the user's Git identity and do not run commit hooks or signing.
`git status`, `git diff`, `git ls-files`, and ordinary local Git writes work in
the workspace. Original history, tags, branch names, remotes, and the eventual
real commit hash are not reproduced in v1. This is content equivalence with a
clean commit, not full history equivalence. No source HEAD is required.

Fail setup if the captured tree contains submodule gitlinks. Dirty submodule
working directories cannot stand in for committed submodule contents. Explicit
submodule checkout support is deferred; never silently omit their code.

## Including ignored files

When `--include-ignored` is absent, do not scan/copy ignored content at all.
When present, import existing source entries selected by the **captured tracked
`.gitignore` files**, the source `info/exclude`, and its configured global excludes.
Do not use unstaged or untracked `.gitignore` content as rules. Copy the local
exclude rules into private Git metadata so imported files remain ignored there.
Use Git to evaluate patterns rather than implement a new ignore engine.

Tracked snapshot paths always take priority. A source path tracked in the
captured source HEAD but absent from the candidate must not be reintroduced by
the ignored-file pass, even if it is newly ignored. Exclude incompatible
descendants of deleted tracked file/symlink paths too. Capture the HEAD tracked
path set alongside the candidate for this purpose; no HEAD means an empty set.
Test file/directory collisions and ignore negations explicitly.

Never import source `.git` metadata, including metadata inside ignored nested
repositories. Do not import ordinary untracked files or copy over existing
tracked workspace files. Preserve symlinks as symlinks and never follow them
while traversing/copying. Copy regular files to independent storage (or reflink
them); never hardlink writable workspace files to source files. Reject sockets,
devices, FIFOs, unreadable files, and copy errors with an actionable setup error.

Ignored files are local inputs, not an atomic snapshot of an active build
directory. Concurrent changes may yield a mixed copy or cause setup to fail.
There is no promise of an atomic snapshot of a concurrently written file either.
Do not run the command after a failed import. Copy cost can be large for build
outputs; the flag is deliberately opt-in, with selective imports deferred.

## Isolation boundary, environment, and process lifetime

This is working-copy isolation, **not an OS security sandbox**. Normal writes to
copied workspace files remain private. A command can still write through an
absolute path, an escaping symlink, a network service, or a variable such as
`CARGO_TARGET_DIR=/original/repo/target`. Inherit normal application environment
variables without guessing which paths to rewrite. Explain this limit in CLI
help and examples; relative `target/` writes stay disposable.

Discover using the source Git environment, then clear repository-routing Git
variables before invoking Git in the workspace or running the command. Use
Git's `rev-parse --local-env-vars` list plus explicit private overrides to avoid
inheriting source `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, object directories,
or equivalent repository-scoped settings. Preserve normal tool configuration;
do not silently erase unrelated environment variables.

Allocate temporary directories outside the source worktree so ignored-file
enumeration cannot recursively import the workspace. Per-run directories and
Git stores are unique; concurrent invocations share no mutable workspace state.

Supervise a distinct child process group. SIGINT, SIGTERM, and SIGHUP must end
the command group and allow cleanup. Preserve working foreground terminal input
when a controlling terminal exists; coordinate process-group ownership rather
than accidentally backgrounding interactive commands. Restore terminal ownership
on exit. On command completion, terminate remaining processes in its group before
removing the workspace, with a bounded grace period and forced termination if
necessary. Do not signal the caller's shell or hook process group. Processes
that deliberately detach from the group are outside the v1 lifetime guarantee.

No cleanup is guaranteed after SIGKILL, machine failure, or deliberately escaped
processes. Do not sweep unknown temporary directories automatically in v1.

## Future persistence and optimizations

Record a future explicit persistence switch, but do not ship an ambiguous
`--write-through` boolean in v1. It could mean publishing tracked edits, sharing
specific ignored caches, or running directly in the original checkout. These
have different semantics, especially with partially staged files. A later design
must specify destinations, conflict handling, and whether index changes persist.
No automatic writeback or implicit source-cache sharing is introduced now.

Possible subsequent strategies, without changing the snapshot contract:

- Reuse immutable prepared checkouts and provision independent workspace copies
  using reflinks or ordinary copies.
- Select OverlayFS on Linux where mounts and backing filesystems support it.
- Add a lazy Git-backed filesystem if measured workloads justify it.
- Validate macOS and use APFS clones where appropriate.

Checkout caches must include conversion/configuration inputs, not just tree IDs;
ignored content must not be assumed immutable because the staged tree is unchanged.
Compatible strategy pairs should be explicit: not every materializer can be
combined with every isolation mechanism. Backend selection must report the
chosen strategy and must not change visibility or persistence guarantees.

## Acceptance evidence and implementation sequence

First build a mount-free vertical slice: mixed staged/unstaged source, a child
command reading the staged version, command mutation, and unchanged source files,
index, and refs afterward. Extend it with private Git state and conversion
fidelity, ignored-file policies and copy strategies, then lifecycle/diagnostics.
This sequence is an implementation outline; the detailed task plan follows
design review.

Use integration tests that create disposable real Git repositories and execute
the built binary. Compare index bytes and source working files/refs before and
after both successful and failing commands. Required coverage includes:

- Partial staging, staged deletion/rename/type change, unstaged missing files,
  empty/unborn repositories, alternate indexes, linked worktrees, split indexes,
  sparse checkouts, SHA-256 repos, conflicts, and intent-to-add rejection.
- Executable bits, symlinks, binary/empty files, pathnames containing whitespace,
  newlines and non-UTF-8 bytes, staged versus unstaged attributes, export-ignore,
  line-ending conversion, a real test smudge/clean filter, and submodule rejection.
- Initially clean private Git state, source-independent retained workspaces,
  command-side Git edits, and absence of inherited source Git routing.
- Ignored-file opt-in, staged ignore rules/negations, local/global excludes,
  tracked collisions and staged deletions, independent writes under all copy
  modes, nested `.git` exclusion, symlinks and special-file rejection.
- Child status/argv/stdin, command-not-found, no child on setup failure, concurrent
  runs, retention policy, signals including grandchildren, terminal interaction,
  and cleanup failure without losing the original command result.

Measure cold preparation, repeated preparation, and one staged-file change on
a representative repository before adding checkout caches or filesystem mounts.
No v1 performance threshold or speedup claim is promised.

## Primary technical references

- [Git checkout-index](https://git-scm.com/docs/git-checkout-index)
- [Git read-tree](https://git-scm.com/docs/git-read-tree)
- [Git environment and attribute source](https://git-scm.com/docs/git)
- [Git hooks and repository environment](https://git-scm.com/docs/githooks)
- [Git attributes and checkout conversions](https://git-scm.com/docs/gitattributes)
- [Git ignore rules](https://git-scm.com/docs/gitignore)
- [Git commit-tree](https://git-scm.com/docs/git-commit-tree)
- [Linux reflinks](https://www.man7.org/linux/man-pages/man2/FICLONE.2const.html)
- [OverlayFS](https://docs.kernel.org/filesystems/overlayfs.html)
