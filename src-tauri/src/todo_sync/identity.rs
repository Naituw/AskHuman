//! Project identity (spec todo-sync §5.1, T2): the normalized git remote of a project directory.
//!
//! The same repository checked out in several directories (or cloned over ssh on one Mac and https on
//! another) maps to one identity, which is what makes those directories replicas of one cloud project.
//! Remotes are read straight from `.git/config` — no `git` subprocess — following a worktree's
//! `gitdir:` / `commondir` indirection.

use super::model::Link;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Normalize one remote URL to `host/path`: lowercase, no scheme / user / port / `.git` / trailing `/`.
/// Local paths, `file://` and anything without a host return `None`.
pub fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let (host, path) = if let Some(at) = url.find("://") {
        let scheme = url[..at].to_ascii_lowercase();
        if !matches!(
            scheme.as_str(),
            "ssh" | "https" | "http" | "git" | "git+ssh" | "ssh+git"
        ) {
            return None;
        }
        let rest = &url[at + 3..];
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host_port = authority.rsplit('@').next().unwrap_or("");
        if host_port.starts_with('[') {
            return None; // bracketed IPv6 literals: not worth supporting
        }
        (host_port.split(':').next().unwrap_or(""), path)
    } else {
        // scp-like `[user@]host:path`; anything that looks like a filesystem path is rejected.
        if url.starts_with(['/', '.', '~', '\\']) {
            return None;
        }
        let (left, path) = url.split_once(':')?;
        if left.contains('/') || left.contains('\\') {
            return None;
        }
        let host = left.rsplit('@').next().unwrap_or("");
        if host.len() == 1 && host.chars().all(|c| c.is_ascii_alphabetic()) {
            return None; // `C:\repo` — a Windows drive, not a host
        }
        (host, path)
    };
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c == '/') {
        return None;
    }
    let mut path = path.trim().trim_matches('/').to_ascii_lowercase();
    if let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.trim_end_matches('/').to_string();
    }
    if path.is_empty() {
        return None;
    }
    Some(format!("{host}/{path}"))
}

/// Pick the project identity from a repository's remotes: `origin`, then `upstream`, then the rest by
/// name; the first one that normalizes wins (unqualified remotes are skipped).
pub fn choose_identity(remotes: &[(String, String)]) -> Option<String> {
    let mut order: Vec<&(String, String)> = Vec::new();
    for preferred in ["origin", "upstream"] {
        order.extend(remotes.iter().filter(|(name, _)| name == preferred));
    }
    let mut rest: Vec<&(String, String)> = remotes
        .iter()
        .filter(|(name, _)| name != "origin" && name != "upstream")
        .collect();
    rest.sort_by(|a, b| a.0.cmp(&b.0));
    order.extend(rest);
    order.iter().find_map(|(_, url)| normalize_remote(url))
}

/// `(remote name, first url)` pairs from the text of a git config file.
pub fn parse_remotes(config: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut current: Option<String> = None;
    for raw in config.lines() {
        let line = raw.trim();
        if let Some(header) = line.strip_prefix('[') {
            current = None;
            let header = header.split(']').next().unwrap_or("").trim();
            let mut parts = header.splitn(2, char::is_whitespace);
            let section = parts.next().unwrap_or("");
            if section.eq_ignore_ascii_case("remote") {
                if let Some(name) = parts.next() {
                    let name = name.trim().trim_matches('"');
                    if !name.is_empty() {
                        current = Some(name.to_string());
                    }
                }
            }
            continue;
        }
        let Some(name) = &current else { continue };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("url") {
            continue;
        }
        let value = config_value(value);
        if !value.is_empty() && !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), value));
        }
    }
    out
}

/// A git config value: surrounding quotes removed, trailing ` # comment` / ` ; comment` dropped.
fn config_value(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(quoted) = raw.strip_prefix('"') {
        return quoted.split('"').next().unwrap_or("").to_string();
    }
    let cut = [" #", " ;", "\t#", "\t;"]
        .iter()
        .filter_map(|m| raw.find(m))
        .min()
        .unwrap_or(raw.len());
    raw[..cut].trim().to_string()
}

