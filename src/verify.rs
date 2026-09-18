//! Verificação de integridade *pós-análise* de um manifesto já salvo.
//!
//! Relê cada arquivo citado no relatório e recomputa o BLAKE3 completo para
//! detectar *bit rot*, edição silenciosa ou remoção externa desde o scan.
//! A verificação é paralelizada com Rayon (é o mesmo custo de I/O do hash
//! completo, agora sobre dados que podem ter dias de idade).

use crate::hasher::compute_full_hash;
use crate::progress::Reporter;
use crate::report::load_report;
use anyhow::Result;
use console::style;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

/// Resultado agregado da auditoria.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerifySummary {
    pub groups: usize,
    pub files: usize,
    pub verified: usize,
    pub missing: usize,
    pub changed: usize,
    pub unreadable: usize,
}

impl VerifySummary {
    /// `true` quando algo exige atenção do usuário.
    pub fn has_problems(&self) -> bool {
        self.missing > 0 || self.changed > 0 || self.unreadable > 0
    }
}

/// Veredito por arquivo (mantém a ordem do manifesto para impressão estável).
enum Verdict {
    Verified,
    Missing(PathBuf),
    Changed {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    Unreadable {
        path: PathBuf,
        message: String,
    },
}

/// Audita o manifesto sem relatar progresso.
pub fn verify_report(report_path: &Path) -> Result<VerifySummary> {
    verify_report_with_progress(report_path, &Reporter::hidden())
}

/// Audita o manifesto relatando progresso.
pub fn verify_report_with_progress(
    report_path: &Path,
    reporter: &Reporter,
) -> Result<VerifySummary> {
    let report = load_report(report_path)?;

    println!(
        "{}",
        style(format!(
            "Auditando manifesto de: {} (formato v{})",
            report.target_path.display(),
            report.format_version
        ))
        .bold()
    );
    println!(
        "Grupos: {} | Arquivos a verificar: {}",
        report.duplicate_groups.len(),
        report
            .duplicate_groups
            .iter()
            .map(|g| g.files.len())
            .sum::<usize>()
    );

    let pairs: Vec<(&str, &PathBuf)> = report
        .duplicate_groups
        .iter()
        .flat_map(|group| {
            group
                .files
                .iter()
                .map(move |path| (group.full_hash.as_str(), path))
        })
        .collect();

    reporter.start_verify(pairs.len());
    let verdicts: Vec<Verdict> = pairs
        .par_iter()
        .map(|(expected, path)| {
            reporter.inc_verify();
            // `Path::exists` retorna false em qualquer erro (até falta de
            // permissão), então um arquivo ilegível seria relatado como
            // "FALTANDO". `symlink_metadata` distingue: só NotFound é ausência.
            if matches!(
                std::fs::symlink_metadata(path).map_err(|e| e.kind()),
                Err(std::io::ErrorKind::NotFound)
            ) {
                return Verdict::Missing((*path).clone());
            }
            match compute_full_hash(path) {
                Ok(outcome) if outcome.hex.eq_ignore_ascii_case(expected) => Verdict::Verified,
                Ok(outcome) => Verdict::Changed {
                    path: (*path).clone(),
                    expected: (*expected).to_string(),
                    actual: outcome.hex,
                },
                Err(err) => Verdict::Unreadable {
                    path: (*path).clone(),
                    message: err.to_string(),
                },
            }
        })
        .collect();
    reporter.finish_verify();

    let mut summary = VerifySummary {
        groups: report.duplicate_groups.len(),
        files: pairs.len(),
        ..Default::default()
    };

    for verdict in verdicts {
        match verdict {
            Verdict::Verified => summary.verified += 1,
            Verdict::Missing(path) => {
                println!("{} {}", style("[FALTANDO]").red().bold(), path.display());
                summary.missing += 1;
            }
            Verdict::Changed {
                path,
                expected,
                actual,
            } => {
                println!(
                    "{} {}\n   esperado: {}\n   atual:    {}",
                    style("[ALTERADO/CORROMPIDO]").red().bold(),
                    path.display(),
                    expected,
                    actual
                );
                summary.changed += 1;
            }
            Verdict::Unreadable { path, message } => {
                eprintln!(
                    "{} {}: {}",
                    style("[ERRO]").red().bold(),
                    path.display(),
                    message
                );
                summary.unreadable += 1;
            }
        }
    }

    println!("\nResumo da verificação:");
    println!("  Íntegros:      {}", style(summary.verified).green());
    println!("  Faltando:      {}", style(summary.missing).yellow());
    println!("  Alterados:     {}", style(summary.changed).red());
    println!("  Ilegíveis:     {}", style(summary.unreadable).red());

    Ok(summary)
}
