use anyhow::{bail, Context, Result};
use arc_swap::ArcSwap;
use gix::filter::plumbing::driver::apply::Delay;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use gix::bstr::ByteSlice;
use gix::diff::Rewrites;
use gix::dir::entry::Status;
use gix::objs::tree::EntryKind;
use gix::repository::blame_file::Options as BlameOptions;
use gix::sec::trust::DefaultForLevel;
use gix::status::{
    index_worktree::Item,
    plumbing::index_as_worktree::{Change, EntryStatus},
    UntrackedFiles,
};
use gix::{Commit, ObjectId, Repository, ThreadSafeRepository};

use crate::blame::BlameHunk;
use crate::{FileBlame, FileChange, LineBlame, WorkingTreeStatus};

#[cfg(test)]
mod test;

#[inline]
fn get_repo_dir(file: &Path) -> Result<&Path> {
    file.parent().context("file has no parent directory")
}

pub fn get_diff_base(file: &Path, trust_full: bool) -> Result<Vec<u8>> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());
    let file = gix::path::realpath(file).context("resolve symlinks")?;

    // TODO cache repository lookup

    let repo_dir = get_repo_dir(&file)?;
    let repo = open_repo(repo_dir, trust_full)
        .context("failed to open git repo")?
        .to_thread_local();
    let head = repo.head_commit()?;
    let file_oid = find_file_in_commit(&repo, &head, &file)?;

    let file_object = repo.find_object(file_oid)?;
    let data = file_object.detach().data;
    // Get the actual data that git would make out of the git object.
    // This will apply the user's git config or attributes like crlf conversions.
    //
    // The whole filter pipeline still runs in untrusted (`Trust::Reduced`) mode so built-in
    // conversions like autocrlf keep working, but gix drops `filter.*.clean` / `filter.*.smudge`
    // drivers defined in untrusted (repository-local) config, so those external programs are not
    // executed unless the workspace was explicitly trusted. This relies on `open_repo` forcing the
    // trust level instead of letting gix re-derive it from `.git` ownership; see the note there.
    if let Some(work_dir) = repo.workdir() {
        let rela_path = file.strip_prefix(work_dir)?;
        let rela_path = gix::path::try_into_bstr(rela_path)?;
        let (mut pipeline, _) = repo.filter_pipeline(None)?;
        let mut worktree_outcome =
            pipeline.convert_to_worktree(&data, rela_path.as_ref(), Delay::Forbid)?;
        let mut buf = Vec::with_capacity(data.len());
        worktree_outcome.read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        Ok(data)
    }
}

pub fn blame_file(file: &Path, trust_full: bool) -> Result<FileBlame> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());
    let file = gix::path::realpath(file).context("resolve symlinks")?;

    let repo_dir = get_repo_dir(&file)?;
    let repo = open_repo(repo_dir, trust_full)
        .context("failed to open git repo")?
        .to_thread_local();
    let head = repo.head_commit()?;

    let work_dir = repo.workdir().context("repo has no worktree")?;
    let rela_path = file.strip_prefix(work_dir)?;
    let rela_path = gix::path::try_into_bstr(rela_path)?;

    let outcome = repo.blame_file(rela_path.as_ref(), head.id, BlameOptions::default())?;
    file_blame_from_outcome(&repo, outcome)
}

fn file_blame_from_outcome(repo: &Repository, outcome: gix::blame::Outcome) -> Result<FileBlame> {
    let mut blames_by_commit = HashMap::new();
    let mut hunks = Vec::with_capacity(outcome.entries.len());

    for entry in outcome.entries {
        let blame = match blames_by_commit.get(&entry.commit_id) {
            Some(blame) => Arc::clone(blame),
            None => {
                let blame = Arc::new(line_blame_for_commit(repo, entry.commit_id)?);
                blames_by_commit.insert(entry.commit_id, Arc::clone(&blame));
                blame
            }
        };
        let start = entry.start_in_blamed_file;
        hunks.push(BlameHunk::new(start..start + entry.len.get(), blame));
    }

    Ok(FileBlame::new(hunks))
}

