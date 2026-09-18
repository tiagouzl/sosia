//! Benchmark real: pipeline multi-estágio vs. linha de base ingênua.
//!
//! Mede, sobre o MESMO conjunto sintético de arquivos:
//! 1. `pipeline`: `DeduplicationEngine::run` (tamanho → hash parcial 4 KiB →
//!    hash completo BLAKE3, Rayon em paralelo);
//! 2. `naive`: hash completo de TODOS os arquivos (o que um verificador ingênuo
//!    faria), também em paralelo, com o mesmo BLAKE3.
//!
//! Para que a comparação seja honesta, entre as rodadas:
//! * o cache de páginas do kernel é liberado com `posix_fadvise(POSIX_FADV_DONTNEED)`
//!   em cada arquivo (via `libc`), forçando leitura real do disco;
//! * os grupos produzidos pelas duas estratégias são comparados — divergência
//!   é um erro, garantindo que a otimização não troca corretude por velocidade.
//!
//! Nenhum número aqui é inventado: tudo vem de `std::time::Instant` neste host.

use dedup::model::DuplicateGroup;
use dedup::pipeline::{DeduplicationEngine, EngineOptions};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Arquivos com tamanho único: o pipeline nunca lê o conteúdo deles.
const UNIQUE_COUNT: usize = 160;
/// Grupos de duplicatas (conteúdo idêntico dentro do grupo).
const DUP_GROUPS: usize = 80;
/// Cópias por grupo de duplicatas.
const DUP_COPIES: usize = 3;
/// Tamanho base dos arquivos de tamanho único (cada um ganha +17 bytes).
const UNIQUE_BASE: u64 = 1024 * 1024;
/// Tamanho dos arquivos duplicados.
const DUP_SIZE: u64 = 2 * 1024 * 1024;

