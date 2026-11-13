//! Projects: the level above terminal tabs in the session list.
//!
//! A tab belongs to the project of the folder it works in: the git
//! repository that holds the folder, else the folder itself. Everything
//! here is pure, except [`has_git_entry`], the one filesystem probe, which
//! the view runs off the UI thread and only for folders of this machine.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use uuid::Uuid;

/// The folder a tab works in, as text: what the terminal reported (OSC 7),
/// else the path a shell title carries (`user@host: ~/folder`), else the
/// folder the terminal was started in. `home` expands a leading `~`; pass
/// `None` for a remote tab, whose home is not known here.
pub(super) fn tab_folder(
    reported: Option<&str>,
    title: &str,
    started_in: Option<&str>,
    home: Option<&str>,
) -> Option<String> {
    // Only a shell title: a program title such as `Fix: /login page` has
    // a colon and a slash too, and would move the tab to a project.
    let from_title = title
        .trim()
        .split_once(": ")
        .filter(|(who, _)| who.contains('@') && !who.contains(char::is_whitespace))
        .map(|(_, path)| path.trim())
        .filter(|path| path.starts_with('/') || *path == "~" || path.starts_with("~/"));
    // OSC 7 carries a URL: shells write a space or an accent as `%20`,
    // `%C3%A9`, and the folder on disk has the character itself.
    let reported = reported.map(percent_decoded);
    let folder = [reported.as_deref(), from_title, started_in]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|folder| !folder.is_empty())?;
    let home = home.map(|home| home.trim_end_matches('/'));
    Some(match (folder.strip_prefix('~'), home) {
        (Some(rest), Some(home))
            if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) =>
        {
            format!("{home}{rest}")
        }
        _ => folder.to_string(),
    })
}

/// `path` with its `%XX` escapes replaced by the bytes they stand for. Left
/// as written when an escape is malformed or the result is not UTF-8.
fn percent_decoded(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'%' {
            decoded.push(bytes[at]);
            at += 1;
            continue;
        }
        let byte = bytes
            .get(at + 1..at + 3)
            .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match byte {
            Some(byte) => decoded.push(byte),
            None => return path.to_string(),
        }
        at += 3;
    }
    String::from_utf8(decoded).unwrap_or_else(|_| path.to_string())
}

/// A path without its trailing slashes (`/` stays `/`).
fn trimmed(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/"
    } else {
        trimmed
    }
}

/// The project of a folder of this machine: the nearest folder, going up
/// from it, that holds a `.git` entry, else the folder itself. `None` for
/// the home directory, the root and anything that is not an absolute path.
///
/// The walk stops below the home directory: a home that is itself a
/// repository (dotfiles) would otherwise make one project of everything.
/// `has_git` is the filesystem probe, so the rule can be tested without one.
pub(super) fn local_project_root(
    folder: &str,
    home: Option<&str>,
    has_git: impl Fn(&Path) -> bool,
) -> Option<String> {
    let folder = trimmed(folder.trim());
    let home = home.map(|home| trimmed(home.trim()));
    if !folder.starts_with('/') || folder == "/" || Some(folder) == home {
        return None;
    }
    let mut dir = Path::new(folder);
    loop {
        if dir.parent().is_none() || dir.to_str() == home {
            break;
        }
        if has_git(dir) {
            return dir.to_str().map(str::to_string);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    Some(folder.to_string())
}

/// The project of a folder on another machine: the folder itself. Its
/// repository cannot be looked for from here, and a path of that machine
/// must never be probed on this one. `None` for what reads as a home
/// directory (`~`, `/root`, `/home/name`, `/Users/name`) and for the root.
pub(super) fn remote_project_root(folder: &str) -> Option<String> {
    let folder = trimmed(folder.trim());
    if folder == "/" || folder == "~" || folder == "/root" {
        return None;
    }
    if !folder.starts_with('/') && !folder.starts_with("~/") {
        return None;
    }
    let user_home = ["/home/", "/Users/"].iter().any(|prefix| {
        folder
            .strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
    });
    if user_home {
        return None;
    }
    Some(folder.to_string())
}

/// Whether `dir` holds a `.git` entry: a directory in a clone, a file in a
/// worktree or a submodule. Git itself is never run.
pub(super) fn has_git_entry(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
}

/// What the view knows about the projects of local folders. Finding a
/// repository reads the disk, so it is done off the UI thread: the view
/// asks with [`ProjectIndex::request`], answers with
/// [`ProjectIndex::resolve`], and rendering only reads [`ProjectIndex::root`].
#[derive(Debug, Default)]
pub(super) struct ProjectIndex {
    roots: HashMap<String, Option<String>>,
    pending: HashSet<String>,
}

impl ProjectIndex {
    /// The project of `folder`, when it has been looked for. `None` while
    /// the first lookup runs and for a folder without a project.
    pub(super) fn root(&self, folder: &str) -> Option<&str> {
        self.roots.get(folder)?.as_deref()
    }

    /// A tab entered `folder`. `true` when the caller has to look for its
    /// project; `false` while a lookup for that folder already runs. A
    /// folder seen before is looked at again (a repository may have been
    /// created there since) and keeps its answer meanwhile.
    pub(super) fn request(&mut self, folder: &str) -> bool {
        self.pending.insert(folder.to_string())
    }

    /// The lookup for `folder` ended. `true` when the answer changed.
    pub(super) fn resolve(&mut self, folder: &str, root: Option<String>) -> bool {
        self.pending.remove(folder);
        self.roots.insert(folder.to_string(), root.clone()) != Some(root)
    }
}

/// What groups a tab in the session list by project.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ProjectKey {
    /// Home directory, unknown folder: no project.
    None,
    /// A project folder. The same path on two machines is two projects.
    Folder {
        connection_id: Option<Uuid>,
        root: String,
    },
}

/// Where one tab is, as the grouping needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TabPlace {
    /// `None` is this machine.
    pub connection_id: Option<Uuid>,
    /// The machine's name, for a connection tab.
    pub host: Option<String>,
    /// The project folder, when the tab is in one.
    pub root: Option<String>,
}

