//! Contrato do CLI (flags aceitas/rejeitadas) e formatadores.

use clap::Parser as _;
use dedup::cli::{Cli, Commands};
use dedup::report::format_bytes;

#[test]
fn scan_accepts_expected_flags() {
    let cli = Cli::try_parse_from([
        "dedup",
        "scan",
        "/tmp/alvo",
        "--json",
        "--include-hidden",
        "--include-empty",
        "--output",
        "rel.json",
        "--threads",
        "4",
    ])
    .expect("flags válidas de scan");

    match cli.command {
        Commands::Scan(args) => {
            assert!(args.json);
            assert!(args.include_hidden);
            assert!(args.include_empty);
            assert_eq!(args.output.unwrap().to_string_lossy(), "rel.json");
            assert_eq!(args.path.to_string_lossy(), "/tmp/alvo");
        }
        _ => panic!("subcomando esperado: scan"),
    }
    assert_eq!(cli.threads, 4);
}

#[test]
fn scan_defaults_are_restrictive() {
    let cli = Cli::try_parse_from(["dedup", "scan", "/tmp"]).unwrap();

    match cli.command {
        Commands::Scan(args) => {
            assert!(!args.json);
            assert!(!args.include_empty, "vazios fora por padrão");
            assert!(!args.include_hidden, "ocultos fora por padrão");
            assert!(args.output.is_none());
        }
        _ => panic!("subcomando esperado: scan"),
    }
    assert!(!cli.no_progress);
}

#[test]
fn find_and_verify_take_a_saved_report() {
    let cli = Cli::try_parse_from(["dedup", "find", "rel.json", "abc123de"]).unwrap();
    match cli.command {
        Commands::Find(args) => {
            // `find` consulta um manifesto já gerado: não há caminho para varrer
            // de novo, logo não existe inconsistência de filtros entre comandos.
            assert_eq!(args.report.to_string_lossy(), "rel.json");
            assert_eq!(args.hash, "abc123de");
        }
        _ => panic!("subcomando esperado: find"),
    }

    Cli::try_parse_from(["dedup", "verify", "rel.json"]).expect("verify válido");
}

#[test]
fn interactive_defaults_to_simulation() {
    let cli = Cli::try_parse_from(["dedup", "interactive", "/tmp"]).unwrap();
    match cli.command {
        Commands::Interactive(args) => assert!(!args.apply, "simula por padrão"),
        _ => panic!("subcomando esperado: interactive"),
    }

    let applied = Cli::try_parse_from(["dedup", "interactive", "/tmp", "--apply"]).unwrap();
    match applied.command {
        Commands::Interactive(args) => assert!(args.apply),
        _ => panic!("subcomando esperado: interactive"),
    }
}

#[test]
fn the_broken_dry_run_flag_no_longer_exists() {
    // O bug: `#[arg(long, default_value_t = true)]` em um `bool` fazia o clap
    // exigir um valor explícito (`--dry-run true`), então `--dry-run` sozinho
    // falhava. A flag foi removida em favor de `--apply`.
    assert!(Cli::try_parse_from(["dedup", "interactive", "/tmp", "--dry-run"]).is_err());
    assert!(Cli::try_parse_from(["dedup", "scan", "/tmp", "--dry-run"]).is_err());
}

#[test]
fn global_flags_work_after_the_subcommand() {
    let cli =
        Cli::try_parse_from(["dedup", "scan", "/tmp", "--no-progress", "--threads", "2"]).unwrap();

    assert!(cli.no_progress);
    assert_eq!(cli.threads, 2);
}

#[test]
fn format_bytes_uses_binary_units() {
    assert_eq!(format_bytes(0), "0 B");
    assert_eq!(format_bytes(1023), "1023 B");
    assert_eq!(format_bytes(1024), "1.00 KB");
    assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
    assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
    assert_eq!(format_bytes(1024 * 1024 * 1024 * 1024), "1.00 TB");
    assert_eq!(format_bytes(1536), "1.50 KB");
}
