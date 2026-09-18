//! Manifesto JSON: round-trip, busca por hash e auditoria de integridade.

mod common;

use common::{scan, write_copies, write_file};
use dedup::report::{self, MIN_HASH_PREFIX};
use dedup::verify::verify_report;
use std::fs;

#[test]
fn report_round_trips_through_json_and_supports_find() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"payload para o manifesto";
    write_copies(dir.path(), "copy", 2, payload);

    let original = scan(dir.path(), false);
    // O diretório-pai não existe: `save_report` deve criá-lo.
    let report_path = dir.path().join("nested").join("manifesto.json");
    report::save_report(&original, &report_path).unwrap();

    let loaded = report::load_report(&report_path).unwrap();
    assert_eq!(loaded, original);

    let full_hash = &original.duplicate_groups[0].full_hash;
    assert_eq!(full_hash.len(), 64, "BLAKE3 em hex tem 64 caracteres");
    assert_eq!(report::find_groups(&loaded, full_hash).len(), 1);
    assert_eq!(
        report::find_groups(&loaded, &full_hash[..MIN_HASH_PREFIX]).len(),
        1,
        "prefixo de 8 caracteres localiza o grupo"
    );
    assert_eq!(
        report::find_groups(&loaded, &full_hash.to_ascii_uppercase()).len(),
        1,
        "a busca é case-insensitive"
    );
    assert!(report::find_groups(&loaded, "deadbeef").is_empty());
    assert!(report::find_groups(&loaded, "").is_empty());
    assert!(report::load_report(&dir.path().join("inexistente.json")).is_err());
}

#[test]
fn report_without_optional_fields_is_still_loadable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("antigo.json");
    // Manifesto no formato da v0.1: sem os campos estatísticos aditivos.
    let legacy = r#"{"target_path": "/tmp/x", "unique_sizes": 1, "duplicate_groups": [
             {"full_hash": "aa", "file_size": 10, "files": ["/tmp/x/a", "/tmp/x/b"]}],
             "total_recoverable_bytes": 10, "elapsed_seconds": 0.5}"#;
    fs::write(&path, legacy).unwrap();

    let loaded = report::load_report(&path).unwrap();

    assert_eq!(loaded.format_version, 0);
    assert_eq!(loaded.unique_sizes, 1);
    assert_eq!(loaded.duplicate_groups.len(), 1);
    assert_eq!(loaded.bytes_hashed, 0);
    assert_eq!(loaded.skipped_hidden, 0);
}

#[test]
fn verify_reports_clean_groups_as_verified() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"conteudo original do arquivo";
    write_file(&dir.path().join("a.bin"), payload);
    write_file(&dir.path().join("b.bin"), payload);
    write_file(&dir.path().join("c.bin"), payload);

    let report_path = dir.path().join("manifesto.json");
    report::save_report(&scan(dir.path(), false), &report_path).unwrap();

    let summary = verify_report(&report_path).unwrap();

    assert_eq!(summary.groups, 1);
    assert_eq!(summary.files, 3);
    assert_eq!(summary.verified, 3);
    assert!(!summary.has_problems());
}

#[test]
fn verify_detects_changed_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"conteudo original do arquivo";
    let first = write_file(&dir.path().join("a.bin"), payload);
    let second = write_file(&dir.path().join("b.bin"), payload);
    write_file(&dir.path().join("c.bin"), payload);

    let report_path = dir.path().join("manifesto.json");
    report::save_report(&scan(dir.path(), false), &report_path).unwrap();

    // Corrompe um arquivo (mesmo tamanho, conteúdo diferente) e remove outro.
    write_file(&first, b"conteudo ALTERADO do arquivo");
    fs::remove_file(&second).unwrap();

    let damaged = verify_report(&report_path).unwrap();

    assert_eq!(damaged.files, 3);
    assert_eq!(damaged.verified, 1);
    assert_eq!(damaged.changed, 1);
    assert_eq!(damaged.missing, 1);
    assert_eq!(damaged.unreadable, 0);
    assert!(damaged.has_problems());
}

#[test]
fn recoverable_space_keeps_one_copy() {
    let dir = tempfile::tempdir().unwrap();
    let payload = vec![0x33_u8; 700];
    write_copies(dir.path(), "copy", 4, &payload);

    let report = scan(dir.path(), false);
    let group = &report.duplicate_groups[0];

    assert_eq!(group.recoverable_space(), 3 * 700);
    assert_eq!(report.total_recoverable_bytes, group.recoverable_space());
}