/// The git config file that holds a working tree's remotes, plus its modification time (ms).
/// `root` is the project directory (a git root). A worktree / submodule has a `.git` *file* pointing at
/// its own git dir, whose `commondir` leads back to the main repository's config.
pub fn config_path(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let meta = std::fs::metadata(&dot_git).ok()?;
    let git_dir = if meta.is_dir() {
        dot_git
    } else {
        let text = std::fs::read_to_string(&dot_git).ok()?;
        let target = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("gitdir:"))?
            .trim();
        let target = Path::new(target);
        if target.is_absolute() {
            target.to_path_buf()
        } else {
            root.join(target)
        }
    };
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let config_dir = match common {
        Some(common) => {
            let common = Path::new(&common);
            if common.is_absolute() {
                common.to_path_buf()
            } else {
                git_dir.join(common)
            }
        }
        None => git_dir,
    };
    Some(config_dir.join("config"))
}

fn mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Resolves a local project key (a git root path) to its identity.
pub trait IdentityResolver {
    /// `prev` is the previous result for the same key: implementations may reuse it when nothing
    /// changed since (the filesystem one compares the git config's modification time).
    fn resolve(&self, key: &str, prev: Option<&Link>, now_ms: u64) -> Link;
}

/// Production resolver: reads `.git/config`, no subprocess.
pub struct FsResolver;

impl IdentityResolver for FsResolver {
    fn resolve(&self, key: &str, prev: Option<&Link>, now_ms: u64) -> Link {
        let path = config_path(Path::new(key));
        let mtime = path.as_deref().map(mtime_ms).unwrap_or(0);
        if let Some(prev) = prev {
            if prev.cfg_mtime_ms == mtime && (path.is_some() || prev.p.is_none()) {
                return Link {
                    checked_ms: now_ms,
                    ..prev.clone()
                };
            }
        }
        let identity = path
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| choose_identity(&parse_remotes(&text)));
        Link {
            p: identity,
            cfg_mtime_ms: mtime,
            checked_ms: now_ms,
        }
    }
}

