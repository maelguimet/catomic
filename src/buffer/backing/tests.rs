//! Purpose: pin change-time decisions and fail-closed promotion of provisional baselines.
//! Owns: injected-clock fixtures and same-length restored-mtime rewrites.
//! Must not: sleep for whole-second granularity or depend on the host clock tick.
//! Invariants: every fixture file is removed by the test that created it.

use std::io::{Seek, Write};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::*;

struct Fixture {
    path: std::path::PathBuf,
    file: File,
}

impl Fixture {
    fn new(label: &str, text: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "catomic_backing_{label}_{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        Self { path, file }
    }

    fn digest_all(&self) -> RangeDigester {
        let mut digest = RangeDigester::new(0);
        let bytes = std::fs::read(&self.path).unwrap();
        digest.observe(0, &bytes[..3]).unwrap();
        digest.observe(2, &bytes[2..7]).unwrap();
        digest.observe(7, &bytes[7..]).unwrap();
        digest
    }

    fn rewrite_same_length_keeping_mtime(&mut self, text: &str) {
        let modified = self.file.metadata().unwrap().modified().unwrap();
        self.file.rewind().unwrap();
        self.file.write_all(text.as_bytes()).unwrap();
        self.file.set_modified(modified).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        TEST_NOW.with(|now| now.set(None));
    }
}

fn changed_at(snapshot: &DescriptorSnapshot) -> SystemTime {
    UNIX_EPOCH
        + Duration::new(
            snapshot.changed_seconds as u64,
            snapshot.changed_nanoseconds as u32,
        )
}

fn set_now(now: SystemTime) {
    TEST_NOW.with(|cell| cell.set(Some(now)));
}

#[test]
fn change_time_granularity_is_one_tick_or_two_seconds_for_whole_second_stamps() {
    let fixture = Fixture::new("granularity", "abc");
    let precise = DescriptorSnapshot {
        changed_nanoseconds: 500_000_000,
        ..DescriptorSnapshot::capture(&fixture.file).unwrap()
    };
    let whole_second = DescriptorSnapshot {
        changed_nanoseconds: 0,
        ..precise
    };

    set_now(changed_at(&precise) + Duration::from_millis(5));
    assert_eq!(
        precise.change_time(),
        ChangeTime::Pending(Duration::from_millis(6))
    );
    set_now(changed_at(&precise) + Duration::from_millis(11));
    assert_eq!(precise.change_time(), ChangeTime::Settled);

    set_now(changed_at(&whole_second) + Duration::from_millis(500));
    assert_eq!(
        whole_second.change_time(),
        ChangeTime::Pending(Duration::from_millis(1_500))
    );
    set_now(changed_at(&whole_second) + Duration::from_secs(2));
    assert_eq!(whole_second.change_time(), ChangeTime::Settled);
}

#[test]
fn whole_second_pending_baseline_is_refused_immediately_instead_of_waited() {
    let fixture = Fixture::new("refuse_whole_second", "first\nsecond\nthird");
    let snapshot = DescriptorSnapshot {
        changed_nanoseconds: 0,
        ..DescriptorSnapshot::capture(&fixture.file).unwrap()
    };
    let trust = BaselineTrust::provisional(fixture.digest_all());
    set_now(changed_at(&snapshot));
    SETTLE_WAITS.with(|waits| waits.set(0));

    let started = Instant::now();
    let error = trust.require(&snapshot, &fixture.file).unwrap_err();

    assert!(started.elapsed() < Duration::from_millis(50));
    assert_eq!(SETTLE_WAITS.with(std::cell::Cell::get), 0);
    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert_eq!(
        error.to_string(),
        "File changed moments ago; try again in 2.0s once its timestamp settles."
    );
    trust.refresh(&snapshot, &fixture.file).unwrap();
    assert!(!trust.promoted.get(), "pending refresh must not promote");
}

#[test]
fn future_change_time_fails_closed_with_an_explanation() {
    let fixture = Fixture::new("future", "first\nsecond\nthird");
    let snapshot = DescriptorSnapshot::capture(&fixture.file).unwrap();
    let trust = BaselineTrust::provisional(fixture.digest_all());
    set_now(changed_at(&snapshot) - Duration::from_secs(60));

    let error = trust.require(&snapshot, &fixture.file).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
    assert!(error.to_string().contains("in the future"), "{error}");
    assert!(!BackingFileChanged::is(&error));
}

#[test]
fn settled_provisional_baseline_exposes_a_same_length_rewrite_with_restored_mtime() {
    let mut fixture = Fixture::new("rewrite", "first\nsecond\nthird");
    let snapshot = DescriptorSnapshot::capture(&fixture.file).unwrap();
    let trust = BaselineTrust::provisional(fixture.digest_all());

    fixture.rewrite_same_length_keeping_mtime("FIRST\nSECOND\nTHIRD");
    set_now(changed_at(&snapshot) + Duration::from_secs(3));

    let error = trust.require(&snapshot, &fixture.file).unwrap_err();
    assert!(BackingFileChanged::is(&error), "{error}");
    let error = trust.refresh(&snapshot, &fixture.file).unwrap_err();
    assert!(BackingFileChanged::is(&error), "{error}");
    assert!(!trust.promoted.get());
}

#[test]
fn unchanged_provisional_baseline_is_promoted_once_its_tick_passes() {
    let fixture = Fixture::new("promote", "first\nsecond\nthird");
    let snapshot = DescriptorSnapshot::capture(&fixture.file).unwrap();
    let trust = BaselineTrust::provisional(fixture.digest_all());
    set_now(changed_at(&snapshot) + Duration::from_secs(3));

    trust.refresh(&snapshot, &fixture.file).unwrap();

    assert!(trust.promoted.get());
    set_now(changed_at(&snapshot));
    trust.require(&snapshot, &fixture.file).unwrap();
}

#[test]
fn digest_is_independent_of_how_reads_were_chunked() {
    let fixture = Fixture::new("chunking", "first\nsecond\nthird");
    let mut whole = RangeDigester::new(0);
    whole
        .observe(0, &std::fs::read(&fixture.path).unwrap())
        .unwrap();
    let whole = whole.finish();
    let pieces = fixture.digest_all().finish();
    assert!(whole.matches(&fixture.file).unwrap());
    assert!(pieces.matches(&fixture.file).unwrap());
    let mut gap = RangeDigester::new(0);
    assert!(gap.observe(4, b"x").is_err());
}
