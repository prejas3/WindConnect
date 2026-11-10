//! Where an incoming file may go (PROTOCOL.md §9.1, paths). The path comes
//! from the other device, so the receiver checks it whatever the sender's
//! system, and creates nothing outside its inbox.
//!
//! CCS/1 SEC:peer-chosen names; refuse, never repair (only clashes get " (n)").

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use super::wire::MAX_PATH;

/// Longest part of a path, in UTF-8 bytes.
pub const MAX_PART: usize = 255;
/// Most parts in a path.
pub const MAX_DEPTH: usize = 32;

/// Why a path was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    Empty,
    TooLong,
    TooDeep,
    /// Empty, `.` or `..` part (also a leading `/`).
    BadPart,
    /// A character not allowed (`\`, `:`, `<>"|?*`, control, bidi).
    BadChar(char),
    /// A Windows device name (CON, NUL, COM1…), with or without extension.
    Reserved,
    /// A part ending in `.` or a space.
    TrailingDotOrSpace,
}

const BIDI: [char; 12] = [
    '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
    '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
];

fn bad_char(c: char) -> bool {
    matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        || c.is_control()
        || BIDI.contains(&c)
}

fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let (head, tail) = match upper.char_indices().nth(3) {
        Some((i, _)) => upper.split_at(i),
        None => return false,
    };
    (head == "COM" || head == "LPT")
        && matches!(
            tail,
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
}

/// Checks a relative path from an `Offer`; its parts on success.
pub fn check(path: &str) -> Result<Vec<&str>, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.len() > MAX_PATH {
        return Err(PathError::TooLong);
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return Err(PathError::TooDeep);
    }
    for part in &parts {
        if part.is_empty() || *part == "." || *part == ".." {
            return Err(PathError::BadPart);
        }
        if part.len() > MAX_PART {
            return Err(PathError::TooLong);
        }
        if let Some(c) = part.chars().find(|c| bad_char(*c)) {
            return Err(PathError::BadChar(c));
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Err(PathError::TrailingDotOrSpace);
        }
        if reserved(part) {
            return Err(PathError::Reserved);
        }
    }
    Ok(parts)
}

fn not_a_dir(path: &Path) -> io::Error {
    io::Error::other(format!("{} is not a plain folder", path.display()))
}

/// Creates the folders of `parts` (all but the last) under `inbox`, one at
/// a time, refusing any that is a link or junction; returns the folder.
fn make_parent(inbox: &Path, parts: &[&str]) -> io::Result<PathBuf> {
    let root = inbox.canonicalize()?;
    let mut dir = root.clone();
    for part in &parts[..parts.len() - 1] {
        dir.push(part);
        match fs::create_dir(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        // On Windows `is_symlink` covers name-surrogate reparse points
        // (symbolic links and junctions).
        let meta = fs::symlink_metadata(&dir)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(not_a_dir(&dir));
        }
    }
    if !dir.canonicalize()?.starts_with(&root) {
        return Err(not_a_dir(&dir));
    }
    Ok(dir)
}

/// `name` with " (n)" before its extension.
fn numbered(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_owned();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({n})"),
    }
}

/// Claims a free name for a checked path under `inbox`: creates an empty
/// file with `create_new` ("x", "x (1)", …), so nothing existing is ever
/// replaced and two transfers never get the same name. The caller renames
/// its finished file over the claimed one.
pub fn claim(inbox: &Path, parts: &[&str]) -> io::Result<(PathBuf, File)> {
    let dir = make_parent(inbox, parts)?;
    let name = parts[parts.len() - 1];
    for n in 0..10_000 {
        let candidate = numbered(name, n);
        if candidate.len() > MAX_PART {
            break;
        }
        let path = dir.join(&candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free name for the incoming file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_paths_are_refused() {
        for (path, why) in [
            ("", PathError::Empty),
            ("../x", PathError::BadPart),
            ("a/../../x", PathError::BadPart),
            ("./x", PathError::BadPart),
            ("a//x", PathError::BadPart),
            ("/x", PathError::BadPart),
            ("x/", PathError::BadPart),
            ("C:/x", PathError::BadChar(':')),
            ("x:ads", PathError::BadChar(':')),
            ("a\\..\\x", PathError::BadChar('\\')),
            ("a<b", PathError::BadChar('<')),
            ("a?", PathError::BadChar('?')),
            ("a\u{7}b", PathError::BadChar('\u{7}')),
            ("evil\u{202E}txt.exe", PathError::BadChar('\u{202E}')),
            ("nul", PathError::Reserved),
            ("NUL.txt", PathError::Reserved),
            ("dir/con.txt", PathError::Reserved),
            ("com1", PathError::Reserved),
            ("Lpt9.log", PathError::Reserved),
            ("com\u{b9}", PathError::Reserved),
            ("a.", PathError::TrailingDotOrSpace),
            ("a ", PathError::TrailingDotOrSpace),
            ("dir./x", PathError::TrailingDotOrSpace),
        ] {
            assert_eq!(check(path), Err(why), "{path:?}");
        }
        assert_eq!(check(&"a".repeat(256)), Err(PathError::TooLong));
        assert_eq!(check(&["a"; 33].join("/")), Err(PathError::TooDeep));
    }

    #[test]
    fn good_paths_pass() {
        for path in [
            "photo.jpg",
            "Zdjęcia/2026/IMG 1.jpg",
            "console.txt",
            "com10",
            "lpt",
            "nul_x",
            ".hidden",
            "a..b",
        ] {
            assert!(check(path).is_ok(), "{path:?}");
        }
    }

    #[test]
    fn names_are_claimed_without_replacing() {
        let inbox = tempfile::tempdir().unwrap();
        let parts = check("d/x.txt").unwrap();
        let (first, _) = claim(inbox.path(), &parts).unwrap();
        let (second, _) = claim(inbox.path(), &parts).unwrap();
        assert_eq!(first.file_name().unwrap(), "x.txt");
        assert_eq!(second.file_name().unwrap(), "x (1).txt");
        assert_eq!(numbered("noext", 2), "noext (2)");
        assert_eq!(numbered(".hidden", 1), ".hidden (1)");
    }

    #[test]
    fn simultaneous_claims_get_different_names() {
        let inbox = tempfile::tempdir().unwrap();
        let root = inbox.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || claim(&root, &["same.bin"]).unwrap().0)
            })
            .collect();
        let mut names: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 8);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_inbox_is_not_followed() {
        let inbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), inbox.path().join("d")).unwrap();
        assert!(claim(inbox.path(), &["d", "x"]).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_inside_the_inbox_is_not_followed() {
        let inbox = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = inbox.path().join("d");
        // A junction needs no privilege, unlike a symbolic link.
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        assert!(claim(inbox.path(), &["d", "x"]).is_err());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