fn line_blame_for_commit(repo: &Repository, commit_id: ObjectId) -> Result<LineBlame> {
    let commit = repo.find_commit(commit_id)?;
    let author = commit.author()?;

    Ok(LineBlame {
        commit: commit_id.to_string(),
        author: author.name.to_string(),
        timestamp: author.seconds(),
        message: commit.message()?.summary().to_string(),
    })
}

const MERGE_LOOKUP_TIME_SLACK_SECONDS: i64 = 60 * 60 * 24;

pub fn merge_message(file: &Path, trust_full: bool, commit: &str) -> Result<Option<String>> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());
    let file = gix::path::realpath(file).context("resolve symlinks")?;

    let repo_dir = get_repo_dir(&file)?;
    let repo = open_repo(repo_dir, trust_full)
        .context("failed to open git repo")?
        .to_thread_local();
    let commit_id = ObjectId::from_hex(commit.as_bytes())?;

    match find_merging_commit(&repo, commit_id)? {
        Some(merge_id) => Ok(Some(repo.find_commit(merge_id)?.message_raw()?.to_string())),
        None => Ok(None),
    }
}

fn find_merging_commit(repo: &Repository, target: ObjectId) -> Result<Option<ObjectId>> {
    let cutoff = repo.find_commit(target)?.time()?.seconds - MERGE_LOOKUP_TIME_SLACK_SECONDS;

    let mut first_parent_chain = Vec::new();
    let walk = repo
        .rev_walk([repo.head_commit()?.id])
        .first_parent_only()
        .sorting(gix::revision::walk::Sorting::ByCommitTimeCutoff {
            order: Default::default(),
            seconds: cutoff,
        })
        .all()?;
    for info in walk {
        let info = info?;
        if info.id == target {
            return Ok(None);
        }
        first_parent_chain.push(info.id);
    }

    let Some(&head) = first_parent_chain.first() else {
        return Ok(None);
    };
    if !is_ancestor(repo, target, head) {
        return Ok(None);
    }

    let (mut newest, mut oldest) = (0, first_parent_chain.len() - 1);
    while newest < oldest {
        let middle = (newest + oldest + 1) / 2;
        if is_ancestor(repo, target, first_parent_chain[middle]) {
            newest = middle;
        } else {
            oldest = middle - 1;
        }
    }

    let merge_id = first_parent_chain[newest];
    let is_merge = repo.find_commit(merge_id)?.parent_ids().count() > 1;
    Ok(is_merge.then_some(merge_id))
}

fn is_ancestor(repo: &Repository, ancestor: ObjectId, descendant: ObjectId) -> bool {
    repo.merge_base(ancestor, descendant)
        .map(|base| base.detach() == ancestor)
        .unwrap_or(false)
}

pub fn remote_url(file: &Path, trust_full: bool) -> Result<String> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());
    let file = gix::path::realpath(file).context("resolve symlinks")?;

    let repo_dir = get_repo_dir(&file)?;
    let repo = open_repo(repo_dir, trust_full)
        .context("failed to open git repo")?
        .to_thread_local();
    let remote = repo
        .find_default_remote(gix::remote::Direction::Fetch)
        .context("no default remote")??;
    let url = remote
        .url(gix::remote::Direction::Fetch)
        .context("remote has no url")?;

    Ok(url.to_bstring().to_string())
}

pub fn get_current_head_name(file: &Path, trust_full: bool) -> Result<Arc<ArcSwap<Box<str>>>> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());
    let file = gix::path::realpath(file).context("resolve symlinks")?;

    let repo_dir = get_repo_dir(&file)?;
    let repo = open_repo(repo_dir, trust_full)
        .context("failed to open git repo")?
        .to_thread_local();
    let head_ref = repo.head_ref()?;
    let head_commit = repo.head_commit()?;

    let name = match head_ref {
        Some(reference) => reference.name().shorten().to_string(),
        None => head_commit.id.to_hex_with_len(8).to_string(),
    };

    Ok(Arc::new(ArcSwap::from_pointee(name.into_boxed_str())))
}

