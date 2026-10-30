use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};

use crate::{
    git::{ChildEnvironment, Git, quote_path, record},
    workspace::RunDirectory,
};

pub struct SourceRepository {
    pub root: PathBuf,
    pub index: PathBuf,
    pub objects: PathBuf,
    pub format: OsString,
    pub routing: Vec<OsString>,
    pub git: Git,
}

impl SourceRepository {
    pub fn discover(
        cwd: &Path,
        supervisor: std::sync::Arc<crate::process::Supervisor>,
    ) -> Result<Self> {
        let git = Git::source(cwd, supervisor);
        let version = git.output(&["--version"])?;
        let version = String::from_utf8_lossy(&version);
        let mut numbers = version
            .trim()
            .strip_prefix("git version ")
            .unwrap_or("")
            .split('.');
        let major = numbers
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let minor = numbers
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        ensure!(
            (major, minor) >= (2, 43),
            "Git >= 2.43 is required; found {}",
            version.trim()
        );
        let root = PathBuf::from(record(
            git.output(&["rev-parse", "--show-toplevel"])
                .context("not inside a non-bare Git working tree")?,
        ));
        let index = PathBuf::from(record(git.output(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "index",
        ])?));
        let objects = PathBuf::from(record(git.output(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ])?));
        let format = record(git.output(&["rev-parse", "--show-object-format"])?);
        let routing = git
            .output(&["rev-parse", "--local-env-vars"])?
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
            .map(|s| OsString::from(String::from_utf8_lossy(s).into_owned()))
            .collect();
        Ok(Self {
            root,
            index,
            objects,
            format,
            routing,
            git,
        })
    }

    pub fn child_environment(&self) -> ChildEnvironment {
        ChildEnvironment {
            remove: self.routing.clone(),
            set: Vec::new(),
        }
    }
}

pub struct Candidate {
    pub tree: OsString,
    pub git: Git,
    pub checkout: CheckoutSettings,
    pub tracked: HashSet<PathBuf>,
    pub head_tracked: HashSet<PathBuf>,
    pub ignores: IgnoreSettings,
}

pub struct IgnoreSettings {
    pub info_exclude: Option<Vec<u8>>,
    pub global_exclude: Option<Vec<u8>>,
}

pub struct CheckoutSettings {
    pub config: Vec<(OsString, OsString)>,
    pub info_attributes: Option<Vec<u8>>,
    pub external_attributes: Option<Vec<u8>>,
}

pub fn capture(source: &SourceRepository, run: &RunDirectory) -> Result<Candidate> {
    let git = Git::private(
        run.checkout(),
        &source.routing,
        std::sync::Arc::clone(&source.git.supervisor),
    );
    let sparse_rules = initialize_private(&git, &source.format)?;
    let private_index = run.checkout().join(".git/index");
    match File::open(&source.index) {
        Ok(original) => copy_index(original, &private_index)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            git.output(&["read-tree", "--empty"])?;
        }
        Err(error) => return Err(error).context("read effective Git index"),
    }
    let mut alternate = quote_path(source.objects.as_os_str());
    if let Some(extra) = std::env::var_os("GIT_ALTERNATE_OBJECT_DIRECTORIES") {
        alternate.push(":");
        alternate.push(extra);
    }
    let mut borrowed = git
        .with_env("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternate)
        .with_env("GIT_DIR", run.checkout().join(".git"))
        .with_env("GIT_WORK_TREE", &source.root);
    // Git resolves relative environment alternates from the source worktree root.
    // Only capture plumbing uses this cwd; index, objects, and config remain private.
    borrowed.cwd.clone_from(&source.root);
    let shared_indexes = copy_split_dependencies(source, &private_index)?;
    let head_tracked =
        match source
            .git
            .optional_output(&["rev-parse", "--verify", "--quiet", "HEAD"])?
        {
            Some(head) => paths(&borrowed.os_output(&[
                "ls-tree".as_ref(),
                "-r".as_ref(),
                "--name-only".as_ref(),
                "-z".as_ref(),
                &record(head),
            ])?),
            None => HashSet::new(),
        };
    let pack = run.scratch().join("candidate.pack");
    // Sparse expansion can write trees and freshen objects in alternates even
    // in read-only Git commands. Import sparse trees and blobs without expansion,
    // then do all validation/normalization against independent object storage.
    let entries = borrowed.output(&["ls-files", "--stage", "--sparse", "-z"])?;
    borrowed.pack(staged_roots(&entries)?, &pack)?;
    git.import_pack(&pack)?;
    fs::remove_file(pack)?;
    for key in [
        "core.sparseCheckout",
        "core.sparseCheckoutCone",
        "index.sparse",
    ] {
        git.output(&["config", key, "false"])?;
    }
    git.output(&["config", "--unset", "sparse.expectFilesOutsideOfPatterns"])?;
    fs::remove_file(sparse_rules)?;
    validate_index(&git)?;
    // Rebuild without the copied index's split/sparse/cache-tree extensions.
    // This forces tree creation in our own object store, with no source alternates.
    let entries = git.output(&["ls-files", "--stage", "-z"])?;
    fs::remove_file(&private_index)?;
    git.output(&["read-tree", "--empty"])?;
    crate::git::checked(
        git.input_output(&["update-index", "-z", "--index-info"], entries)?,
        "update-index",
    )?;
    for path in shared_indexes {
        fs::remove_file(path)?;
    }
    let tree = record(git.output(&["write-tree"])?);
    let tracked = paths(&git.output(&["ls-files", "-z"])?);
    let checkout = checkout_settings(source)?;
    let info = PathBuf::from(record(source.git.output(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "info/exclude",
    ])?));
    let ignores = IgnoreSettings {
        info_exclude: read_optional(&info)?,
        global_exclude: external_rules(source, "core.excludesFile", "ignore")?,
    };
    Ok(Candidate {
        tree,
        git,
        checkout,
        tracked,
        head_tracked,
        ignores,
    })
}

