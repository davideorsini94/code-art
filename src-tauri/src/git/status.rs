//! Analisi di `git status --porcelain=v2 -z --branch`.
use serde::Serialize;

/// Voci oltre le quali lo stato viene troncato.
pub const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub path: String,
    pub orig_path: Option<String>,
    /// Lettera di stato nell'indice (`M`, `A`, `D`, `R`, …); `None` se invariato.
    pub staged: Option<char>,
    /// Lettera di stato nella cartella di lavoro; `None` se invariato.
    pub unstaged: Option<char>,
    pub conflict: bool,
    pub untracked: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BranchHeader {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub detached: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub root: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub detached: bool,
    pub merging: bool,
    pub entries: Vec<StatusEntry>,
    pub truncated: bool,
}

fn letter(c: char) -> Option<char> {
    (c != '.').then_some(c)
}

fn xy(s: &str) -> (Option<char>, Option<char>) {
    let mut it = s.chars();
    (it.next().and_then(letter), it.next().and_then(letter))
}

fn entry(path: &str, orig: Option<&str>, x: Option<char>, y: Option<char>, conflict: bool) -> StatusEntry {
    StatusEntry { path: path.into(), orig_path: orig.map(Into::into), staged: x, unstaged: y, conflict, untracked: false }
}

/// `z` = uscita NUL-separata di `status --porcelain=v2 -z --branch`. Le voci illeggibili si saltano.
pub fn parse_porcelain_v2(z: &[u8]) -> (BranchHeader, Vec<StatusEntry>) {
    let text = String::from_utf8_lossy(z);
    let mut header = BranchHeader::default();
    let mut entries = Vec::new();
    let mut recs = text.split('\0');
    while let Some(rec) = recs.next() {
        if let Some(h) = rec.strip_prefix("# ") {
            match h.split_once(' ') {
                Some(("branch.head", v)) if v == "(detached)" => header.detached = true,
                Some(("branch.head", v)) => header.branch = Some(v.into()),
                Some(("branch.upstream", v)) => header.upstream = Some(v.into()),
                Some(("branch.ab", v)) => {
                    let mut p = v.split(' ');
                    header.ahead = p.next().and_then(|a| a.strip_prefix('+')).and_then(|a| a.parse().ok()).unwrap_or(0);
                    header.behind = p.next().and_then(|b| b.strip_prefix('-')).and_then(|b| b.parse().ok()).unwrap_or(0);
                }
                _ => {}
            }
            continue;
        }
        match rec.as_bytes().first() {
            Some(b'1') => {
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                if let [_, xy_, .., path] = f[..] {
                    let (x, y) = xy(xy_);
                    entries.push(entry(path, None, x, y, false));
                }
            }
            Some(b'2') => {
                let f: Vec<&str> = rec.splitn(10, ' ').collect();
                let orig = recs.next().unwrap_or("");
                if let [_, xy_, .., path] = f[..] {
                    let (x, y) = xy(xy_);
                    entries.push(entry(path, Some(orig), x, y, false));
                }
            }
            Some(b'u') => {
                let f: Vec<&str> = rec.splitn(11, ' ').collect();
                if let [_, xy_, .., path] = f[..] {
                    let (x, y) = xy(xy_);
                    entries.push(entry(path, None, x, y, true));
                }
            }
            Some(b'?') => {
                if let Some(path) = rec.strip_prefix("? ") {
                    entries.push(StatusEntry { untracked: true, ..entry(path, None, None, None, false) });
                }
            }
            _ => {}
        }
    }
    (header, entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_v2_parses_all_kinds() {
        let z = "# branch.oid abc\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 h h mod.txt\0\
1 A. N... 000000 100644 100644 h h added.txt\0\
1 .D N... 100644 100644 000000 h h gone.txt\0\
2 R. N... 100644 100644 100644 h h R100 new name.txt\0old name.txt\0\
u UU N... 100644 100644 100644 100644 h h h clash.txt\0\
? loose.txt\0";
        let (h, e) = parse_porcelain_v2(z.as_bytes());
        assert_eq!(h.branch.as_deref(), Some("main"));
        assert_eq!(e.len(), 6);
        assert_eq!((e[0].path.as_str(), e[0].staged, e[0].unstaged), ("mod.txt", None, Some('M')));
        assert_eq!((e[1].staged, e[1].unstaged), (Some('A'), None));
        assert_eq!((e[2].staged, e[2].unstaged), (None, Some('D')));
        assert_eq!((e[3].path.as_str(), e[3].orig_path.as_deref(), e[3].staged), ("new name.txt", Some("old name.txt"), Some('R')));
        assert!(e[4].conflict && e[4].path == "clash.txt" && e[4].staged == Some('U'));
        assert!(e[5].untracked && e[5].path == "loose.txt" && !e[5].conflict);
    }

    #[test]
    fn ahead_behind_from_header() {
        let (h, e) = parse_porcelain_v2(b"# branch.oid x\0# branch.head dev\0# branch.upstream origin/dev\0# branch.ab +3 -7\0");
        assert_eq!((h.ahead, h.behind, h.upstream.as_deref()), (3, 7, Some("origin/dev")));
        assert!(e.is_empty() && !h.detached);
        // senza upstream: nessuna riga branch.ab
        let (h, _) = parse_porcelain_v2(b"# branch.oid x\0# branch.head dev\0");
        assert_eq!((h.ahead, h.behind, h.upstream), (0, 0, None));
    }

    #[test]
    fn detached_head_header() {
        let (h, _) = parse_porcelain_v2(b"# branch.oid x\0# branch.head (detached)\0");
        assert!(h.detached && h.branch.is_none());
    }
}