pub fn for_each_changed_file(
    cwd: &Path,
    trust_full: bool,
    f: impl Fn(Result<FileChange>) -> bool,
) -> Result<()> {
    status(&open_repo(cwd, trust_full)?.to_thread_local(), f)
}

pub fn working_tree_status(cwd: &Path, trust_full: bool) -> Result<WorkingTreeStatus> {
    status_with_staged(&open_repo(cwd, trust_full)?.to_thread_local())
}

fn open_repo(path: &Path, trust_full: bool) -> Result<ThreadSafeRepository> {
    // `trust_full` is the workspace-trust decision made by the caller, and it must be the
    // authority on the gix trust level. gix's own discovery (`discover_*`) ignores a
    // caller-supplied trust level: it always re-derives trust from `.git` ownership, so a malicious
    // `.git/config` in a user-owned directory would be opened as `Trust::Full` regardless of our
    // gate. Worse, the GIT_DIR-environment branch of that discovery panics because it never sets a
    // trust level at all. So we split discovery from opening: find the repository path ourselves,
    // then `open_opts(..).with(trust)`, which forces the trust level and skips gix's ownership
    // check. Under `Trust::Reduced`, gix then refuses to honor untrusted repository-local config
    // such as `filter.*` smudge/clean drivers.

    let trust = if trust_full {
        gix::sec::Trust::Full
    } else {
        gix::sec::Trust::Reduced
    };

    // On Windows various configuration options are bundled as part of the git installation. The
    // lookup is expensive; only do it there.
    let config = gix::open::permissions::Config {
        system: true,
        git: true,
        user: true,
        env: true,
        includes: true,
        git_binary: cfg!(windows),
    };

    let permissions = gix::open::Permissions {
        config,
        ..gix::open::Permissions::default_for_level(trust)
    };

    let discover_options = gix::discover::upwards::Options {
        dot_git_only: true,
        ..Default::default()
    };
    let (repo_path, _trust_from_ownership) = gix::discover::upwards_opts(path, discover_options)
        .context("failed to discover git repo")?;
    let (git_dir, _work_dir) = repo_path.into_repository_and_work_tree_directories();

    let options = gix::open::Options::default()
        .permissions(permissions)
        // `git_dir` is the discovered `.git` directory (or a linked-worktree git dir), so open it
        // as-is rather than letting gix append `.git` again.
        .open_path_as_is(true)
        .with(trust);

    Ok(ThreadSafeRepository::open_opts(git_dir, options)?)
}

/// Emulates the result of running `git status` from the command line.
fn status(repo: &Repository, f: impl Fn(Result<FileChange>) -> bool) -> Result<()> {
    let work_dir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("working tree not found"))?
        .to_path_buf();

    let status_platform = repo
        .status(gix::progress::Discard)?
        // Here we discard the `status.showUntrackedFiles` config, as it makes little sense in
        // our case to not list new (untracked) files. We could have respected this config
        // if the default value weren't `Collapsed` though, as this default value would render
        // the feature unusable to many.
        .untracked_files(UntrackedFiles::Files)
        // Turn on file rename detection, which is off by default.
        .index_worktree_rewrites(Some(Rewrites {
            copies: None,
            percentage: Some(0.5),
            limit: 1000,
            ..Default::default()
        }));

    // No filtering based on path
    let empty_patterns = vec![];

    let status_iter = status_platform.into_index_worktree_iter(empty_patterns)?;

    for item in status_iter {
        let Ok(item) = item.map_err(|err| f(Err(err.into()))) else {
            continue;
        };
        let Some(change) = index_worktree_change(&work_dir, item)? else {
            continue;
        };
        if !f(Ok(change)) {
            break;
        }
    }

    Ok(())
}

