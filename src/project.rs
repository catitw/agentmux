//! Project → branch → session grouping, derived from each session's working
//! directory. Pure file-based: no `git` processes are spawned and no new
//! dependencies are needed. The classification is derived data — the
//! persisted session schema (`work_dir`) is unchanged.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often a project root's HEAD is re-read (branch flips without a cwd
/// change, e.g. `git checkout`).
const HEAD_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// A session's classification at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    /// Absolute project root: the repo root for git projects, else the cwd
    /// itself (standalone directory project).
    pub root: PathBuf,
    /// Display name: basename of `root`.
    pub name: String,
    /// Current branch/ref when the project is a git repo and HEAD parses
    /// (detached HEAD → short sha); `None` otherwise.
    pub branch: Option<String>,
}

/// Per-project HEAD read cache (branch refresh cadence).
#[derive(Debug, Clone)]
struct HeadCacheEntry {
    branch: Option<String>,
    read_at: Instant,
}

/// Classifies session cwds into projects/branches, caching HEAD reads.
#[derive(Default)]
pub struct ProjectClassifier {
    heads: HashMap<PathBuf, HeadCacheEntry>,
}

impl ProjectClassifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify a session's working directory. `now` drives the HEAD
    /// re-read cadence; a changed cwd re-classifies immediately.
    pub fn classify(&mut self, cwd: &Path, now: Instant) -> ProjectInfo {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let root = find_project_root(&cwd);
        let branch = self.read_branch(&root, now);
        ProjectInfo {
            name: root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string()),
            root,
            branch,
        }
    }

    /// Read the project's branch, honoring the HEAD cache cadence.
    fn read_branch(&mut self, root: &Path, now: Instant) -> Option<String> {
        if let Some(entry) = self.heads.get(root)
            && now.duration_since(entry.read_at) < HEAD_REFRESH_INTERVAL
        {
            return entry.branch.clone();
        }
        let branch = read_head_branch(root);
        self.heads.insert(
            root.to_path_buf(),
            HeadCacheEntry {
                branch: branch.clone(),
                read_at: now,
            },
        );
        branch
    }
}

/// Walk up from `cwd` looking for `.git` (a directory OR a file — worktrees
/// have a `.git` FILE). Returns the repo root, or `cwd` itself when no git
/// project is found (standalone directory project).
pub fn find_project_root(cwd: &Path) -> PathBuf {
    let mut dir = Some(cwd);
    while let Some(dir_path) = dir {
        if dir_path.join(".git").exists() {
            return dir_path.to_path_buf();
        }
        dir = dir_path.parent();
    }
    cwd.to_path_buf()
}

/// Parse the current branch/ref of a repo root WITHOUT spawning git:
/// - normal repo: `<root>/.git/HEAD` → `ref: refs/heads/<branch>`;
/// - linked worktree: `<root>/.git` is a file with `gitdir: <path>` (relative
///   to `root`), then `<path>/HEAD`;
/// - detached HEAD: the 40-hex sha, shortened to 7 characters;
/// - unparseable/missing → `None`.
pub fn read_head_branch(root: &Path) -> Option<String> {
    let git_path = root.join(".git");
    let head_path = if git_path.is_dir() {
        git_path.join("HEAD")
    } else if git_path.is_file() {
        let content = std::fs::read_to_string(&git_path).ok()?;
        let line = content.lines().next()?.trim();
        let gitdir = line.strip_prefix("gitdir:")?.trim();
        // The gitdir path is relative to the directory containing the .git
        // file, i.e. the repo root.
        root.join(gitdir).join("HEAD")
    } else {
        return None;
    };

    let head = std::fs::read_to_string(head_path).ok()?;
    let line = head.lines().next()?.trim();
    if let Some(branch) = line.strip_prefix("ref: refs/heads/") {
        let branch = branch.trim();
        (!branch.is_empty()).then(|| branch.to_owned())
    } else if line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(line[..7].to_owned())
    } else {
        None
    }
}

