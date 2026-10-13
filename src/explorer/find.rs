//! 查找文件, after WinSCP's Find Files: every folder under one is read, a
//! listing at a time, and what the mask matches is kept. The model is pure:
//! the SFTP tab asks the worker for the listings it names and hands their
//! answers back, so a search is the same `List` requests browsing makes.

use super::{FileEntry, child_path};
use crate::sftp::DirectoryListing;
use std::collections::{HashMap, HashSet, VecDeque};

/// Listings asked for at once: each is a round trip, so a few at a time
/// read a deep tree several times faster without crowding the connection.
pub const PARALLEL_READS: usize = 4;

/// 文件名, WinSCP's file masks: names separated by `;` or `,`, each matched
/// whole and regardless of case, `*` standing for any run of characters and
/// `?` for any one (`*.*` is every name, dot or not). A mask finds files,
/// and links to files; one ending in `/` finds folders, and links to them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMask {
    files: Vec<Vec<char>>,
    folders: Vec<Vec<char>>,
}

impl FileMask {
    /// `None` when the text names nothing.
    pub fn parse(text: &str) -> Option<Self> {
        let mut mask = Self {
            files: Vec::new(),
            folders: Vec::new(),
        };
        for pattern in text.split([';', ',']).map(str::trim) {
            let (pattern, patterns) = match pattern.strip_suffix('/') {
                Some(folder) => (folder, &mut mask.folders),
                None => (pattern, &mut mask.files),
            };
            if pattern.is_empty() {
                continue;
            }
            let pattern = match pattern.to_lowercase() {
                every if every == "*.*" => "*".to_string(),
                pattern => pattern,
            };
            patterns.push(pattern.chars().collect());
        }
        (!mask.files.is_empty() || !mask.folders.is_empty()).then_some(mask)
    }

    pub fn matches(&self, name: &str, folder: bool) -> bool {
        let name: Vec<char> = name.to_lowercase().chars().collect();
        let patterns = if folder { &self.folders } else { &self.files };
        patterns
            .iter()
            .any(|pattern| wildcard_matches(pattern, &name))
    }
}

