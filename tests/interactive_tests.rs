//! Núcleo destrutivo do modo interativo, testado sem terminal.

mod common;

use common::write_copies;
use dedup::interactive::apply_removal;

#[test]
fn dry_run_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let files = write_copies(dir.path(), "copy", 3, b"duplicado");

    let outcome = apply_removal(&files, 0, true).unwrap();

    assert_eq!(outcome.pending, 2);
    assert_eq!(outcome.removed, 0);
    assert!(outcome.failed.is_empty());
    assert_eq!(outcome.total_processed(), 2);
    assert!(files.iter().all(|path| path.exists()));
}

#[test]
fn apply_deletes_everything_but_the_kept_file() {
    let dir = tempfile::tempdir().unwrap();
    let files = write_copies(dir.path(), "copy", 3, b"duplicado");

    let outcome = apply_removal(&files, 1, false).unwrap();

    assert_eq!(outcome.removed, 2);
    assert_eq!(outcome.pending, 0);
    assert!(outcome.failed.is_empty());
    assert!(files[1].exists());
    assert!(!files[0].exists());
    assert!(!files[2].exists());
}

#[test]
fn invalid_selection_errors_without_touching_files() {
    let dir = tempfile::tempdir().unwrap();
    let files = write_copies(dir.path(), "copy", 2, b"duplicado");

    assert!(apply_removal(&files, 2, false).is_err());
    assert!(apply_removal(&[], 0, false).is_err());
    assert!(files.iter().all(|path| path.exists()));
}

#[test]
fn missing_file_is_recorded_as_a_failure_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let files = write_copies(dir.path(), "copy", 2, b"duplicado");
    std::fs::remove_file(&files[0]).unwrap();

    let outcome = apply_removal(&files, 1, false).unwrap();

    assert_eq!(outcome.removed, 0);
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(outcome.failed[0].0, files[0]);
    assert!(files[1].exists(), "o arquivo preservado continua intacto");
}

#[test]
fn single_file_group_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    let files = write_copies(dir.path(), "copy", 1, b"unico");

    let outcome = apply_removal(&files, 0, false).unwrap();

    assert_eq!(outcome.total_processed(), 0);
    assert!(files[0].exists());
}
