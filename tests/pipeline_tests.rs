//! Invariantes do pipeline multi-estágio.

mod common;

use common::{scan, write_copies, write_file};
use dedup::model::REPORT_FORMAT_VERSION;
use std::path::Path;

#[test]
fn identical_files_are_grouped_together() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"Conteudo absolutamente identico para o teste";
    write_copies(dir.path(), "copy", 2, payload);

    let report = scan(dir.path(), false);

    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(report.duplicate_groups[0].files.len(), 2);
    assert_eq!(report.total_recoverable_bytes, payload.len() as u64);
    assert_eq!(report.scanned_files, 2);
    assert_eq!(report.indexed_files, 2);
    assert_eq!(report.unique_sizes, 1);
    assert_eq!(report.format_version, REPORT_FORMAT_VERSION);
}

#[test]
fn different_content_with_same_size_is_not_grouped() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir.path().join("a.bin"), b"AAAAAAAAAAAAAAAA");
    write_file(&dir.path().join("b.bin"), b"BBBBBBBBBBBBBBBB");

    let report = scan(dir.path(), false);

    assert!(report.duplicate_groups.is_empty());
    assert_eq!(report.total_recoverable_bytes, 0);
    // Ambos sobrevivem ao filtro de tamanho, mas o hash parcial já os separa.
    assert_eq!(report.unique_sizes, 1);
}

#[test]
fn three_way_group_reports_two_recoverable_copies() {
    let dir = tempfile::tempdir().unwrap();
    let payload = vec![0x42_u8; 1000];
    write_copies(dir.path(), "tri", 3, &payload);

    let report = scan(dir.path(), false);

    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(report.duplicate_groups[0].files.len(), 3);
    assert_eq!(report.total_recoverable_bytes, 2 * 1000);
}

#[test]
fn partial_hash_collision_does_not_create_a_group() {
    let dir = tempfile::tempdir().unwrap();

    // Mesmos 4 KiB iniciais, mesmo tamanho, caudas divergentes.
    let mut first = vec![0xAA_u8; 4096];
    let mut second = vec![0xAA_u8; 4096];
    first.extend_from_slice(b"DIFERENCA_NO_FINAL_1");
    second.extend_from_slice(b"DIFERENCA_NO_FINAL_2");
    assert_eq!(first.len(), second.len());

    write_file(&dir.path().join("coll_1.bin"), &first);
    write_file(&dir.path().join("coll_2.bin"), &second);

    let report = scan(dir.path(), false);

    assert!(
        report.duplicate_groups.is_empty(),
        "arquivos com o mesmo prefixo de 4 KiB e finais diferentes não são duplicatas"
    );
    // O hash completo precisou ler os dois arquivos por inteiro, depois dos 4 KiB
    // gastos no estágio anterior.
    assert_eq!(report.bytes_hashed, 2 * (4096 + first.len() as u64));
}

#[test]
fn partial_hash_collision_still_finds_the_real_duplicate() {
    let dir = tempfile::tempdir().unwrap();

    // `a` e `b` são idênticos entre si; `c` compartilha o prefixo mas difere.
    let mut shared = vec![0x11_u8; 4096];
    shared.extend_from_slice(b"cauda comum a e b");
    let mut different = vec![0x11_u8; 4096];
    different.extend_from_slice(b"cauda somente do c");

    write_file(&dir.path().join("a.bin"), &shared);
    write_file(&dir.path().join("b.bin"), &shared);
    write_file(&dir.path().join("c.bin"), &different);

    let report = scan(dir.path(), false);

    assert_eq!(report.duplicate_groups.len(), 1);
    let group = &report.duplicate_groups[0];
    assert_eq!(group.files.len(), 2);
    assert!(contains_file(group, "a.bin"));
    assert!(contains_file(group, "b.bin"));
    assert!(!contains_file(group, "c.bin"));
}

#[test]
fn empty_files_obey_include_empty() {
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir.path().join("empty1.txt"), b"");
    write_file(&dir.path().join("empty2.txt"), b"");

    let without = scan(dir.path(), false);
    assert!(without.duplicate_groups.is_empty());
    assert_eq!(without.skipped_empty, 2);
    assert_eq!(without.scanned_files, 2);
    assert_eq!(without.indexed_files, 0);

    let with = scan(dir.path(), true);
    assert_eq!(with.duplicate_groups.len(), 1);
    assert_eq!(with.duplicate_groups[0].files.len(), 2);
    // Remover arquivos vazios não devolve espaço algum.
    assert_eq!(with.total_recoverable_bytes, 0);
    assert_eq!(with.skipped_empty, 0);
}

#[test]
fn groups_are_sorted_by_recoverable_space() {
    let dir = tempfile::tempdir().unwrap();
    write_copies(dir.path(), "pequeno", 2, &[0x01_u8; 100]);
    write_copies(dir.path(), "grande", 2, &[0x02_u8; 10_000]);

    let report = scan(dir.path(), false);

    assert_eq!(report.duplicate_groups.len(), 2);
    assert_eq!(report.duplicate_groups[0].file_size, 10_000);
    assert_eq!(report.duplicate_groups[1].file_size, 100);
    assert_eq!(report.total_recoverable_bytes, 10_100);
}

#[test]
fn io_accounting_reflects_the_pipeline_stages() {
    let dir = tempfile::tempdir().unwrap();
    // Tamanho único => descartado antes de qualquer leitura de conteúdo.
    write_file(&dir.path().join("unico.bin"), &vec![0x01_u8; 5000]);
    // Duas cópias de 10 KiB: 4 KiB de hash parcial + 10 KiB de hash completo cada.
    write_copies(dir.path(), "dup", 2, &vec![0x02_u8; 10 * 1024]);

    let report = scan(dir.path(), false);

    assert_eq!(report.unique_sizes, 2);
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(report.bytes_hashed, 2 * (4096 + 10 * 1024));
    // O baseline ingênuo leria 2 * 5000 + 2 * 10240 bytes.
    assert!(report.bytes_hashed < 2 * 5000 + 2 * 10 * 1024);
}

fn contains_file(group: &dedup::model::DuplicateGroup, name: &str) -> bool {
    group
        .files
        .iter()
        .any(|path| path.file_name() == Some(Path::new(name).as_os_str()))
}