fn staged_roots(entries: &[u8]) -> Result<Vec<u8>> {
    let mut roots = Vec::new();
    for entry in entries.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
        let header = entry
            .split(|b| *b == b'\t')
            .next()
            .context("staged entry header")?;
        ensure!(
            header.ends_with(b" 0"),
            "resolve index merge conflicts before running git-commitfs"
        );
        ensure!(
            !header.starts_with(b"160000 "),
            "submodule gitlinks are unsupported; cannot substitute a dirty submodule checkout"
        );
        let oid = entry
            .split(|b| *b == b' ')
            .nth(1)
            .context("staged object ID")?;
        roots.extend_from_slice(oid);
        roots.push(b'\n');
    }
    Ok(roots)
}

fn initialize_private(git: &Git, format: &OsStr) -> Result<PathBuf> {
    git.os_output(&[
        "init".as_ref(),
        "--quiet".as_ref(),
        "--template=".as_ref(),
        "--object-format".as_ref(),
        format,
    ])?;
    for (key, value) in [
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("gc.auto", "0"),
        ("maintenance.auto", "false"),
        ("core.sparseCheckout", "true"),
        ("core.sparseCheckoutCone", "true"),
        ("index.sparse", "true"),
        ("sparse.expectFilesOutsideOfPatterns", "true"),
    ] {
        git.output(&["config", key, value])?;
    }
    // Keep sparse directory entries collapsed while source objects are borrowed.
    // Git requires cone patterns as well as config, even for `ls-files --sparse`.
    let sparse_rules = git.cwd.join(".git/info/sparse-checkout");
    fs::create_dir_all(
        sparse_rules
            .parent()
            .context("private Git info directory")?,
    )?;
    fs::write(&sparse_rules, b"/*\n!/*/\n")?;
    Ok(sparse_rules)
}

fn copy_index(mut original: File, destination: &Path) -> Result<()> {
    // An atomic pathname replacement by concurrent staging cannot change this opened file.
    io::copy(&mut original, &mut File::create(destination)?)?;
    Ok(())
}

fn paths(data: &[u8]) -> HashSet<PathBuf> {
    data.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(OsString::from_vec(p.to_vec())))
        .collect()
}

