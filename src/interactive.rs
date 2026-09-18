//! Resolução interativa de duplicatas.
//!
//! A lógica de remoção foi extraída para [`apply_removal`], uma função pura
//! (sem terminal, sem `dialoguer`) que recebe o grupo, o índice a preservar e o
//! modo de simulação. Isso torna o comportamento destrutivo testável sem TTY —
//! o loop abaixo cuida apenas de apresentação e seleção.
//!
//! Segurança: por padrão nada é apagado (`dry_run = true`, derivado da ausência
//! de `--apply`); não existe flag redundante para "desligar o dry-run".

use crate::model::DuplicateGroup;
use anyhow::{Context, Result};
use console::style;
use dialoguer::{theme::ColorfulTheme, Select};
use std::fs;
use std::path::PathBuf;

/// Resultado de uma rodada de remoção.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemovalOutcome {
    /// Arquivos efetivamente removidos do disco.
    pub removed: usize,
    /// Arquivos que *seriam* removidos (modo simulação).
    pub pending: usize,
    /// Falhas por arquivo (caminho + mensagem do SO).
    pub failed: Vec<(PathBuf, String)>,
}

impl RemovalOutcome {
    pub fn total_processed(&self) -> usize {
        self.removed + self.pending + self.failed.len()
    }
}

/// Remove todos os arquivos de `files`, exceto `keep`.
///
/// Com `dry_run = true` nada é tocado no disco e os arquivos são contabilizados
/// em [`RemovalOutcome::pending`]. Falhas individuais não abortam o restante.
/// Retorna erro apenas para uma seleção inválida (`keep` fora dos limites).
pub fn apply_removal(files: &[PathBuf], keep: usize, dry_run: bool) -> Result<RemovalOutcome> {
    if keep >= files.len() {
        anyhow::bail!(
            "seleção inválida: índice {} não existe em um grupo de {} arquivo(s)",
            keep,
            files.len()
        );
    }

    let mut outcome = RemovalOutcome::default();
    for (index, path) in files.iter().enumerate() {
        if index == keep {
            continue;
        }
        if dry_run {
            outcome.pending += 1;
            continue;
        }
        match fs::remove_file(path) {
            Ok(()) => outcome.removed += 1,
            Err(err) => outcome.failed.push((path.clone(), err.to_string())),
        }
    }
    Ok(outcome)
}

/// Loop interativo: mostra cada grupo, pergunta qual arquivo preservar e aplica
/// a resolução (respeitando `dry_run`).
pub fn run_interactive_resolution(groups: &[DuplicateGroup], dry_run: bool) -> Result<()> {
    if groups.is_empty() {
        println!(
            "{}",
            style("Nenhum grupo de duplicatas para resolver!").green()
        );
        return Ok(());
    }

    println!(
        "{}",
        style(format!(
            "Modo interativo para {} grupo(s) de duplicatas.",
            groups.len()
        ))
        .bold()
        .cyan()
    );
    if dry_run {
        println!(
            "{}",
            style("[SIMULAÇÃO] Nada será apagado do disco. Use --apply para efetivar.")
                .yellow()
                .bold()
        );
    } else {
        println!(
            "{}",
            style("[EXCLUSÃO REAL] Os arquivos não preservados serão apagados.")
                .red()
                .bold()
        );
    }

    let mut total_removed = 0usize;
    let mut total_pending = 0usize;
    let mut total_failed = 0usize;
    let mut total_preserved = 0usize;
    let mut skipped_groups = 0usize;

    for (index, group) in groups.iter().enumerate() {
        println!("\n--------------------------------------------------");
        println!(
            "Grupo {}/{}: hash {} | {} cada | {} cópias | recuperável: {}",
            index + 1,
            groups.len(),
            style(crate::report::short_hash(&group.full_hash)).yellow(),
            crate::report::format_bytes(group.file_size),
            group.files.len(),
            crate::report::format_bytes(group.recoverable_space())
        );

        let mut choices: Vec<String> = group
            .files
            .iter()
            .map(|path| format!("Manter: {}", path.display()))
            .collect();
        choices.push("Pular este grupo (não remover nada)".to_string());

        let selection = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("Escolha qual arquivo PRESERVAR (os demais serão removidos)")
            .default(0)
            .items(&choices)
            .interact()
            .context("não foi possível ler a seleção (o modo interativo exige um terminal)")?;

        if selection == group.files.len() {
            println!("{}", style("Grupo pulado.").dim());
            skipped_groups += 1;
            continue;
        }

        let outcome = apply_removal(&group.files, selection, dry_run)?;

        for (path, message) in &outcome.failed {
            eprintln!(
                "{} falha ao remover {}: {}",
                style("ERRO:").red().bold(),
                path.display(),
                message
            );
        }

        for (file_index, path) in group.files.iter().enumerate() {
            if file_index == selection {
                continue;
            }
            let failed = outcome
                .failed
                .iter()
                .any(|(failed_path, _)| failed_path == path);
            if dry_run {
                println!(
                    "{} {}",
                    style("[SIMULAÇÃO] excluiria:").yellow(),
                    path.display()
                );
            } else if !failed {
                println!("{} {}", style("Removido:").red(), path.display());
            }
        }

        println!(
            "{} {}",
            style("Preservado:").green(),
            group.files[selection].display()
        );

        total_removed += outcome.removed;
        total_pending += outcome.pending;
        total_failed += outcome.failed.len();
        total_preserved += 1;
    }

    println!("\n{}", style("Resumo da sessão:").bold());
    if dry_run {
        println!(
            "  simulação: {} arquivo(s) seriam removidos, {} preservado(s)",
            style(total_pending).yellow(),
            total_preserved
        );
    } else {
        println!(
            "  removidos: {} | preservados: {}",
            style(total_removed).red(),
            total_preserved
        );
    }
    println!("  grupos pulados: {}", skipped_groups);
    if total_failed > 0 {
        println!("  falhas:         {}", style(total_failed).red().bold());
    }

    Ok(())
}
