use super::*;

struct Directory(PathBuf);
impl Directory {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("catomic_completion_{label}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, suffix: &str) -> String {
        format!("{}/{suffix}", self.0.display())
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn completes_spaces_unicode_directories_and_a_component_at_the_caret() {
    let dir = Directory::new("unique");
    std::fs::write(dir.0.join("猫 notes.txt"), "").unwrap();
    std::fs::create_dir(dir.0.join("folder name")).unwrap();
    let input = dir.path("猫 n");
    let found = complete(&input, input.len(), None).unwrap();
    assert_eq!(found.text, "猫 notes.txt");
    assert!(found.notice.is_none());
    let input = dir.path("folTYPO/child.txt");
    let found = complete(&input, dir.path("fol").len(), None).unwrap();
    assert_eq!(found.text, "folder name");
    assert_eq!(&input[found.end..], "/child.txt");
    let input = dir.path("fol");
    assert_eq!(
        complete(&input, input.len(), None).unwrap().text,
        "folder name/"
    );
}

#[test]
fn ambiguity_extends_only_common_graphemes_and_reports_no_matches() {
    let dir = Directory::new("ambiguous");
    for name in [
        "common-one",
        "common-two",
        "a\u{301}bc",
        "a\u{308}de",
        ".hidden",
    ] {
        std::fs::write(dir.0.join(name), "").unwrap();
    }
    let input = dir.path("com");
    let found = complete(&input, input.len(), None).unwrap();
    assert_eq!(found.text, "common-");
    assert!(found.notice.unwrap().contains("2 matching"));
    let input = dir.path("a");
    assert!(complete(&input, input.len(), None).is_err());
    let input = dir.path("missing");
    assert_eq!(
        complete(&input, input.len(), None).err().unwrap(),
        "No matching paths."
    );
    let input = dir.path(".h");
    assert_eq!(complete(&input, input.len(), None).unwrap().text, ".hidden");
}

#[test]
fn incomplete_directory_enumeration_never_claims_a_unique_match() {
    let dir = Directory::new("limit");
    for name in ["one", "two", "three"] {
        std::fs::write(dir.0.join(name), "").unwrap();
    }
    let input = dir.path("o");
    let result = complete_with_limit(&input, input.len(), None, 1);
    assert!(result.err().unwrap().contains("scan limit"));
}

#[test]
fn home_expansion_is_only_for_lookup_and_missing_directory_is_reported() {
    let dir = Directory::new("home");
    std::fs::write(dir.0.join("notes"), "").unwrap();
    let input = "~/no";
    let found = complete(input, input.len(), Some(dir.0.as_os_str())).unwrap();
    assert_eq!(found.start, 2);
    assert_eq!(found.text, "notes");
    assert_eq!(
        complete("~", 1, Some(dir.0.as_os_str())).unwrap().text,
        "~/"
    );
    assert!(complete(input, input.len(), None)
        .err()
        .unwrap()
        .contains("HOME"));
    let input = dir.path("missing/no");
    assert!(complete(&input, input.len(), None)
        .err()
        .unwrap()
        .contains("Completion error"));
}