fn checkout_settings(source: &SourceRepository) -> Result<CheckoutSettings> {
    let values = source
        .git
        .optional_output(&[
            "config",
            "--null",
            "--get-regexp",
            "^(filter\\..*|core\\.\
             (autocrlf|eol|safecrlf|checkroundtripencoding|symlinks|filemode))$",
        ])?
        .unwrap_or_default();
    let mut config = Vec::new();
    for record in values.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let separator = record
            .iter()
            .position(|b| *b == b'\n')
            .context("malformed Git configuration record")?;
        config.push((
            OsString::from_vec(record[..separator].to_vec()),
            OsString::from_vec(record[separator + 1..].to_vec()),
        ));
    }
    let info = PathBuf::from(record(source.git.output(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        "info/attributes",
    ])?));
    Ok(CheckoutSettings {
        config,
        info_attributes: read_optional(&info)?,
        external_attributes: external_rules(source, "core.attributesFile", "attributes")?,
    })
}

pub fn external_rules(
    source: &SourceRepository,
    key: &str,
    default_file: &str,
) -> Result<Option<Vec<u8>>> {
    let path = if let Some(value) = source
        .git
        .optional_output(&["config", "--path", "--get", key])?
    {
        let path = PathBuf::from(record(value));
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        if path.is_absolute() {
            path
        } else {
            source.root.join(path)
        }
    } else {
        let config_root = std::env::var_os("XDG_CONFIG_HOME").map_or_else(
            || std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")),
            |dir| Some(PathBuf::from(dir)),
        );
        let Some(root) = config_root else {
            return Ok(None);
        };
        root.join("git").join(default_file)
    };
    read_optional(&path)
}

pub fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read Git rules from {}", path.display())),
    }
}

fn copy_split_dependencies(source: &SourceRepository, index: &Path) -> Result<Vec<PathBuf>> {
    // Even `rev-parse --shared-index-path` refreshes the shared index's mtime.
    // Copy possible dependencies first, then let Git interpret the index privately.
    // Git prefers its gitdir, with the effective index's directory as fallback.
    let gitdir = PathBuf::from(record(
        source.git.output(&["rev-parse", "--absolute-git-dir"])?,
    ));
    let parent = source.index.parent().context("source index parent")?;
    let destination = index.parent().context("private index parent")?;
    let mut copied = Vec::new();
    for directory in [gitdir.as_path(), parent] {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("read shared-index dependency directory"),
        };
        for entry in entries {
            source.git.supervisor.check()?;
            let entry = entry?;
            if !entry
                .file_name()
                .as_encoded_bytes()
                .starts_with(b"sharedindex.")
            {
                continue;
            }
            let target = destination.join(entry.file_name());
            if !target.exists() {
                fs::copy(entry.path(), &target).context("copy shared Git index dependency")?;
                copied.push(target);
            }
        }
    }
    Ok(copied)
}

fn validate_index(git: &Git) -> Result<()> {
    ensure!(
        git.output(&["ls-files", "--unmerged", "-z"])?.is_empty(),
        "resolve index merge conflicts before running git-commitfs"
    );
    let visible = git.output(&[
        "diff",
        "--cached",
        "--ita-visible-in-index",
        "--name-only",
        "-z",
    ])?;
    let invisible = git.output(&[
        "diff",
        "--cached",
        "--ita-invisible-in-index",
        "--name-only",
        "-z",
    ])?;
    ensure!(
        visible == invisible,
        "intent-to-add entries are unsupported; stage their contents or remove them from the index"
    );
    let entries = git.output(&["ls-files", "--stage", "-z"])?;
    ensure!(
        !entries
            .split(|b| *b == 0)
            .any(|entry| entry.starts_with(b"160000 ")),
        "submodule gitlinks are unsupported; cannot substitute a dirty submodule checkout"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_capture_reads_the_open_handle_after_pathname_replacement() {
        let dir = tempfile::tempdir().expect("index fixture");
        let index = dir.path().join("index");
        let replacement = dir.path().join("index.lock");
        let captured = dir.path().join("captured");
        fs::write(&index, b"original complete index").expect("original index");
        let opened = File::open(&index).expect("open index");
        fs::write(&replacement, b"replacement index").expect("replacement index");
        fs::rename(replacement, &index).expect("atomic replacement");
        copy_index(opened, &captured).expect("capture opened index");
        assert_eq!(
            fs::read(captured).expect("captured index"),
            b"original complete index"
        );
        assert_eq!(
            fs::read(index).expect("current index"),
            b"replacement index"
        );
    }
}
