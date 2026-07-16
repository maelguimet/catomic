//! Purpose: verify App-level next/previous search across descriptor-backed pages.
//! Owns: the focused asynchronous navigation fixture.
//! Must not: contain production behavior or broad search integration coverage.
//! Invariants: waits are bounded and temporary files are removed after the test.
//! Phase: 3-b paged search navigation.

use super::*;

#[test]
fn whole_file_search_navigates_next_previous_and_wraps() {
    let path = std::env::temp_dir().join(format!(
        "catomic_whole_search_navigation_{}.txt",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "target zero\ntarget one\ntarget two").unwrap();
    let mut app = super::super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PagedFileBuffer::open(&path, 1).unwrap());
    let mut out = Vec::new();

    enter_query(&mut app, "target", &mut out);
    wait_for_search(&mut app, &mut out);
    assert_eq!(app.buffer.page_info().unwrap().page_number, 1);

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    wait_for_search(&mut app, &mut out);
    assert_eq!(app.buffer.page_info().unwrap().page_number, 2);

    app.handle_key_with(&mut out, key(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    wait_for_search(&mut app, &mut out);
    assert_eq!(app.buffer.page_info().unwrap().page_number, 1);

    app.handle_key_with(&mut out, key(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    wait_for_search(&mut app, &mut out);
    assert_eq!(app.buffer.page_info().unwrap().page_number, 3);

    let _ = std::fs::remove_file(path);
}

fn wait_for_search(app: &mut super::super::super::App, out: &mut Vec<u8>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while app.search.running.is_some() && std::time::Instant::now() < deadline {
        poll_search(app, out).unwrap();
        std::thread::yield_now();
    }
    assert!(app.search.running.is_none(), "search did not complete");
}