/// Short display names for a set of identities: the last path segment, with enough leading segments
/// added (`owner/repo`) to tell colliding names apart.
pub fn display_names<'a>(
    identities: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, String> {
    let ids: Vec<&str> = identities.into_iter().collect();
    let segments: Vec<Vec<&str>> = ids.iter().map(|id| id.split('/').collect()).collect();
    let mut depth = vec![1usize; ids.len()];
    let name_at = |i: usize, d: usize| -> String {
        let seg = &segments[i];
        // The host is the first segment; it is only included when everything else collides.
        let d = d.min(seg.len());
        seg[seg.len() - d..].join("/")
    };
    loop {
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, d) in depth.iter().enumerate() {
            groups.entry(name_at(i, *d)).or_default().push(i);
        }
        let mut grew = false;
        for members in groups.values().filter(|m| m.len() > 1) {
            for &i in members {
                if depth[i] < segments[i].len() {
                    depth[i] += 1;
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    ids.iter()
        .enumerate()
        .map(|(i, id)| (id.to_string(), name_at(i, depth[i])))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_table_from_the_spec() {
        let same = [
            "git@github.com:NaituW/HumanInLoop.git",
            "https://github.com/naituw/HumanInLoop",
            "https://user@github.com/naituw/HumanInLoop.git/",
            "ssh://git@github.com/naituw/humaninloop.git",
            "git://GitHub.com/NaituW/HumanInLoop.git",
            "https://github.com:443/naituw/HumanInLoop",
            "HTTP://token:x-oauth@github.com/naituw/humaninloop",
        ];
        for url in same {
            assert_eq!(
                normalize_remote(url).as_deref(),
                Some("github.com/naituw/humaninloop"),
                "{url}"
            );
        }
        assert_eq!(
            normalize_remote("ssh://git@gitlab.corp.example:2222/team/api.git").as_deref(),
            Some("gitlab.corp.example/team/api")
        );
        assert_eq!(
            normalize_remote("git@gitlab.corp.example:group/sub/repo.git").as_deref(),
            Some("gitlab.corp.example/group/sub/repo")
        );
    }

    #[test]
    fn unqualified_remotes_are_rejected() {
        for url in [
            "",
            "   ",
            "/Users/x/repos/bare.git",
            "../sibling",
            "./local",
            "~/code/repo",
            "file:///Users/x/repo",
            "C:\\repos\\thing",
            "C:/repos/thing",
            "https://github.com",
            "https://github.com/",
            "ssh://[::1]/repo",
            "just-a-word",
            "svn://host/repo",
        ] {
            assert_eq!(normalize_remote(url), None, "{url:?}");
        }
    }

    #[test]
    fn choose_prefers_origin_then_upstream_then_name_and_skips_unqualified() {
        let r = |name: &str, url: &str| (name.to_string(), url.to_string());
        assert_eq!(
            choose_identity(&[
                r("zeta", "https://z.example/a/b"),
                r("upstream", "https://up.example/a/b"),
                r("origin", "https://o.example/a/b"),
            ])
            .as_deref(),
            Some("o.example/a/b")
        );
        assert_eq!(
            choose_identity(&[
                r("zeta", "https://z.example/a/b"),
                r("upstream", "https://up.example/a/b")
            ])
            .as_deref(),
            Some("up.example/a/b")
        );
        assert_eq!(
            choose_identity(&[
                r("zeta", "https://z.example/a/b"),
                r("alpha", "https://a.example/a/b")
            ])
            .as_deref(),
            Some("a.example/a/b")
        );
        // origin is a local path: fall through to the next qualified one.
        assert_eq!(
            choose_identity(&[
                r("origin", "/srv/git/x.git"),
                r("fork", "git@h.example:me/x.git")
            ])
            .as_deref(),
            Some("h.example/me/x")
        );
        assert_eq!(choose_identity(&[r("origin", "/srv/git/x.git")]), None);
        assert_eq!(choose_identity(&[]), None);
    }

    #[test]
    fn parses_remotes_out_of_a_git_config() {
        let cfg = r#"
[core]
	repositoryformatversion = 0
[remote "origin"]
	url = git@github.com:NaituW/HumanInLoop.git   # trailing comment
	fetch = +refs/heads/*:refs/remotes/origin/*
	url = git@example.com:ignored/second.git
[branch "main"]
	remote = origin
[remote "upstream"]
	url = "https://github.com/up/stream.git"
[remote "pushonly"]
	pushurl = git@example.com:x/y.git
"#;
        assert_eq!(
            parse_remotes(cfg),
            vec![
                (
                    "origin".to_string(),
                    "git@github.com:NaituW/HumanInLoop.git".to_string()
                ),
                (
                    "upstream".to_string(),
                    "https://github.com/up/stream.git".to_string()
                ),
            ]
        );
    }

    #[test]
    fn display_names_disambiguate_collisions() {
        let names = display_names(["github.com/a/api", "gitlab.corp/b/api", "github.com/a/web"]);
        assert_eq!(names["github.com/a/web"], "web");
        assert_eq!(names["github.com/a/api"], "a/api");
        assert_eq!(names["gitlab.corp/b/api"], "b/api");
        let same_owner = display_names(["github.com/a/api", "gitlab.corp/a/api"]);
        assert_eq!(same_owner["github.com/a/api"], "github.com/a/api");
        assert_eq!(same_owner["gitlab.corp/a/api"], "gitlab.corp/a/api");
    }

    #[test]
    fn reads_plain_repos_worktrees_and_missing_configs_without_git() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::write(
            main.join(".git/config"),
            "[remote \"origin\"]\n\turl = git@github.com:Me/Repo.git\n",
        )
        .unwrap();
        // The worktree's gitdir lives under the main repo; commondir points back (relative, as git writes it).
        std::fs::write(main.join(".git/worktrees/wt/commondir"), "../..\n").unwrap();
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        let bare_dir = tmp.path().join("plain");
        std::fs::create_dir_all(bare_dir.join(".git")).unwrap();

        let resolver = FsResolver;
        let a = resolver.resolve(main.to_str().unwrap(), None, 10);
        let b = resolver.resolve(wt.to_str().unwrap(), None, 10);
        assert_eq!(a.p.as_deref(), Some("github.com/me/repo"));
        assert_eq!(b.p, a.p, "a worktree shares its main repository's identity");
        assert!(a.cfg_mtime_ms > 0);
        let none = resolver.resolve(bare_dir.to_str().unwrap(), None, 10);
        assert_eq!(none.p, None, "no remote → local-only");
        let missing = resolver.resolve(tmp.path().join("nope").to_str().unwrap(), None, 10);
        assert_eq!(missing.p, None);

        // A second resolve with unchanged mtime reuses the previous result (and only bumps checkedMs).
        let again = resolver.resolve(main.to_str().unwrap(), Some(&a), 99);
        assert_eq!(again.p, a.p);
        assert_eq!(again.checked_ms, 99);
        // A changed config is picked up.
        std::fs::write(
            main.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://example.com/other/thing\n",
        )
        .unwrap();
        let mut stale = a.clone();
        stale.cfg_mtime_ms = 1; // force the mtime comparison to differ regardless of fs timestamp granularity
        let changed = resolver.resolve(main.to_str().unwrap(), Some(&stale), 100);
        assert_eq!(changed.p.as_deref(), Some("example.com/other/thing"));
    }
}