/// Live working directory of a session's shell process. On Linux the PTY
/// child's cwd is read via `/proc/<pid>/cwd` (one readlink); on macOS via
/// libproc's `proc_pidinfo(PROC_PIDVNODEPATHINFO)` — both are a single
/// syscall, so `cd` in the terminal re-classifies the session. Windows has
/// no sane API for another process's cwd → `None`, and the caller falls
/// back to the spawn work_dir (caveat documented in docs/phase5-grouping.md).
#[cfg(target_os = "linux")]
pub fn live_cwd(shell_pid: u32) -> Option<PathBuf> {
    if shell_pid == 0 {
        return None;
    }
    std::fs::read_link(format!("/proc/{shell_pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
pub fn live_cwd(shell_pid: u32) -> Option<PathBuf> {
    if shell_pid == 0 {
        return None;
    }
    let mut info = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::uninit();
    // SAFETY: proc_pidinfo fills the vnode-path buffer (fixed MAXPATHLEN);
    // the buffer is a valid writable region of the right size.
    let ret = unsafe {
        libc::proc_pidinfo(
            shell_pid as libc::pid_t,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr() as *mut libc::c_void,
            std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int,
        )
    };
    if ret <= 0 {
        return None;
    }
    // SAFETY: ret > 0 means proc_pidinfo wrote the struct.
    let info = unsafe { info.assume_init() };
    // SAFETY: vip_path is a NUL-terminated C string at the start of the
    // pvi_cdir field (the array's first element address).
    let path = unsafe {
        std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr() as *const libc::c_char)
    };
    Some(PathBuf::from(path.to_string_lossy().into_owned()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn live_cwd(_shell_pid: u32) -> Option<PathBuf> {
    None
}

/// One sidebar project group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectGroup {
    pub name: String,
    pub path: PathBuf,
    /// Branch groups (git projects with a parseable HEAD). Empty for
    /// non-git projects — those sessions sit directly under the project.
    pub branches: Vec<BranchGroup>,
    /// Sessions of non-git (or unparseable-HEAD) projects.
    pub sessions: Vec<u64>,
}

/// Sessions sharing one branch of one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchGroup {
    pub branch: String,
    pub sessions: Vec<u64>,
}

/// Per project root: (display name, [(session id, branch)]).
type ProjectMembers = BTreeMap<PathBuf, (String, Vec<(u64, Option<String>)>)>;

/// Pure grouping: `(session id, classification)` pairs → a deterministic
/// project tree. Projects sorted by name, branches by name, sessions by id.
pub fn group_sessions(sessions: &[(u64, ProjectInfo)]) -> Vec<ProjectGroup> {
    // Collect per project root: (display name, [(id, branch)]).
    let mut by_root: ProjectMembers = BTreeMap::new();
    for (id, info) in sessions {
        let entry = by_root
            .entry(info.root.clone())
            .or_insert_with(|| (info.name.clone(), Vec::new()));
        entry.1.push((*id, info.branch.clone()));
    }

    let mut projects = Vec::new();
    for (path, (name, mut members)) in by_root {
        members.sort_by_key(|(id, _)| *id);
        let has_branches = members.iter().any(|(_, branch)| branch.is_some());
        let group = if has_branches {
            let mut by_branch: BTreeMap<String, Vec<u64>> = BTreeMap::new();
            for (id, branch) in members {
                if let Some(branch) = branch {
                    by_branch.entry(branch).or_default().push(id);
                } else {
                    // HEAD unparseable for one session but parseable for
                    // another: fold the orphan into a "<no branch>" group.
                    by_branch
                        .entry("<no branch>".to_owned())
                        .or_default()
                        .push(id);
                }
            }
            ProjectGroup {
                branches: by_branch
                    .into_iter()
                    .map(|(branch, sessions)| BranchGroup { branch, sessions })
                    .collect(),
                sessions: Vec::new(),
                name,
                path,
            }
        } else {
            ProjectGroup {
                branches: Vec::new(),
                sessions: members.into_iter().map(|(id, _)| id).collect(),
                name,
                path,
            }
        };
        projects.push(group);
    }
    // Deterministic order: projects by display name.
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    projects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agentmux-proj-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .expect("git should run");
        assert!(status.success(), "git {args:?} failed in {}", cwd.display());
    }

    #[test]
    fn git_root_discovered_from_nested_dir() {
        let dir = unique_dir("root");
        git(&["-c", "init.defaultBranch=main", "init", "-q"], &dir);
        git(
            &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"],
            &dir,
        );
        let nested = dir.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_project_root(&nested), dir);
        // The repo root itself is its own project root.
        assert_eq!(find_project_root(&dir), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_git_dir_is_standalone_project() {
        let dir = unique_dir("plain");
        let nested = dir.join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(find_project_root(&nested), nested, "no .git anywhere → cwd itself");
        assert_eq!(find_project_root(&dir), dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_parsing_regular_branch() {
        let dir = unique_dir("head");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(read_head_branch(&dir).as_deref(), Some("feature/x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_parsing_detached_sha() {
        let dir = unique_dir("detached");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(read_head_branch(&dir).as_deref(), Some("0123456"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_parsing_worktree_gitdir_file() {
        let dir = unique_dir("worktree");
        let common = dir.join("common");
        std::fs::create_dir_all(common.join(".git/worktrees/feature")).unwrap();
        std::fs::write(common.join(".git/worktrees/feature/HEAD"), "ref: refs/heads/feature\n").unwrap();
        // The worktree's .git is a FILE pointing at the common gitdir
        // (relative to the directory containing the .git file, i.e. root).
        std::fs::write(dir.join(".git"), "gitdir: common/.git/worktrees/feature\n").unwrap();
        assert_eq!(read_head_branch(&dir).as_deref(), Some("feature"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_parsing_garbage_is_none() {
        let dir = unique_dir("garbage");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "not a ref at all\n").unwrap();
        assert_eq!(read_head_branch(&dir), None);
        // No .git at all → None.
        let plain = unique_dir("nogit");
        assert_eq!(read_head_branch(&plain), None);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&plain);
    }

    fn info(root: &str, name: &str, branch: Option<&str>) -> ProjectInfo {
        ProjectInfo {
            root: PathBuf::from(root),
            name: name.to_owned(),
            branch: branch.map(str::to_owned),
        }
    }

    #[test]
    fn grouping_mixed_repos_and_standalone() {
        let sessions = vec![
            (1, info("/r/agentmux", "agentmux", Some("main"))),
            (2, info("/r/agentmux", "agentmux", Some("main"))),
            (3, info("/r/agentmux", "agentmux", Some("feat/x"))),
            (4, info("/home/user/somedir", "somedir", None)),
            (5, info("/home/user/somedir", "somedir", None)),
            (6, info("/r/other", "other", Some("main"))),
        ];
        let groups = group_sessions(&sessions);
        assert_eq!(groups.len(), 3, "three projects");
        // Sorted by name: agentmux, other, somedir.
        assert_eq!(groups[0].name, "agentmux");
        assert_eq!(groups[0].path, PathBuf::from("/r/agentmux"));
        assert_eq!(groups[0].sessions, Vec::<u64>::new());
        assert_eq!(groups[0].branches.len(), 2, "two branches, sorted");
        assert_eq!(groups[0].branches[0].branch, "feat/x");
        assert_eq!(groups[0].branches[0].sessions, vec![3]);
        assert_eq!(groups[0].branches[1].branch, "main");
        assert_eq!(groups[0].branches[1].sessions, vec![1, 2], "sessions by id");

        assert_eq!(groups[1].name, "other");
        assert_eq!(groups[2].name, "somedir");
        assert_eq!(groups[2].branches, Vec::new());
        assert_eq!(groups[2].sessions, vec![4, 5], "standalone sessions by id");
    }

    #[test]
    fn grouping_orphan_session_without_branch() {
        let sessions = vec![
            (1, info("/r/agentmux", "agentmux", Some("main"))),
            (2, info("/r/agentmux", "agentmux", None)), // HEAD unparseable
        ];
        let groups = group_sessions(&sessions);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].branches.len(), 2);
        // "<no branch>" sorts before "main" — deterministic either way.
        assert_eq!(groups[0].branches[0].branch, "<no branch>");
        assert_eq!(groups[0].branches[0].sessions, vec![2]);
        assert_eq!(groups[0].branches[1].branch, "main");
    }

    #[test]
    fn classifier_caches_head_reads() {
        let dir = unique_dir("cache");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();

        let mut classifier = ProjectClassifier::new();
        let t0 = Instant::now();
        let info = classifier.classify(&dir, t0);
        assert_eq!(info.name, dir.file_name().unwrap().to_string_lossy());
        assert_eq!(info.branch.as_deref(), Some("main"));

        // Within the cache window, a HEAD change is not picked up...
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/other\n").unwrap();
        let info = classifier.classify(&dir, t0 + Duration::from_millis(100));
        assert_eq!(info.branch.as_deref(), Some("main"), "cached");

        // ...but after the refresh interval it is.
        let info = classifier.classify(&dir, t0 + Duration::from_secs(3));
        assert_eq!(info.branch.as_deref(), Some("other"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
