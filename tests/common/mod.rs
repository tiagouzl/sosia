//! Helpers compartilhados pelos testes de integração.
//!
//! Idiomático em Rust: cada arquivo em `tests/` é um crate próprio e usa
//! `mod common;` para não repetir utilitários. `dead_code` é permitido porque
//! cada crate de teste usa um subconjunto diferente destes helpers.
#![allow(dead_code)]

use sosia::model::ScanReport;
use sosia::pipeline::{DeduplicationEngine, EngineOptions};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Cria um arquivo com o conteúdo indicado, criando diretórios-pai se preciso.
pub fn write_file(path: &Path, contents: &[u8]) -> PathBuf {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = File::create(path).unwrap();
    file.write_all(contents).unwrap();
    file.flush().unwrap();
    path.to_path_buf()
}

/// Cria `count` arquivos com o mesmo conteúdo e devolve os caminhos.
pub fn write_copies(dir: &Path, prefix: &str, count: usize, contents: &[u8]) -> Vec<PathBuf> {
    (0..count)
        .map(|index| write_file(&dir.join(format!("{}-{}.bin", prefix, index)), contents))
        .collect()
}

/// Executa o pipeline com o construtor de conveniência.
pub fn scan(dir: &Path, include_empty: bool) -> ScanReport {
    DeduplicationEngine::new(include_empty).run(dir)
}

/// Executa o pipeline com opções completas.
pub fn scan_with(dir: &Path, options: EngineOptions) -> ScanReport {
    DeduplicationEngine::with_options(options).run(dir)
}
