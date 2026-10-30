use std::{
    ffi::OsString,
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
    pub fn discover(cwd: &Path) -> Result<Self> {
        let git = Git::source(cwd);
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
}

pub struct CheckoutSettings {
    pub config: Vec<(OsString, OsString)>,
    pub info_attributes: Option<Vec<u8>>,
    pub external_attributes: Option<Vec<u8>>,
}

pub fn capture(source: &SourceRepository, run: &RunDirectory) -> Result<Candidate> {
    let git = Git::private(run.checkout(), &source.routing);
    git.os_output(&[
        "init".as_ref(),
        "--quiet".as_ref(),
        "--template=".as_ref(),
        "--object-format".as_ref(),
        &source.format,
    ])?;
    for (key, value) in [
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("gc.auto", "0"),
        ("maintenance.auto", "false"),
    ] {
        git.output(&["config", key, value])?;
    }
    let private_index = run.checkout().join(".git/index");
    match File::open(&source.index) {
        Ok(mut original) => {
            io::copy(&mut original, &mut File::create(&private_index)?)?;
        }
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
    let borrowed = git.with_env("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternate);
    copy_split_dependency(source, &private_index)?;
    borrowed.output(&["update-index", "--no-split-index"])?;
    validate_index(&borrowed)?;
    let tree = record(borrowed.output(&["write-tree"])?);
    let pack = run.scratch().join("candidate.pack");
    borrowed.pack(&tree, &pack)?;
    git.import_pack(&pack)?;
    fs::remove_file(pack)?;
    let checkout = checkout_settings(source)?;
    Ok(Candidate {
        tree,
        git,
        checkout,
    })
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

fn copy_split_dependency(source: &SourceRepository, index: &Path) -> Result<()> {
    let query = source.git.with_env("GIT_INDEX_FILE", index.as_os_str());
    let shared =
        record(query.output(&["rev-parse", "--path-format=absolute", "--shared-index-path"])?);
    if !shared.is_empty() {
        let path = PathBuf::from(shared);
        let destination = index
            .parent()
            .context("private index parent")?
            .join(path.file_name().context("shared index filename")?);
        fs::copy(&path, destination).context("copy shared Git index dependency")?;
    }
    Ok(())
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
