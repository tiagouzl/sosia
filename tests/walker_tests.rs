//! Invariantes da varredura: hard links, entradas ocultas e resiliência.

mod common;

use common::{scan, scan_with, write_copies, write_file};
use sosia::pipeline::EngineOptions;
use sosia::walker::{scan_directory, WalkOptions, WalkStats};
use std::fs;
use std::path::Path;

#[cfg(unix)]
#[test]
fn hard_links_are_counted_once_and_do_not_inflate_recoverable_space() {
    let dir = tempfile::tempdir().unwrap();
    let payload = vec![0x7F_u8; 2048];

    let original = write_file(&dir.path().join("original.bin"), &payload);
    let copy = write_file(&dir.path().join("copia-real.bin"), &payload);
    let hard_link = dir.path().join("hard-link.bin");
    fs::hard_link(&original, &hard_link).unwrap();

    let report = scan(dir.path(), false);

    // 3 arquivos no disco, 2 entradas indexadas: o hard link foi ignorado.
    assert_eq!(report.scanned_files, 3);
    assert_eq!(report.indexed_files, 2);
    assert_eq!(report.skipped_hard_links, 1);

    // Só uma das entradas do mesmo inode é espaço recuperável — elas dividem
    // os mesmos blocos no disco. Qual caminho fica depende da ordem de
    // `readdir`, então o teste é agnóstico quanto a isso.
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(report.duplicate_groups[0].files.len(), 2);
    assert_eq!(report.total_recoverable_bytes, payload.len() as u64);
    assert!(report.duplicate_groups[0].files.contains(&copy));
    assert!(
        !(report.duplicate_groups[0].files.contains(&original)
            && report.duplicate_groups[0].files.contains(&hard_link)),
        "as duas entradas do mesmo inode não podem entrar juntas no grupo"
    );
}

#[cfg(unix)]
#[test]
fn walker_reports_hard_links_in_stats() {
    let dir = tempfile::tempdir().unwrap();
    let original = write_file(&dir.path().join("a.txt"), b"conteudo");
    fs::hard_link(&original, dir.path().join("link.txt")).unwrap();

    let outcome = scan_directory(dir.path(), WalkOptions::default());

    assert_eq!(
        outcome.stats,
        WalkStats {
            scanned_files: 2,
            indexed_files: 1,
            skipped_empty: 0,
            skipped_hard_links: 1,
            skipped_hidden: 0,
            walk_errors: 0,
        }
    );
}

#[test]
fn hidden_entries_require_include_hidden() {
    // OBS.: `tempdir()` cria um diretório `.tmpXXXX`, ou seja, a própria raiz é
    // oculta — exatamente o caso que a filtragem sem checagem de profundidade
    // quebrava (a raiz era podada e a varredura devolvia zero arquivos).
    let dir = tempfile::tempdir().unwrap();
    let payload = b"tres copias identicas";
    write_copies(dir.path(), "visivel", 2, payload);
    write_file(&dir.path().join(".oculto.bin"), payload);

    let default_scan = scan(dir.path(), false);
    assert_eq!(default_scan.duplicate_groups.len(), 1);
    assert_eq!(default_scan.duplicate_groups[0].files.len(), 2);
    assert_eq!(default_scan.skipped_hidden, 1);
    assert_eq!(default_scan.indexed_files, 2);

    let inclusive = scan_with(
        dir.path(),
        EngineOptions {
            include_empty: false,
            include_hidden: true,
        },
    );
    assert_eq!(inclusive.duplicate_groups[0].files.len(), 3);
    assert_eq!(inclusive.skipped_hidden, 0);
}

#[test]
fn hidden_root_directory_is_still_scanned() {
    let dir = tempfile::tempdir().unwrap();
    let hidden_root = dir.path().join(".backup");
    fs::create_dir_all(&hidden_root).unwrap();
    write_copies(&hidden_root, "copy", 2, b"conteudo dentro de raiz oculta");

    let report = scan(&hidden_root, false);

    assert_eq!(
        report.duplicate_groups.len(),
        1,
        "a raiz indicada pelo usuário deve ser varrida mesmo sendo oculta"
    );
    assert_eq!(report.duplicate_groups[0].files.len(), 2);
}

#[test]
fn hidden_directories_are_pruned_entirely() {
    let dir = tempfile::tempdir().unwrap();
    write_copies(dir.path(), "visivel", 2, b"x");
    let hidden_dir = dir.path().join(".cache");
    fs::create_dir_all(&hidden_dir).unwrap();
    write_file(&hidden_dir.join("a.bin"), b"y");
    write_file(&hidden_dir.join("b.bin"), b"y");

    let report = scan(dir.path(), false);

    // Um grupo (os dois visíveis); o diretório oculto foi podado inteiro.
    assert_eq!(report.duplicate_groups.len(), 1);
    assert_eq!(report.scanned_files, 2);
    assert_eq!(report.skipped_hidden, 1);
}

#[test]
fn nonexistent_root_fails_validation_instead_of_an_empty_success() {
    let ghost = "/tmp/nao-existe-sosia-definitivamente";

    // O walkdir em si só registra um erro de iteração (a varredura continua —
    // comportamento correto para erros no meio da árvore).
    let outcome = scan_directory(Path::new(ghost), WalkOptions::default());
    assert_eq!(outcome.stats.walk_errors, 1);
    assert_eq!(outcome.stats.scanned_files, 0);

    // A validação explícita transforma isso em um erro do usuário, que o CLI
    // reporta com exit 2 em vez de "nenhuma duplicata".
    assert!(sosia::walker::ensure_scannable(ghost).is_err());

    let dir = tempfile::tempdir().unwrap();
    assert!(sosia::walker::ensure_scannable(dir.path()).is_ok());
    let file = write_file(&dir.path().join("arq.txt"), b"x");
    assert!(
        sosia::walker::ensure_scannable(&file).is_err(),
        "arquivo regular não é uma raiz válida"
    );
}

#[test]
fn symlinks_are_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let payload = b"conteudo apontado por symlink";
    let target = write_file(&dir.path().join("alvo.bin"), payload);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, dir.path().join("atalho.bin")).unwrap();

    let report = scan(dir.path(), false);

    // Como symlinks não são seguidos, o atalho não é indexado: sem duplicatas.
    assert_eq!(report.scanned_files, 1);
    assert_eq!(report.indexed_files, 1);
    assert!(report.duplicate_groups.is_empty());
}

#[test]
fn unreadable_directories_do_not_abort_the_walk() {
    let dir = tempfile::tempdir().unwrap();
    write_copies(dir.path(), "copy", 2, b"conteudo valido");
    let blocked = dir.path().join("bloqueado");
    fs::create_dir_all(&blocked).unwrap();
    write_file(&blocked.join("segredo.bin"), b"conteudo valido");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Sem permissão de leitura/execução: o walkdir devolve erro e seguimos.
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    }

    let report = scan(dir.path(), false);

    // Rodando como root (comum em contêineres) as permissões não bloqueiam nada,
    // então o teste valida apenas o invariante de resiliência: nunca abortar.
    assert!(report.indexed_files >= 2);
    assert!(report.walk_errors <= 1);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Restaura para que o `TempDir` consiga remover a árvore.
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