impl TabPlace {
    fn key(&self) -> ProjectKey {
        match &self.root {
            Some(root) => ProjectKey::Folder {
                connection_id: self.connection_id,
                root: root.clone(),
            },
            None => ProjectKey::None,
        }
    }
}

/// The tabs of one project in the session list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectGroup {
    pub key: ProjectKey,
    /// The machine's name for a project on a connection, when a tab
    /// recorded one.
    pub host: Option<String>,
    /// Indices into the tab list, in tab order.
    pub tabs: Vec<usize>,
}

/// Group the tabs by project: each project in the order its first tab was
/// opened, the tabs without a project last. Tab order is kept inside a
/// group, as in the grouping by machine.
pub(super) fn project_groups(places: &[TabPlace]) -> Vec<ProjectGroup> {
    let mut groups: Vec<ProjectGroup> = Vec::new();
    for (index, place) in places.iter().enumerate() {
        let key = place.key();
        // A tab without a project says nothing about a machine.
        let host = match key {
            ProjectKey::None => None,
            ProjectKey::Folder { .. } => place.host.clone(),
        };
        match groups.iter_mut().find(|group| group.key == key) {
            Some(group) => {
                group.tabs.push(index);
                if group.host.is_none() {
                    group.host = host;
                }
            }
            None => groups.push(ProjectGroup {
                key,
                host,
                tabs: vec![index],
            }),
        }
    }
    // Stable: only the group without a project moves, to the end.
    groups.sort_by_key(|group| group.key == ProjectKey::None);
    groups
}

/// A project folder as a header shows it: its name, and the path of the
/// folder that holds it, with the home directory written `~`.
pub(super) fn project_label(root: &str, home: Option<&str>) -> (String, Option<String>) {
    let root = trimmed(root);
    let Some((parent, name)) = root.rsplit_once('/') else {
        return (root.to_string(), None);
    };
    if name.is_empty() {
        return (root.to_string(), None);
    }
    let parent = if parent.is_empty() { "/" } else { parent };
    let parent = match home.map(trimmed).filter(|home| !home.is_empty()) {
        Some(home) if parent == home => "~".to_string(),
        Some(home) => match parent.strip_prefix(home) {
            Some(rest) if rest.starts_with('/') => format!("~{rest}"),
            _ => parent.to_string(),
        },
        None => parent.to_string(),
    };
    (name.to_string(), Some(parent))
}