fn index_worktree_change(work_dir: &Path, item: Item) -> Result<Option<FileChange>> {
    let change = match item {
        Item::Modification {
            rela_path, status, ..
        } => {
            let path = work_dir.join(rela_path.to_path()?);
            match status {
                EntryStatus::Conflict { .. } => FileChange::Conflict { path },
                EntryStatus::Change(Change::Removed) => FileChange::Deleted { path },
                EntryStatus::Change(Change::Modification { .. }) => FileChange::Modified { path },
                // Files marked with `git add --intent-to-add`. Such files
                // still show up as new in `git status`, so it's appropriate
                // to show them the same way as untracked files in the
                // "changed file" picker. One example of this being used
                // is Jujutsu, a Git-compatible VCS. It marks all new files
                // with `--intent-to-add` automatically.
                EntryStatus::IntentToAdd => FileChange::Untracked { path },
                _ => return Ok(None),
            }
        }
        Item::DirectoryContents { entry, .. } if entry.status == Status::Untracked => {
            FileChange::Untracked {
                path: work_dir.join(entry.rela_path.to_path()?),
            }
        }
        Item::Rewrite {
            source,
            dirwalk_entry,
            ..
        } => FileChange::Renamed {
            from_path: work_dir.join(source.rela_path().to_path()?),
            to_path: work_dir.join(dirwalk_entry.rela_path.to_path()?),
        },
        _ => return Ok(None),
    };
    Ok(Some(change))
}

fn status_with_staged(repo: &Repository) -> Result<WorkingTreeStatus> {
    let work_dir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("working tree not found"))?
        .to_path_buf();

    let rewrites = Rewrites {
        copies: None,
        percentage: Some(0.5),
        limit: 1000,
        ..Default::default()
    };

    let status_platform = repo
        .status(gix::progress::Discard)?
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_rewrites(Some(rewrites))
        .head_tree(repo.head_tree_id_or_empty()?.detach())
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Given(rewrites));

    let empty_patterns = Vec::<gix::bstr::BString>::new();

    let mut result = WorkingTreeStatus::default();
    for item in status_platform.into_iter(empty_patterns)? {
        let Ok(item) = item else {
            continue;
        };
        match item {
            gix::status::Item::IndexWorktree(item) => {
                if let Some(change) = index_worktree_change(&work_dir, item)? {
                    result.unstaged.push(change);
                }
            }
            gix::status::Item::TreeIndex(change) => {
                if let Some(change) = tree_index_change(&work_dir, change)? {
                    result.staged.push(change);
                }
            }
        }
    }

    Ok(result)
}

fn tree_index_change(
    work_dir: &Path,
    change: gix::diff::index::Change,
) -> Result<Option<FileChange>> {
    use gix::diff::index::ChangeRef;

    let change = match change {
        ChangeRef::Addition { location, .. } => FileChange::Untracked {
            path: work_dir.join(location.to_path()?),
        },
        ChangeRef::Deletion { location, .. } => FileChange::Deleted {
            path: work_dir.join(location.to_path()?),
        },
        ChangeRef::Modification { location, .. } => FileChange::Modified {
            path: work_dir.join(location.to_path()?),
        },
        ChangeRef::Rewrite {
            source_location,
            location,
            ..
        } => FileChange::Renamed {
            from_path: work_dir.join(source_location.to_path()?),
            to_path: work_dir.join(location.to_path()?),
        },
    };
    Ok(Some(change))
}

/// Finds the object that contains the contents of a file at a specific commit.
fn find_file_in_commit(repo: &Repository, commit: &Commit, file: &Path) -> Result<ObjectId> {
    let repo_dir = repo.workdir().context("repo has no worktree")?;
    let rel_path = file.strip_prefix(repo_dir)?;
    let tree = commit.tree()?;
    let tree_entry = tree
        .lookup_entry_by_path(rel_path)?
        .context("file is untracked")?;
    match tree_entry.mode().kind() {
        // not a file, everything is new, do not show diff
        mode @ (EntryKind::Tree | EntryKind::Commit | EntryKind::Link) => {
            bail!("entry at {} is not a file but a {mode:?}", file.display())
        }
        // found a file
        EntryKind::Blob | EntryKind::BlobExecutable => Ok(tree_entry.object_id()),
    }
}
