//! Leitura/escrita do manifesto JSON, busca por hash e relatório humano.

use crate::model::{DuplicateGroup, ScanReport, REPORT_FORMAT_VERSION};
use anyhow::{Context, Result};
use console::style;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

/// Menor prefixo de hash aceito por `sosia find` (8 caracteres hex = 32 bits).
pub const MIN_HASH_PREFIX: usize = 8;

/// Quantidade de grupos detalhados no relatório humano.
pub const HUMAN_REPORT_GROUP_LIMIT: usize = 5;

/// Serializa o manifesto e grava em `path`, criando diretórios-pai se preciso.
pub fn save_report(report: &ScanReport, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .with_context(|| format!("não foi possível criar {}", parent.display()))?;
        }
    }

    let file =
        File::create(path).with_context(|| format!("não foi possível criar {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, report)
        .context("falha ao serializar o relatório em JSON")?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Serializa o manifesto para uma `String`.
pub fn to_json(report: &ScanReport) -> Result<String> {
    serde_json::to_string_pretty(report).context("falha ao serializar o relatório em JSON")
}

/// Lê um manifesto salvo por `sosia scan --json --output`.
///
/// Relatórios gerados por versões anteriores (sem campos aditivos) são aceitos
/// via `#[serde(default)]`, com aviso quando `format_version` não bate.
pub fn load_report(path: &Path) -> Result<ScanReport> {
    let file = File::open(path)
        .with_context(|| format!("não foi possível abrir o relatório {}", path.display()))?;
    let report: ScanReport = serde_json::from_reader(BufReader::new(file)).with_context(|| {
        format!(
            "{} não é um manifesto válido (gere um com `sosia scan <DIR> --json --output <ARQ>`)",
            path.display()
        )
    })?;

    if report.format_version != REPORT_FORMAT_VERSION {
        eprintln!(
            "{}",
            style(format!(
                "aviso: manifesto no formato v{} (esta versão usa v{}); campos novos assumem o valor padrão",
                report.format_version, REPORT_FORMAT_VERSION
            ))
            .yellow()
        );
    }

    Ok(report)
}

/// Prefixo curto para exibição; não panica em manifestos legados/artesanais
/// com hash menor que 8 caracteres.
pub fn short_hash(hash: &str) -> &str {
    hash.get(..MIN_HASH_PREFIX).unwrap_or(hash)
}

/// Busca grupos de um manifesto por hash completo (case-insensitive) ou por
/// prefixo de 8+ caracteres hexadecimais.
pub fn find_groups<'a>(report: &'a ScanReport, hash: &str) -> Vec<&'a DuplicateGroup> {
    let needle = hash.trim().to_ascii_lowercase();
    report
        .duplicate_groups
        .iter()
        .filter(|group| {
            let candidate = group.full_hash.to_ascii_lowercase();
            candidate == needle
                || (needle.len() >= MIN_HASH_PREFIX && candidate.starts_with(&needle))
        })
        .collect()
}

/// Formata bytes em unidade legível (binária).
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Imprime o resumo humano do manifesto.
pub fn print_human_report(report: &ScanReport) {
    println!(
        "{}",
        style("=== Relatório de Deduplicação ===").bold().cyan()
    );
    println!("Diretório:               {}", report.target_path.display());
    println!("Arquivos encontrados:    {}", report.scanned_files);
    println!("Arquivos indexados:      {}", report.indexed_files);
    println!("Tamanhos únicos:         {}", report.unique_sizes);
    println!(
        "Bytes relidos p/ hash:   {} ({} B)",
        format_bytes(report.bytes_hashed),
        report.bytes_hashed
    );
    println!("Grupos duplicados:       {}", report.duplicate_groups.len());
    println!(
        "Espaço recuperável:      {}",
        style(format_bytes(report.total_recoverable_bytes))
            .bold()
            .green()
    );
    println!("Tempo total:             {:.3}s", report.elapsed_seconds);

    // Transparência de recall: o usuário precisa saber o que ficou de fora.
    println!("\n{}", style("Filtros aplicados:").bold());
    println!("  vazios ignorados:       {}", report.skipped_empty);
    println!("  hard links ignorados:   {}", report.skipped_hard_links);
    println!("  entradas ocultas:       {}", report.skipped_hidden);
    println!("  alterados na varredura: {}", report.skipped_modified);
    println!("  ilegíveis:              {}", report.unreadable_files);
    println!("  erros de varredura:     {}", report.walk_errors);
    if report.skipped_empty > 0 {
        println!(
            "{}",
            style("  (use --include-empty para considerar arquivos vazios)").dim()
        );
    }
    if report.skipped_hidden > 0 {
        println!(
            "{}",
            style("  (use --include-hidden para varrer entradas ocultas)").dim()
        );
    }
    if report.skipped_hard_links > 0 {
        println!(
            "{}",
            style("  (hard links dividem os mesmos blocos: apagar um não libera espaço)").dim()
        );
    }

    if !report.duplicate_groups.is_empty() {
        println!(
            "\n{}",
            style(format!(
                "Maiores grupos de duplicatas (de {}):",
                report.duplicate_groups.len()
            ))
            .bold()
        );
        for (index, group) in report
            .duplicate_groups
            .iter()
            .take(HUMAN_REPORT_GROUP_LIMIT)
            .enumerate()
        {
            println!(
                " [{}] hash {} | {} cada | {} cópias | recuperável: {}",
                index + 1,
                short_hash(&group.full_hash),
                format_bytes(group.file_size),
                group.files.len(),
                format_bytes(group.recoverable_space())
            );
            for file in &group.files {
                println!("     - {}", file.display());
            }
        }
        if report.duplicate_groups.len() > HUMAN_REPORT_GROUP_LIMIT {
            println!(
                "{}",
                style(format!(
                    " ... e mais {} grupo(s) — use --json para o manifesto completo",
                    report.duplicate_groups.len() - HUMAN_REPORT_GROUP_LIMIT
                ))
                .dim()
            );
        }
    }
}