/// The tab to select when moving to the next (`forward`) or the previous
/// project from the tab `active`: the most recently used tab of that
/// project, else its first. `recent` lists tab indices, most recent first.
/// `None` when there is no other project to go to.
pub(super) fn adjacent_project_tab(
    groups: &[ProjectGroup],
    active: usize,
    recent: &[usize],
    forward: bool,
) -> Option<usize> {
    if groups.len() < 2 {
        return None;
    }
    let current = groups
        .iter()
        .position(|group| group.tabs.contains(&active))?;
    let target = if forward {
        (current + 1) % groups.len()
    } else {
        (current + groups.len() - 1) % groups.len()
    };
    let tabs = &groups[target].tabs;
    recent
        .iter()
        .copied()
        .find(|index| tabs.contains(index))
        .or_else(|| tabs.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A filesystem where exactly `repos` hold a `.git` entry.
    fn with_repos<'a>(repos: &'a [&'a str]) -> impl Fn(&Path) -> bool + 'a {
        move |dir| repos.iter().any(|repo| Path::new(repo) == dir)
    }

    // SDTEST-3140 / SDUC-584. A tab is in the repository that holds its
    // folder: the nearest one going up (a nested repository or a worktree
    // is its own project), and a folder outside any repository is its own
    // project.
    #[test]
    fn sdtest_3140_a_folder_belongs_to_the_nearest_repository_above_it() {
        let home = Some("/home/ann");
        let repos = ["/home/ann/dev/app", "/home/ann/dev/app/vendor/lib"];
        let root = |folder: &str| local_project_root(folder, home, with_repos(&repos));

        assert_eq!(
            root("/home/ann/dev/app").as_deref(),
            Some("/home/ann/dev/app")
        );
        assert_eq!(
            root("/home/ann/dev/app/src/ui/").as_deref(),
            Some("/home/ann/dev/app")
        );
        // The inner repository wins inside it, the outer one beside it.
        assert_eq!(
            root("/home/ann/dev/app/vendor/lib/src").as_deref(),
            Some("/home/ann/dev/app/vendor/lib")
        );
        assert_eq!(
            root("/home/ann/dev/app/vendor").as_deref(),
            Some("/home/ann/dev/app")
        );
        // No repository: the folder itself, in and out of the home.
        assert_eq!(
            root("/home/ann/notes/2026").as_deref(),
            Some("/home/ann/notes/2026")
        );
        assert_eq!(root("/etc/nginx").as_deref(), Some("/etc/nginx"));
    }

    // SDTEST-3141 / SDUC-584. The home directory, the root and what is not
    // a path are no project, and a home that is a repository (dotfiles)
    // does not swallow the folders under it.
    #[test]
    fn sdtest_3141_the_home_directory_and_unknown_folders_are_no_project() {
        let none = |_: &Path| false;
        assert_eq!(
            local_project_root("/home/ann", Some("/home/ann"), none),
            None
        );
        assert_eq!(
            local_project_root("/home/ann/", Some("/home/ann/"), none),
            None
        );
        assert_eq!(local_project_root("/", Some("/home/ann"), none), None);
        assert_eq!(local_project_root("", Some("/home/ann"), none), None);
        assert_eq!(local_project_root("notes", Some("/home/ann"), none), None);

        let dotfiles = ["/home/ann", "/"];
        assert_eq!(
            local_project_root("/home/ann/notes", Some("/home/ann"), with_repos(&dotfiles))
                .as_deref(),
            Some("/home/ann/notes")
        );
        // The root is never a project either.
        assert_eq!(
            local_project_root("/srv/www", None, with_repos(&dotfiles)).as_deref(),
            Some("/srv/www")
        );
    }

    // SDTEST-3142 / SDUC-584. A folder on another machine is its own
    // project, worked out from its text alone; what reads as a home
    // directory there is no project.
    #[test]
    fn sdtest_3142_a_remote_folder_is_its_own_project_without_any_lookup() {
        assert_eq!(
            remote_project_root("/var/www/shop/current/").as_deref(),
            Some("/var/www/shop/current")
        );
        assert_eq!(
            remote_project_root("~/sites/blog").as_deref(),
            Some("~/sites/blog")
        );
        assert_eq!(
            remote_project_root("/home/deploy/app").as_deref(),
            Some("/home/deploy/app")
        );
        for home in ["~", "/root", "/home/deploy", "/Users/ann/", "/", "", "vim"] {
            assert_eq!(remote_project_root(home), None, "{home:?}");
        }
    }

    // SDTEST-3143 / SDUC-584. The folder of a tab: what the terminal
    // reported, else the path in a shell title, else where it started. A
    // `~` is the local home, and stays as written for another machine.
    #[test]
    fn sdtest_3143_the_folder_of_a_tab_comes_from_the_report_the_title_or_the_start() {
        let home = Some("/home/ann/");
        assert_eq!(
            tab_folder(Some("/srv/app"), "ann@box: ~/dev", Some("/tmp"), home).as_deref(),
            Some("/srv/app")
        );
        assert_eq!(
            tab_folder(None, "ann@box: ~/dev/app", Some("/tmp"), home).as_deref(),
            Some("/home/ann/dev/app")
        );
        assert_eq!(
            tab_folder(None, "ann@box: ~", None, home).as_deref(),
            Some("/home/ann")
        );
        // A program title is not a folder.
        assert_eq!(
            tab_folder(None, "vim: notes.md", Some("/tmp"), home).as_deref(),
            Some("/tmp")
        );
        assert_eq!(
            tab_folder(None, "Fix: /login page", Some("/tmp"), home).as_deref(),
            Some("/tmp")
        );
        assert_eq!(tab_folder(None, "Terminal", None, home), None);
        // The report is a URL path: escapes are the characters on disk.
        assert_eq!(
            tab_folder(Some("/srv/Mes%20sites/caf%C3%A9"), "", None, home).as_deref(),
            Some("/srv/Mes sites/café")
        );
        // Not an escape, or not text once decoded: left as written.
        for odd in ["/srv/100%", "/srv/50%zz", "/srv/%FF"] {
            assert_eq!(tab_folder(Some(odd), "", None, home).as_deref(), Some(odd));
        }
        assert_eq!(tab_folder(Some("  "), "", None, home), None);
        // Remote: the home is not known here.
        assert_eq!(
            tab_folder(None, "deploy@web1: ~/sites/blog", None, None).as_deref(),
            Some("~/sites/blog")
        );
    }

    fn place(connection: Option<u128>, host: Option<&str>, root: Option<&str>) -> TabPlace {
        TabPlace {
            connection_id: connection.map(Uuid::from_u128),
            host: host.map(str::to_string),
            root: root.map(str::to_string),
        }
    }

    // SDTEST-3144 / SDUC-584. Projects come in the order their first tab
    // was opened, tabs keep their order, the tabs without a project come
    // last whatever their machine, and the same path on two machines is
    // two projects.
    #[test]
    fn sdtest_3144_tabs_are_grouped_by_project_in_tab_order() {
        let places = [
            place(None, None, None),
            place(None, None, Some("/home/ann/dev/app")),
            place(Some(1), Some("web1"), Some("/srv/app")),
            place(None, None, Some("/srv/app")),
            place(Some(1), None, None),
            place(None, None, Some("/home/ann/dev/app")),
            place(Some(1), None, Some("/srv/app")),
        ];
        let groups = project_groups(&places);
        let summary: Vec<(Option<&str>, Option<&str>, &[usize])> = groups
            .iter()
            .map(|group| {
                let root = match &group.key {
                    ProjectKey::Folder { root, .. } => Some(root.as_str()),
                    ProjectKey::None => None,
                };
                (root, group.host.as_deref(), group.tabs.as_slice())
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (Some("/home/ann/dev/app"), None, &[1, 5][..]),
                (Some("/srv/app"), Some("web1"), &[2, 6][..]),
                (Some("/srv/app"), None, &[3][..]),
                (None, None, &[0, 4][..]),
            ]
        );
        assert!(project_groups(&[]).is_empty());
    }

    // SDTEST-3145 / SDUC-584. A project header shows the folder name and,
    // apart, the path that holds it, with the home directory as `~`.
    #[test]
    fn sdtest_3145_a_project_is_shown_as_a_name_and_the_path_that_holds_it() {
        let home = Some("/home/ann");
        let label = |root: &str| project_label(root, home);
        assert_eq!(
            label("/home/ann/dev/app"),
            ("app".to_string(), Some("~/dev".to_string()))
        );
        assert_eq!(
            label("/home/ann/app"),
            ("app".to_string(), Some("~".to_string()))
        );
        assert_eq!(
            label("/srv/app/"),
            ("app".to_string(), Some("/srv".to_string()))
        );
        assert_eq!(label("/srv"), ("srv".to_string(), Some("/".to_string())));
        // A path of another machine is shown as it was written.
        assert_eq!(
            project_label("~/sites/blog", None),
            ("blog".to_string(), Some("~/sites".to_string()))
        );
        // `/home/annabel` is not under `/home/ann`.
        assert_eq!(
            label("/home/annabel/app"),
            ("app".to_string(), Some("/home/annabel".to_string()))
        );
    }

    // SDTEST-3146 / SDUC-584. Next and previous project go round the
    // groups and land on the tab of the project that was used last.
    #[test]
    fn sdtest_3146_moving_to_a_project_selects_its_most_recently_used_tab() {
        let places = [
            place(None, None, Some("/a")),
            place(None, None, Some("/b")),
            place(None, None, Some("/a")),
            place(None, None, Some("/b")),
            place(None, None, None),
        ];
        let groups = project_groups(&places);
        // Tab 3 of /b was used after tab 1; tab 2 of /a after tab 0.
        let recent = [0, 3, 2, 1];
        assert_eq!(adjacent_project_tab(&groups, 0, &recent, true), Some(3));
        // From /b: forward is the tabs without a project, back is /a,
        // where tab 0 is the most recent.
        assert_eq!(adjacent_project_tab(&groups, 3, &recent, true), Some(4));
        assert_eq!(adjacent_project_tab(&groups, 3, &recent, false), Some(0));
        // Round the end, both ways.
        assert_eq!(adjacent_project_tab(&groups, 4, &recent, true), Some(0));
        assert_eq!(adjacent_project_tab(&groups, 0, &recent, false), Some(4));
        // A project never used yet: its first tab.
        assert_eq!(adjacent_project_tab(&groups, 0, &[], true), Some(1));

        // One project only: nowhere to go.
        let alone = project_groups(&places[..1]);
        assert_eq!(adjacent_project_tab(&alone, 0, &recent, true), None);
        assert_eq!(adjacent_project_tab(&groups, 9, &recent, true), None);
    }

    // SDTEST-3147 / SDUC-584. On a real disk: a clone has a `.git`
    // directory and a worktree a `.git` file, both mark a project, and
    // only the folders above the tab's own are looked at, up to the first
    // repository.
    #[test]
    fn sdtest_3147_a_git_directory_or_file_on_disk_marks_the_project() {
        let disk = tempfile::tempdir().unwrap();
        let base = disk.path().canonicalize().unwrap();
        let clone = base.join("clone");
        let worktree = base.join("trees/feature");
        let plain = base.join("plain/deep");
        std::fs::create_dir_all(clone.join(".git")).unwrap();
        std::fs::create_dir_all(clone.join("src/ui")).unwrap();
        std::fs::create_dir_all(worktree.join("docs")).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: elsewhere\n").unwrap();
        std::fs::create_dir_all(&plain).unwrap();

        let asked = Cell::new(0usize);
        let probe = |dir: &Path| {
            assert!(dir.starts_with(&base) || base.starts_with(dir));
            asked.set(asked.get() + 1);
            has_git_entry(dir)
        };
        let root = |folder: &Path| local_project_root(folder.to_str().unwrap(), None, probe);
        assert_eq!(
            root(&clone.join("src/ui")),
            clone.to_str().map(str::to_string)
        );
        assert_eq!(asked.get(), 3, "the walk stops at the repository");
        assert_eq!(
            root(&worktree.join("docs")),
            worktree.to_str().map(str::to_string)
        );
        // A folder that does not exist (any more) is still its own project.
        let gone = plain.join("gone");
        assert_eq!(root(&gone), gone.to_str().map(str::to_string));
    }

    // SDTEST-3148 / SDUC-584. A folder is looked up once at a time, its
    // answer is what rendering reads, and going back to a folder looks
    // again without forgetting the answer meanwhile.
    #[test]
    fn sdtest_3148_a_folder_is_looked_up_off_the_render_path_and_refreshed_on_return() {
        let mut index = ProjectIndex::default();
        assert_eq!(index.root("/a/src"), None);
        assert!(index.request("/a/src"));
        assert!(!index.request("/a/src"), "one lookup at a time");
        assert_eq!(index.root("/a/src"), None, "not known yet");

        assert!(index.resolve("/a/src", Some("/a".into())));
        assert_eq!(index.root("/a/src"), Some("/a"));

        // Back in the folder: asked again, the old answer still shown.
        assert!(index.request("/a/src"));
        assert_eq!(index.root("/a/src"), Some("/a"));
        assert!(
            !index.resolve("/a/src", Some("/a".into())),
            "nothing changed"
        );
        // `git init` happened in the folder itself meanwhile.
        assert!(index.request("/a/src"));
        assert!(index.resolve("/a/src", Some("/a/src".into())));
        assert_eq!(index.root("/a/src"), Some("/a/src"));
    }
}
