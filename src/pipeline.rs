//! Pipeline multi-estágio em paralelo.
//!
//! ```text
//! Varredura ─► [1] agrupa por tamanho ─► [2] hash parcial 4 KiB ─► [3] BLAKE3 completo ─► [4] grupos
//!              (descarta tamanhos       (descarta falsos          (só nos sobreviventes,
//!               únicos)                  positivos baratos)        em paralelo com Rayon)
//! ```
//!
//! Cada estágio só recebe os candidatos que sobreviveram ao anterior, então o
//! disco é tocado o mínimo possível. `ScanReport::bytes_hashed` registra o total
//! de bytes lidos nas etapas de hash — é a métrica que comprova o ganho.

use crate::hasher::{compute_full_hash, compute_partial_hash};
use crate::model::{DuplicateGroup, FileEntry, ScanReport, REPORT_FORMAT_VERSION};
use crate::progress::Reporter;
use crate::walker::{scan_directory_with, WalkOptions};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Opções do motor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineOptions {
    pub include_empty: bool,
    pub include_hidden: bool,
}

/// Motor de deduplicação. É imutável e seguro para uso concorrente.
pub struct DeduplicationEngine {
    options: EngineOptions,
}

impl DeduplicationEngine {
    /// Construtor de conveniência usado nos testes: vazios configuráveis,
    /// entradas ocultas excluídas.
    pub fn new(include_empty: bool) -> Self {
        Self::with_options(EngineOptions {
            include_empty,
            include_hidden: false,
        })
    }

    pub fn with_options(options: EngineOptions) -> Self {
        Self { options }
    }

    pub fn options(&self) -> EngineOptions {
        self.options
    }

    /// Executa o pipeline sem relatar progresso.
    pub fn run<P: AsRef<Path>>(&self, path: P) -> ScanReport {
        self.run_with_progress(path, &Reporter::hidden())
    }

    /// Executa o pipeline relatando o progresso de cada estágio.
    pub fn run_with_progress<P: AsRef<Path>>(&self, path: P, reporter: &Reporter) -> ScanReport {
        let started = Instant::now();
        let target = path.as_ref().to_path_buf();

        // Estágio 0 — varredura sequencial (limitada por metadados, ver walker.rs).
        reporter.start_walk();
        let walk = scan_directory_with(
            &target,
            WalkOptions {
                include_empty: self.options.include_empty,
                include_hidden: self.options.include_hidden,
            },
            || reporter.inc_walk(),
        );
        reporter.finish_walk(walk.stats.indexed_files);

        // Estágio 1 — agrupamento por tamanho. Só tamanhos com 2+ arquivos podem
        // conter duplicatas: o resto é descartado sem tocar no conteúdo.
        let mut size_buckets: HashMap<u64, Vec<FileEntry>> = HashMap::new();
        for file in walk.entries {
            size_buckets.entry(file.size).or_default().push(file);
        }
        let unique_sizes = size_buckets.len();

        let size_candidates: Vec<FileEntry> = size_buckets
            .into_par_iter()
            .filter(|(_, group)| group.len() > 1)
            .flat_map(|(_, group)| group)
            .collect();

        // Contadores compartilhados entre as threads do Rayon.
        let unreadable = AtomicU64::new(0);
        let bytes_read = AtomicU64::new(0);

        // Estágio 2 — hash parcial de 4 KiB: elimina a maioria dos candidatos
        // lendo ~0,003% de um arquivo de 128 MiB.
        reporter.start_partial(size_candidates.len());
        let partial_hashed: Vec<FileEntry> = size_candidates
            .into_par_iter()
            .filter_map(|mut entry| {
                let outcome = compute_partial_hash(&entry.path);
                reporter.inc_partial();
                match outcome {
                    Ok(outcome) => {
                        bytes_read.fetch_add(outcome.bytes_read, Ordering::Relaxed);
                        entry.partial_hash = Some(outcome.hex);
                        Some(entry)
                    }
                    Err(_) => {
                        unreadable.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                }
            })
            .collect();
        reporter.finish_partial();

        let mut partial_buckets: HashMap<(u64, String), Vec<FileEntry>> = HashMap::new();
        for file in partial_hashed {
            if let Some(partial) = file.partial_hash.clone() {
                partial_buckets
                    .entry((file.size, partial))
                    .or_default()
                    .push(file);
            }
        }

        let partial_candidates: Vec<FileEntry> = partial_buckets
            .into_par_iter()
            .filter(|(_, group)| group.len() > 1)
            .flat_map(|(_, group)| group)
            .collect();

        // Estágio 3 — BLAKE3 completo, apenas nos candidatos que sobreviveram.
        // O trabalho é CPU-bound e o Rayon distribui por work-stealing.
        let modified = AtomicU64::new(0);
        reporter.start_full(partial_candidates.len());
        let full_hashed: Vec<FileEntry> = partial_candidates
            .into_par_iter()
            .filter_map(|mut entry| {
                let outcome = compute_full_hash(&entry.path);
                reporter.inc_full();
                match outcome {
                    Ok(outcome) => {
                        bytes_read.fetch_add(outcome.bytes_read, Ordering::Relaxed);

                        // O arquivo pode ter sido alterado entre a varredura e o
                        // hash. O digest reflete o conteúdo lido, mas `size` viria
                        // de um metadata obsoleto: descartamos o candidato para
                        // não publicar um grupo com tamanho mentiroso.
                        let unchanged = std::fs::metadata(&entry.path)
                            .map(|metadata| metadata.len() == entry.size)
                            .unwrap_or(true);
                        if !unchanged {
                            modified.fetch_add(1, Ordering::Relaxed);
                            return None;
                        }

                        entry.full_hash = Some(outcome.hex);
                        Some(entry)
                    }
                    Err(_) => {
                        unreadable.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                }
            })
            .collect();
        reporter.finish_full();

        // Estágio 4 — agrupamento final por (tamanho, hash completo).
        let mut final_buckets: HashMap<(u64, String), Vec<FileEntry>> = HashMap::new();
        for file in full_hashed {
            if let Some(full) = file.full_hash.clone() {
                final_buckets
                    .entry((file.size, full))
                    .or_default()
                    .push(file);
            }
        }

        let mut total_recoverable_bytes = 0u64;
        let mut duplicate_groups = Vec::new();
        for ((size, hash), group) in final_buckets {
            if group.len() > 1 {
                let candidate = DuplicateGroup {
                    full_hash: hash,
                    file_size: size,
                    files: group.into_iter().map(|entry| entry.path).collect(),
                };
                total_recoverable_bytes += candidate.recoverable_space();
                duplicate_groups.push(candidate);
            }
        }

        let mut report = ScanReport::new(target);
        report.scanned_files = walk.stats.scanned_files;
        report.indexed_files = walk.stats.indexed_files;
        report.skipped_empty = walk.stats.skipped_empty;
        report.skipped_hard_links = walk.stats.skipped_hard_links;
        report.skipped_hidden = walk.stats.skipped_hidden;
        report.skipped_modified = modified.load(Ordering::Relaxed) as usize;
        report.unreadable_files = unreadable.load(Ordering::Relaxed) as usize;
        report.walk_errors = walk.stats.walk_errors;
        report.unique_sizes = unique_sizes;
        report.bytes_hashed = bytes_read.load(Ordering::Relaxed);
        report.duplicate_groups = duplicate_groups;
        report.total_recoverable_bytes = total_recoverable_bytes;
        report.format_version = REPORT_FORMAT_VERSION;
        report.sort_groups();
        report.elapsed_seconds = started.elapsed().as_secs_f64();
        report
    }
}
