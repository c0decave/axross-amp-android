//! What a picker answered: the documents chosen, as the Java side writes
//! them (`Bridge.line`) - one line a track, `uri TAB folder TAB name TAB
//! size` (the size the provider gives, -1 for none; older lines end with
//! the name).

use std::cmp::Ordering;

/// A track chosen: where it is (a content:// URI on Android, a path on the
/// host), the folder it lies in below the one picked ("" at the top) and
/// its name as the provider shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picked {
    pub uri: String,
    pub folder: String,
    pub name: String,
    /// What the provider says the document holds (None: it does not say).
    pub size: Option<u64>,
}

/// The lines of a picker's answer; a line without a URI is none.
pub fn parse(text: &str) -> Vec<Picked> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '\t');
            let uri = parts.next()?.trim();
            if uri.is_empty() {
                return None;
            }
            let folder = parts.next().unwrap_or("").trim();
            let name = parts.next().unwrap_or("").trim();
            let size = parts
                .next()
                .and_then(|size| size.trim().parse::<i64>().ok())
                .and_then(|size| u64::try_from(size).ok());
            Some(Picked {
                uri: uri.to_owned(),
                folder: folder.to_owned(),
                name: name.to_owned(),
                size,
            })
        })
        .collect()
}

/// In the order a folder is read in: folder by folder, by name within it,
/// both as `cmp` orders text (the desktop's natural order: 2 before 10).
pub fn sort(tracks: &mut [Picked], cmp: impl Fn(&str, &str) -> Ordering) {
    tracks.sort_by(|a, b| cmp(&a.folder, &b.folder).then_with(|| cmp(&a.name, &b.name)));
}

/// The name's ending, if it is one a file may safely carry in the cache
/// (letters and digits, at most five): the decoder's hint of the format.
pub fn extension(name: &str) -> Option<String> {
    let (stem, ending) = name.rsplit_once('.')?;
    if stem.is_empty()
        || ending.is_empty()
        || ending.len() > 5
        || !ending.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(ending.to_ascii_lowercase())
}

/// The name without its ending: the title until the tags are read.
pub fn title(name: &str) -> String {
    match (extension(name), name.rsplit_once('.')) {
        (Some(_), Some((stem, _))) => stem.to_owned(),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_become_tracks() {
        let text = "content://a/1\tAlbum\t01 One.flac\ncontent://a/2\t\tTwo.mp3\n\n\t\tno uri\n";
        let got = parse(text);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].uri, "content://a/1");
        assert_eq!(got[0].folder, "Album");
        assert_eq!(got[0].name, "01 One.flac");
        assert_eq!(got[1].folder, "");
        assert_eq!(got[1].name, "Two.mp3");
    }

    #[test]
    fn a_line_with_only_a_uri_is_a_track_without_a_name() {
        let got = parse("content://a/9");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "");
    }

    #[test]
    fn a_line_may_carry_the_providers_size() {
        let got = parse("u\tf\ta.mp3\t12345\nv\t\tb.mp3\t-1\nw\t\tc.mp3\tnone\n");
        assert_eq!(got[0].name, "a.mp3");
        assert_eq!(got[0].size, Some(12345));
        assert_eq!(got[1].size, None);
        assert_eq!(got[2].size, None);
        // (a line without one: none)
        assert_eq!(parse("u\tf\ta.mp3")[0].size, None);
    }

    #[test]
    fn sorted_folder_by_folder() {
        let mut got = parse("u3\tB\t1.mp3\nu2\tA\t2.mp3\nu1\tA\t1.mp3\nu0\t\tz.mp3\n");
        sort(&mut got, |a, b| a.cmp(b));
        let uris: Vec<&str> = got.iter().map(|p| p.uri.as_str()).collect();
        assert_eq!(uris, ["u0", "u1", "u2", "u3"]);
    }

    #[test]
    fn endings_and_titles() {
        assert_eq!(extension("01 One.FLAC").as_deref(), Some("flac"));
        assert_eq!(extension("no ending"), None);
        assert_eq!(extension(".hidden"), None);
        assert_eq!(extension("a.b/c"), None);
        assert_eq!(extension("a.toolong"), None);
        assert_eq!(title("01 One.flac"), "01 One");
        assert_eq!(title("Mr. Blue Sky"), "Mr. Blue Sky");
        assert_eq!(title("track"), "track");
    }
}
