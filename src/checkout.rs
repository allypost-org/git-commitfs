use std::fs;

use anyhow::{Context, Result};

use crate::{snapshot::Candidate, workspace::RunDirectory};

pub fn materialize(candidate: &Candidate, run: &RunDirectory) -> Result<()> {
    let git = &candidate.git;
    for (key, value) in &candidate.checkout.config {
        git.os_output(&["config".as_ref(), "--add".as_ref(), key, value])?;
    }
    let info = run.checkout().join(".git/info");
    fs::create_dir_all(&info).context("create private Git attribute storage")?;
    fs::write(
        info.join("attributes"),
        candidate
            .checkout
            .info_attributes
            .as_deref()
            .unwrap_or_default(),
    )?;
    let external = info.join("commitfs-attributes");
    fs::write(
        &external,
        candidate
            .checkout
            .external_attributes
            .as_deref()
            .unwrap_or_default(),
    )?;
    git.os_output(&[
        "config".as_ref(),
        "core.attributesFile".as_ref(),
        external.as_os_str(),
    ])?;
    git.os_output(&["read-tree".as_ref(), &candidate.tree])?;
    fs::write(
        info.join("exclude"),
        candidate
            .ignores
            .info_exclude
            .as_deref()
            .unwrap_or_default(),
    )?;
    let global_exclude = info.join("commitfs-ignore");
    fs::write(
        &global_exclude,
        candidate
            .ignores
            .global_exclude
            .as_deref()
            .unwrap_or_default(),
    )?;
    git.os_output(&[
        "config".as_ref(),
        "core.excludesFile".as_ref(),
        global_exclude.as_os_str(),
    ])?;
    git.with_env("GIT_ATTR_SOURCE", candidate.tree.clone())
        .output(&[
            "checkout-index",
            "--all",
            "--force",
            "--index",
            "--ignore-skip-worktree-bits",
        ])?;
    let identity = git
        .with_env("GIT_AUTHOR_NAME", "git-commitfs")
        .with_env("GIT_AUTHOR_EMAIL", "git-commitfs@localhost")
        .with_env("GIT_COMMITTER_NAME", "git-commitfs")
        .with_env("GIT_COMMITTER_EMAIL", "git-commitfs@localhost")
        .with_env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00+00:00")
        .with_env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00+00:00");
    let commit = crate::git::record(identity.os_output(&[
        "commit-tree".as_ref(),
        &candidate.tree,
        "-m".as_ref(),
        "git-commitfs staged snapshot".as_ref(),
    ])?);
    git.os_output(&[
        "update-ref".as_ref(),
        "--no-deref".as_ref(),
        "HEAD".as_ref(),
        &commit,
    ])?;
    git.output(&["diff", "--quiet"])
        .context("checkout filters did not produce a clean staged baseline")?;
    Ok(())
}
