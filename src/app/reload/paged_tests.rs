use super::*;

fn pinned_then_maybe_replaced(name: &str, replace: bool) -> io::Result<()> {
    let dir = std::env::temp_dir().join(format!(
        "catomic_paged_reload_{}_{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.txt");
    std::fs::write(&path, "first\nsecond\n").unwrap();
    let source = crate::file::io::PinnedFile::open(&path).unwrap().unwrap();
    if replace {
        let replacement = dir.join("replacement.txt");
        std::fs::write(&replacement, "other\n").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
    }
    let result = paged_reload_buffer(&path, source, 1).map(|_| ());
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn paged_reload_accepts_the_pinned_revision() {
    pinned_then_maybe_replaced("kept", false).unwrap();
}

#[test]
fn paged_reload_refuses_a_path_replaced_after_the_descriptor_was_pinned() {
    let error = pinned_then_maybe_replaced("replaced", true).unwrap_err();
    assert!(error.to_string().contains("doc.txt"), "{error}");
}