/// `*` is any run of characters, `?` any one, the rest itself.
fn wildcard_matches(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    // The last `*` and where in the name it was tried, to widen it on a
    // mismatch.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((star_p, star_n)) => {
                    p = star_p + 1;
                    n = star_n + 1;
                    star = Some((star_p, star_n + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// A file or folder the search found, and the folder it is in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundFile {
    pub directory: String,
    pub entry: FileEntry,
}

impl FoundFile {
    pub fn path(&self) -> String {
        child_path(&self.directory, &self.entry.name, true)
    }
}

/// What the results are sorted by: a column, as in the main window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoundColumn {
    Name,
    Folder,
    Size,
    Modified,
}

/// Sort results as the main window sorts its lists: by a column, ascending
/// or descending, with folders ahead of files whichever way; without one, by
/// folder and then name, which is how a search lists them.
pub fn sort_found(rows: &mut [FoundFile], column: Option<(FoundColumn, bool)>) {
    let name = |found: &FoundFile| found.entry.name.to_lowercase();
    rows.sort_by(|a, b| {
        let by_place = || {
            a.directory
                .cmp(&b.directory)
                .then_with(|| name(a).cmp(&name(b)))
        };
        let by_column = match column {
            None => by_place(),
            Some((column, descending)) => {
                let order = match column {
                    FoundColumn::Name => name(a).cmp(&name(b)),
                    FoundColumn::Folder => a.directory.cmp(&b.directory),
                    FoundColumn::Size => a.entry.size.cmp(&b.entry.size),
                    FoundColumn::Modified => a.entry.modified.cmp(&b.entry.modified),
                }
                .then_with(by_place);
                if descending { order.reverse() } else { order }
            }
        };
        b.entry.is_dir().cmp(&a.entry.is_dir()).then(by_column)
    });
}

/// Whether `path` is one of `deleted`, or inside one of them.
fn deleted_with(path: &str, deleted: &[String]) -> bool {
    deleted.iter().any(|gone| {
        path == gone
            || path
                .strip_prefix(gone.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Why a search is over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchEnd {
    /// Every folder was read, or tried.
    Finished,
    /// 停止, or the dialog closed.
    Stopped,
    /// The folder searched could not be read, or the connection dropped.
    Failed(String),
}

/// One search: the folders still to read, the listings asked for, and what
/// matched so far.
pub struct FileSearch {
    root: String,
    mask: FileMask,
    /// Hidden files are searched while the remote side shows them, as the
    /// list shows them: a search finds what 定位 can then show.
    show_hidden: bool,
    /// Folders to read, the next first: depth first, as WinSCP goes, so what
    /// is found comes a folder at a time.
    waiting: VecDeque<String>,
    /// Listings asked for, by request id.
    reading: HashMap<u64, String>,
    /// The folders read, by their real paths (a listing resolves links):
    /// a link back up the tree leads to one of them, and is not read again.
    visited: HashSet<String>,
    found: Vec<FoundFile>,
    /// Folders below the one searched that could not be read.
    unreadable: usize,
    /// The folder asked for last, which the status line names.
    current: String,
    end: Option<SearchEnd>,
}

impl FileSearch {
    pub fn new(root: String, mask: FileMask, show_hidden: bool) -> Self {
        Self {
            waiting: VecDeque::from([root.clone()]),
            current: root.clone(),
            root,
            mask,
            show_hidden,
            reading: HashMap::new(),
            visited: HashSet::new(),
            found: Vec::new(),
            unreadable: 0,
            end: None,
        }
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn found(&self) -> &[FoundFile] {
        &self.found
    }

    pub fn unreadable(&self) -> usize {
        self.unreadable
    }

    pub fn end(&self) -> Option<&SearchEnd> {
        self.end.as_ref()
    }

    pub fn is_running(&self) -> bool {
        self.end.is_none()
    }

    /// The folders to ask for now, up to [`PARALLEL_READS`] at once, each
    /// with a request id from `next_id`.
    pub fn next_reads(&mut self, mut next_id: impl FnMut() -> u64) -> Vec<(u64, String)> {
        let mut reads = Vec::new();
        while self.is_running() && self.reading.len() < PARALLEL_READS {
            let Some(directory) = self.waiting.pop_front() else {
                break;
            };
            let id = next_id();
            self.current = directory.clone();
            self.reading.insert(id, directory.clone());
            reads.push((id, directory));
        }
        reads
    }

    /// A listing the search asked for came back.
    pub fn listed(&mut self, request_id: u64, result: Result<DirectoryListing, String>) {
        let Some(directory) = self.reading.remove(&request_id) else {
            return;
        };
        match result {
            Ok(listing) => self.take(&directory, &listing),
            Err(error) if directory == self.root => {
                self.finish(SearchEnd::Failed(error));
                return;
            }
            // Skipped, as WinSCP does; the count says how many.
            Err(_) => self.unreadable += 1,
        }
        if self.waiting.is_empty() && self.reading.is_empty() {
            self.finish(SearchEnd::Finished);
        }
    }

    /// Keep what matches in one folder, and queue the folders in it to be
    /// read next, ahead of the rest. Links to folders are followed, as in
    /// WinSCP; a folder already read, reached again through one, is not
    /// read twice.
    fn take(&mut self, directory: &str, listing: &DirectoryListing) {
        if !self.visited.insert(listing.path().to_string()) {
            return;
        }
        let mut entries: Vec<FileEntry> = listing
            .entries()
            .iter()
            .map(FileEntry::from_directory_entry)
            .filter(|entry| !matches!(entry.name.as_ref(), "." | ".."))
            .filter(|entry| self.show_hidden || !entry.is_hidden())
            .collect();
        entries.sort_by_key(|entry| entry.name.to_lowercase());
        let mut folders = Vec::new();
        for entry in entries {
            let folder = entry.is_dir();
            if folder {
                folders.push(child_path(directory, &entry.name, true));
            }
            if self.mask.matches(&entry.name, folder) {
                self.found.push(FoundFile {
                    directory: directory.to_string(),
                    entry,
                });
            }
        }
        for folder in folders.into_iter().rev() {
            self.waiting.push_front(folder);
        }
    }

    /// Results deleted since: they go, and so does whatever was found in a
    /// folder that went.
    pub fn forget(&mut self, deleted: &[String]) {
        self.found
            .retain(|found| !deleted_with(&found.path(), deleted));
    }

    /// 停止: listings still on their way are ignored when they come.
    pub fn stop(&mut self) {
        if self.is_running() {
            self.finish(SearchEnd::Stopped);
        }
    }

    /// The connection dropped: nothing asked for will answer.
    pub fn fail(&mut self, message: String) {
        if self.is_running() {
            self.finish(SearchEnd::Failed(message));
        }
    }

    /// Over: what was found goes in order of folder, then name, whichever
    /// order the listings came back in.
    fn finish(&mut self, end: SearchEnd) {
        self.end = Some(end);
        self.waiting.clear();
        self.reading.clear();
        self.found.sort_by(|a, b| {
            a.directory.cmp(&b.directory).then_with(|| {
                a.entry
                    .name
                    .to_lowercase()
                    .cmp(&b.entry.name.to_lowercase())
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{FileMask, FileSearch, FoundColumn, PARALLEL_READS, SearchEnd, sort_found};
    use crate::sftp::{DirectoryEntry, DirectoryListing, EntryKind, FileMetadata};

    fn mask(text: &str) -> FileMask {
        FileMask::parse(text).unwrap()
    }

    #[test]
    fn a_mask_matches_whole_names_regardless_of_case() {
        let file = |text: &str, name: &str| mask(text).matches(name, false);
        assert!(file("docker-compose.yml", "docker-compose.yml"));
        assert!(file("Docker-Compose.YML", "docker-compose.yml"));
        // As in WinSCP, a name without wildcards is the whole name.
        assert!(!file("docker-compose.yml", "docker-compose.yml.bak"));
        assert!(!file("nginx", "nginx.conf"));
        assert!(file("*.log", "app.log"));
        assert!(file("*.log", "APP.LOG"));
        assert!(!file("*.log", "app.log.1"));
        assert!(file("*.log*", "app.log.1"));
        assert!(file("app?.log", "app1.log"));
        assert!(!file("app?.log", "app.log"));
        assert!(file("*a*b*", "xaybz"));
        assert!(!file("*a*b*", "ba"));
        // `*.*` is every name, with a dot or without.
        assert!(file("*.*", "Makefile"));
        assert!(file("*", ".bashrc"));
        assert!(file("*.yml; *.yaml", "config.yaml"));
        assert!(file("*.yml, Dockerfile", "Dockerfile"));
        assert!(file("文件*", "文件 甲.txt"));
        assert_eq!(FileMask::parse(" ; , "), None);
        assert_eq!(FileMask::parse(""), None);
    }

    #[test]
    fn a_mask_ending_in_a_slash_finds_folders_and_the_rest_files() {
        assert!(!mask("logs").matches("logs", true));
        assert!(mask("logs/").matches("logs", true));
        assert!(!mask("logs/").matches("logs", false));
        assert!(mask("*.yml; node_modules/").matches("node_modules", true));
        assert!(mask("*/").matches(".config", true));
        assert!(FileMask::parse("/").is_none());
    }

    fn entry(name: &str, kind: EntryKind) -> DirectoryEntry {
        DirectoryEntry::new(name, FileMetadata::new(kind, 1, Some(1), Some(0o644)))
    }

    fn folder_link(name: &str) -> DirectoryEntry {
        entry(name, EntryKind::Symlink).with_target_kind(Some(EntryKind::Directory))
    }

    fn listing(path: &str, entries: Vec<DirectoryEntry>) -> Result<DirectoryListing, String> {
        Ok(DirectoryListing::new(path, entries))
    }

    fn ids() -> impl FnMut() -> u64 {
        let mut next = 0;
        move || {
            next += 1;
            next
        }
    }

    fn found(search: &FileSearch) -> Vec<String> {
        search.found().iter().map(|found| found.path()).collect()
    }

    #[test]
    fn every_folder_below_is_read_depth_first_and_what_matches_is_kept() {
        use EntryKind::{Directory, File, Symlink};
        let mut id = ids();
        let mut search = FileSearch::new("/srv".into(), mask("*.yml"), false);
        assert_eq!(search.next_reads(&mut id), [(1, "/srv".to_string())]);
        search.listed(
            1,
            listing(
                "/srv",
                vec![
                    entry("b", Directory),
                    entry("a", Directory),
                    entry("top.yml", File),
                    entry("broken.yml", Symlink),
                    entry(".hidden", Directory),
                ],
            ),
        );
        // The first folder first; hidden folders are not entered.
        assert_eq!(
            search.next_reads(&mut id),
            [(2, "/srv/a".to_string()), (3, "/srv/b".to_string())]
        );
        assert_eq!(search.current(), "/srv/b");
        search.listed(3, listing("/srv/b", vec![entry("b.yml", File)]));
        search.listed(2, Err("权限不足".into()));
        assert_eq!(search.end(), Some(&SearchEnd::Finished));
        assert_eq!(search.unreadable(), 1);
        // In order of folder, whatever order they came back in; a link is
        // found by its own name.
        assert_eq!(
            found(&search),
            ["/srv/broken.yml", "/srv/top.yml", "/srv/b/b.yml"]
        );
    }

    #[test]
    fn links_to_folders_are_followed_and_a_folder_is_read_once() {
        use EntryKind::File;
        let mut id = ids();
        let mut search = FileSearch::new("/srv".into(), mask("*.yml"), false);
        search.next_reads(&mut id);
        search.listed(
            1,
            listing(
                "/srv",
                vec![
                    folder_link("current"),
                    entry("releases", EntryKind::Directory),
                ],
            ),
        );
        assert_eq!(
            search.next_reads(&mut id),
            [
                (2, "/srv/current".to_string()),
                (3, "/srv/releases".to_string())
            ]
        );
        // The listing says where the link leads.
        search.listed(2, listing("/srv/releases/v2", vec![entry("app.yml", File)]));
        search.listed(
            3,
            listing("/srv/releases", vec![folder_link("v2"), folder_link("up")]),
        );
        assert_eq!(
            search.next_reads(&mut id),
            [
                (4, "/srv/releases/up".to_string()),
                (5, "/srv/releases/v2".to_string())
            ]
        );
        // A link back up, and the folder already read through `current`.
        search.listed(4, listing("/srv", vec![folder_link("current")]));
        search.listed(5, listing("/srv/releases/v2", vec![entry("app.yml", File)]));
        assert_eq!(search.end(), Some(&SearchEnd::Finished));
        assert_eq!(found(&search), ["/srv/current/app.yml"]);
    }

    #[test]
    fn a_few_folders_are_read_at_once() {
        let mut id = ids();
        let mut search = FileSearch::new("/".into(), mask("x"), true);
        search.next_reads(&mut id);
        let folders = (0..10)
            .map(|ix| entry(&format!("d{ix}"), EntryKind::Directory))
            .collect();
        search.listed(1, listing("/", folders));
        assert_eq!(search.next_reads(&mut id).len(), PARALLEL_READS);
        assert!(search.next_reads(&mut id).is_empty());
    }

    #[test]
    fn hidden_files_are_searched_only_while_shown() {
        use EntryKind::{Directory, File};
        let entries = || vec![entry(".env", File), entry(".config", Directory)];
        let mut hidden = FileSearch::new("/root".into(), mask("*"), false);
        hidden.next_reads(ids());
        hidden.listed(1, listing("/root", entries()));
        assert!(hidden.found().is_empty());
        assert_eq!(hidden.end(), Some(&SearchEnd::Finished));

        let mut id = ids();
        let mut shown = FileSearch::new("/root".into(), mask("*"), true);
        shown.next_reads(&mut id);
        shown.listed(1, listing("/root", entries()));
        assert_eq!(found(&shown), ["/root/.env"]);
        assert_eq!(
            shown.next_reads(&mut id),
            [(2, "/root/.config".to_string())]
        );
    }

    #[test]
    fn an_unreadable_folder_to_search_ends_the_search() {
        let mut search = FileSearch::new("/nowhere".into(), mask("x"), false);
        search.next_reads(ids());
        search.listed(1, Err("没有那个文件或目录".into()));
        assert_eq!(
            search.end(),
            Some(&SearchEnd::Failed("没有那个文件或目录".into()))
        );
    }

    #[test]
    fn stopping_ignores_the_listings_on_their_way() {
        use EntryKind::{Directory, File};
        let mut id = ids();
        let mut search = FileSearch::new("/".into(), mask("a"), false);
        search.next_reads(&mut id);
        search.listed(
            1,
            listing("/", vec![entry("a", File), entry("d", Directory)]),
        );
        search.next_reads(&mut id);
        search.stop();
        assert_eq!(search.end(), Some(&SearchEnd::Stopped));
        search.listed(2, listing("/d", vec![entry("a", File)]));
        assert_eq!(found(&search), ["/a"]);
        assert!(search.next_reads(&mut id).is_empty());
    }

    #[test]
    fn results_sort_by_a_column_with_folders_first() {
        use EntryKind::{Directory, File};
        let mut search = FileSearch::new("/".into(), mask("*; */"), false);
        let mut id = ids();
        search.next_reads(&mut id);
        let sized = |name: &str, kind, size| {
            DirectoryEntry::new(name, FileMetadata::new(kind, size, Some(size as u32), None))
        };
        search.listed(
            1,
            listing(
                "/",
                vec![
                    sized("b.log", File, 30),
                    sized("A.log", File, 10),
                    sized("z", Directory, 0),
                ],
            ),
        );
        assert_eq!(search.next_reads(&mut id), [(2, "/z".to_string())]);
        search.listed(2, listing("/z", vec![sized("c.log", File, 20)]));
        let sorted = |column| {
            let mut rows = search.found().to_vec();
            sort_found(&mut rows, column);
            rows.iter().map(|found| found.path()).collect::<Vec<_>>()
        };
        // By folder and name until a column is chosen; folders first always.
        assert_eq!(sorted(None), ["/z", "/A.log", "/b.log", "/z/c.log"]);
        assert_eq!(
            sorted(Some((FoundColumn::Size, false))),
            ["/z", "/A.log", "/z/c.log", "/b.log"]
        );
        assert_eq!(
            sorted(Some((FoundColumn::Size, true))),
            ["/z", "/b.log", "/z/c.log", "/A.log"]
        );
        assert_eq!(
            sorted(Some((FoundColumn::Name, true))),
            ["/z", "/z/c.log", "/b.log", "/A.log"]
        );
        assert_eq!(
            sorted(Some((FoundColumn::Folder, true))),
            ["/z", "/z/c.log", "/b.log", "/A.log"]
        );
    }

    #[test]
    fn deleted_results_go_with_what_was_inside_them() {
        use EntryKind::{Directory, File};
        let mut search = FileSearch::new("/".into(), mask("*; */"), false);
        let mut id = ids();
        search.next_reads(&mut id);
        search.listed(
            1,
            listing("/", vec![entry("app", Directory), entry("app.log", File)]),
        );
        assert_eq!(search.next_reads(&mut id), [(2, "/app".to_string())]);
        search.listed(2, listing("/app", vec![entry("x.log", File)]));
        assert_eq!(found(&search), ["/app", "/app.log", "/app/x.log"]);
        search.forget(&["/app".to_string()]);
        assert_eq!(found(&search), ["/app.log"]);
    }
}