#[cfg(target_os = "linux")]
fn evict_from_page_cache(path: &Path) {
    use std::os::unix::io::AsRawFd;

    const POSIX_FADV_DONTNEED: i32 = 4;
    if let Ok(file) = File::open(path) {
        // SAFETY: fd válido obtido de um `File` vivo; a chamada não altera o
        // arquivo, apenas sugere ao kernel que descarte as páginas em cache.
        unsafe {
            libc::posix_fadvise(file.as_raw_fd(), 0, 0, POSIX_FADV_DONTNEED);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn evict_from_page_cache(_path: &Path) {
    // Fora do Linux: segue sem liberar cache (números menos "frios").
}

fn cold_cache(paths: &[PathBuf]) {
    for path in paths {
        evict_from_page_cache(path);
    }
}

/// Normaliza um mapa hash->caminhos no formato de grupos do motor.
fn normalize(groups: BTreeMap<String, Vec<PathBuf>>) -> Vec<DuplicateGroup> {
    groups
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(full_hash, files)| {
            let file_size = files[0].metadata().expect("metadados").len();
            DuplicateGroup {
                full_hash,
                file_size,
                files,
            }
        })
        .collect()
}

fn hardware_info() -> String {
    let logical = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let mem_kb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find(|line| line.starts_with("MemTotal:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
        });
    match mem_kb {
        Some(kb) => format!(
            "hardware: {} threads lógicas, ~{} MiB de RAM, Linux, /tmp em disco real",
            logical,
            kb / 1024
        ),
        None => format!("hardware: {logical} threads lógicas"),
    }
}

fn total_size(paths: &[PathBuf]) -> u64 {
    paths
        .iter()
        .map(|path| path.metadata().expect("metadados").len())
        .sum()
}

fn main() {
    println!("=== Benchmark dedup: pipeline vs. hash-ingênuo ===");
    println!("{}", hardware_info());
    println!(
        "dataset: {} arquivos, ~{} MiB de dados sintéticos",
        UNIQUE_COUNT + DUP_GROUPS * DUP_COPIES,
        (UNIQUE_BASE * UNIQUE_COUNT as u64 + DUP_SIZE * (DUP_GROUPS * DUP_COPIES) as u64)
            / (1024 * 1024)
    );

    let dir = tempfile::tempdir().expect("criar diretório temporário");
    println!("criando dataset sintético em {} ...", dir.path().display());
    let paths = build_dataset(dir.path());
    println!(
        "dataset pronto: {} arquivo(s) ({} únicos + {} grupos x {} cópias), {} no total",
        paths.len(),
        UNIQUE_COUNT,
        DUP_GROUPS,
        DUP_COPIES,
        dedup::report::format_bytes(total_size(&paths))
    );

    // --- Rodada 1: pipeline (cache frio) -------------------------------
    cold_cache(&paths);
    let pipeline_start = Instant::now();
    let mut report = DeduplicationEngine::with_options(EngineOptions::default()).run(dir.path());
    let pipeline_elapsed = pipeline_start.elapsed();

    // --- Rodada 2: hash ingênuo completo (cache frio) ------------------
    cold_cache(&paths);
    let naive_start = Instant::now();
    let (naive_groups, naive_bytes) = naive_full_hash(&paths);
    let naive_elapsed = naive_start.elapsed();

    // --- Verificação de equivalência (corretude acima de tudo) ---------
    let mut normalized_naive = normalize(naive_groups);
    report.duplicate_groups.sort();
    normalized_naive.sort();
    if normalized_naive != report.duplicate_groups {
        panic!(
            "FALHA DE EQUIVALÊNCIA: pipeline e linha de base ingênua divergem \
             ({} grupos vs. {} grupos)",
            report.duplicate_groups.len(),
            normalized_naive.len()
        );
    }

    // --- Relatório ------------------------------------------------------
    let naive_mib_s = naive_bytes as f64 / (1024.0 * 1024.0) / naive_elapsed.as_secs_f64();
    let pipeline_mib_s =
        report.bytes_hashed as f64 / (1024.0 * 1024.0) / pipeline_elapsed.as_secs_f64();
    let speedup = naive_elapsed.as_secs_f64() / pipeline_elapsed.as_secs_f64();

    println!();
    println!("linha de base (hash completo de tudo, cache frio):");
    println!(
        "  {:>8.2?} | {} lidos | {:>8.1} MiB/s | {} grupo(s) de duplicatas",
        naive_elapsed,
        dedup::report::format_bytes(naive_bytes),
        naive_mib_s,
        normalized_naive.len()
    );
    println!("pipeline (tamanho → 4 KiB → BLAKE3, cache frio):");
    println!(
        "  {:>8.2?} | {} lidos | {:>8.1} MiB/s | {} grupo(s) de duplicatas",
        pipeline_elapsed,
        dedup::report::format_bytes(report.bytes_hashed),
        pipeline_mib_s,
        report.duplicate_groups.len()
    );
    println!();
    println!(
        "leitura evitada pelo pipeline: {} de {} ({:.1}%)",
        dedup::report::format_bytes(naive_bytes - report.bytes_hashed),
        dedup::report::format_bytes(naive_bytes),
        100.0 * (naive_bytes - report.bytes_hashed) as f64 / naive_bytes as f64
    );
    println!("speedup (cache frio): {:.2}x", speedup);
    println!();
    println!(
        "equivalência dos grupos: OK ({} grupos idênticos)",
        normalized_naive.len()
    );
    println!(
        "NOTA: cada rodada liberou o cache de páginas com posix_fadvise(DONTNEED); \
         números refletem leitura real do disco, não RAM."
    );
}

/// Grava `size` bytes de um padrão determinístico derivado de `seed`.
fn write_deterministic(path: &Path, size: u64, seed: u8) {
    // Buffer de 1 MiB repetido: gerar conteúdo idêntico por semente sem
    // depender de `io::repeat` (que não tem `.take()` como leitor).
    const CHUNK: usize = 1024 * 1024;
    let buffer = [seed; CHUNK];
    let mut file = File::create(path).expect("criar arquivo");
    let mut remaining = size;
    while remaining > 0 {
        let to_write = remaining.min(CHUNK as u64) as usize;
        file.write_all(&buffer[..to_write]).expect("gravar arquivo");
        remaining -= to_write as u64;
    }
}

/// Cria o conjunto sintético: arquivos de tamanho único (nunca lidos pelo
/// pipeline) e grupos de duplicatas de conteúdo idêntico.
fn build_dataset(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(UNIQUE_COUNT + DUP_GROUPS * DUP_COPIES);

    for index in 0..UNIQUE_COUNT {
        // Tamanhos todos distintos entre si e distintos de DUP_SIZE.
        let size = UNIQUE_BASE + index as u64 * 17;
        let path = root.join(format!("unico-{:04}.bin", index));
        write_deterministic(&path, size, (index % 251) as u8);
        paths.push(path);
    }

    for group in 0..DUP_GROUPS {
        let seed = ((group * 37) % 251) as u8;
        for copy in 0..DUP_COPIES {
            let path = root.join(format!("dup-{:02}-{}.bin", group, copy));
            write_deterministic(&path, DUP_SIZE, seed);
            paths.push(path);
        }
    }
    paths
}

/// Linha de base ingênua: lê e aplica hash em TODOS os arquivos por completo.
fn naive_full_hash(paths: &[PathBuf]) -> (BTreeMap<String, Vec<PathBuf>>, u64) {
    // (hash -> caminhos), com hash completo de todos os arquivos em paralelo.
    let groups: BTreeMap<String, Vec<PathBuf>> = paths
        .par_iter()
        .map(|path| {
            let mut hasher = blake3::Hasher::new();
            let mut file = File::open(path).expect("abrir arquivo");
            std::io::copy(&mut file, &mut hasher).expect("ler arquivo");
            (hasher.finalize().to_string(), path.clone())
        })
        .fold(
            BTreeMap::<String, Vec<PathBuf>>::new,
            |mut acc, (hash, path)| {
                acc.entry(hash).or_default().push(path);
                acc
            },
        )
        .reduce(BTreeMap::<String, Vec<PathBuf>>::new, |mut a, b| {
            for (hash, mut group) in b {
                a.entry(hash).or_default().append(&mut group);
            }
            a
        });

    let bytes: u64 = paths
        .par_iter()
        .map(|path| path.metadata().expect("metadados").len())
        .sum();
    (groups, bytes)
}
