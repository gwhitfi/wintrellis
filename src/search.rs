//! Matching launcher apps against what's typed in the search box. Pure (no Win32
//! calls), so it's unit-tested here.

/// Indexes into `names` of the ones that match `query`, best match first:
/// 1. names that start with it ("calc" -> "Calculator")
/// 2. names with a later word that starts with it ("code" -> "VS Code")
/// 3. names that contain it anywhere ("plore" -> "File Explorer")
///
/// Within each group the settings file's order is kept. Case doesn't matter, and an
/// empty query matches everything.
pub fn filter(names: &[&str], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<(u8, usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| rank(&name.to_lowercase(), &query).map(|r| (r, i)))
        .collect();
    // Stable sort, so equal ranks stay in file order.
    matches.sort_by_key(|&(rank, _)| rank);
    matches.into_iter().map(|(_, i)| i).collect()
}

/// 0 is the best match; `None` means no match. Both strings are already lowercase.
fn rank(name: &str, query: &str) -> Option<u8> {
    if name.starts_with(query) {
        return Some(0);
    }
    let is_separator = |c: char| c.is_whitespace() || matches!(c, '-' | '_' | '.');
    if name.split(is_separator).any(|word| word.starts_with(query)) {
        return Some(1);
    }
    name.contains(query).then_some(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPS: [&str; 6] = [
        "Firefox",
        "VS Code",
        "Calculator",
        "File Explorer",
        "Xcode-ish",
        "Discord",
    ];

    fn names(query: &str) -> Vec<&'static str> {
        filter(&APPS, query).into_iter().map(|i| APPS[i]).collect()
    }

    #[test]
    fn empty_query_matches_everything_in_order() {
        assert_eq!(names(""), APPS);
        assert_eq!(names("   "), APPS);
    }

    #[test]
    fn prefix_match() {
        assert_eq!(names("calc"), ["Calculator"]);
        assert_eq!(names("CALC"), ["Calculator"]);
    }

    #[test]
    fn start_of_name_then_word_then_anywhere() {
        // "Firefox" and "File Explorer" start with "fi"; nothing else has it.
        assert_eq!(names("fi"), ["Firefox", "File Explorer"]);
        // "VS Code" has a word starting with "code"; "Xcode-ish" only contains it.
        assert_eq!(names("code"), ["VS Code", "Xcode-ish"]);
        // "Discord" starts with "d"; "VS Code" has "d" inside; ranks put Discord first.
        assert_eq!(names("d")[0], "Discord");
    }

    #[test]
    fn spaces_inside_the_query_count() {
        assert_eq!(names("vs c"), ["VS Code"]);
    }

    #[test]
    fn no_match() {
        assert!(names("zzz").is_empty());
    }
}
