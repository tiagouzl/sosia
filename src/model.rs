//! Estruturas de dados centrais do motor de deduplicação.
//!
//! `FileEntry` descreve um arquivo indexado durante a varredura; `DuplicateGroup`
//! e `ScanReport` são as estruturas serializáveis que compõem o manifesto JSON
//! consumido por `dedup find` e `dedup verify`.

use std::path::PathBuf;
use std::time::SystemTime;

/// Versão do formato do manifesto JSON. Relatórios gerados por versões
/// anteriores têm o campo ausente e são lidos como `0` (`#[serde(default)]`),
/// o que apenas dispara um aviso em `report::load_report`.
pub const REPORT_FORMAT_VERSION: u32 = 1;

/// Identidade física de um arquivo em sistemas POSIX (`st_dev` + `st_ino`).
///
/// Dois caminhos com o mesmo `FileId` são *hard links* do mesmo conteúdo: eles
/// compartilham os mesmos blocos no disco, portanto apagar um deles **não**
/// devolve espaço e ele não deve ser contado como desperdício.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileId {
    pub device: u64,
    pub inode: u64,
}

/// Arquivo indexado na varredura. Os campos de hash são preenchidos conforme o
/// pipeline avança (`None` significa "ainda não calculado").
///
/// Não deriva `Serialize`/`Deserialize`: apenas `DuplicateGroup`/`ScanReport`
/// vão para o JSON, então manter a serialização aqui seria superfície morta.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub file_id: Option<FileId>,
    pub partial_hash: Option<String>,
    pub full_hash: Option<String>,
}

impl FileEntry {
    pub fn new(
        path: PathBuf,
        size: u64,
        modified: Option<SystemTime>,
        file_id: Option<FileId>,
    ) -> Self {
        Self {
            path,
            size,
            modified,
            file_id,
            partial_hash: None,
            full_hash: None,
        }
    }
}

/// Conjunto de arquivos com conteúdo idêntico (mesmo BLAKE3 completo).
///
/// Deriva `Ord` para que o benchmark possa comparar listas de grupos
/// independentemente da ordem de produção (o relatório humano já agrupa por
/// espaço recuperável antes de imprimir).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct DuplicateGroup {
    pub full_hash: String,
    pub file_size: u64,
    /// Cópias do mesmo conteúdo, ordenadas lexicograficamente para que a saída
    /// seja determinística entre execuções.
    pub files: Vec<PathBuf>,
}

impl DuplicateGroup {
    /// Espaço recuperável: tamanho do arquivo multiplicado pelo número de
    /// cópias que podem ser eliminadas (mantendo pelo menos um exemplar).
    pub fn recoverable_space(&self) -> u64 {
        if self.files.len() > 1 {
            (self.files.len() as u64 - 1) * self.file_size
        } else {
            0
        }
    }
}

/// Manifesto completo de uma varredura.
///
/// Os campos estatísticos são aditivos e marcados com `#[serde(default)]` para
/// que relatórios antigos (sem esses campos) continuem sendo lidos em vez de
/// falharem na desserialização.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScanReport {
    pub target_path: PathBuf,
    /// Arquivos regulares encontrados pela varredura, antes de qualquer filtro.
    #[serde(default)]
    pub scanned_files: usize,
    /// Arquivos que entraram no pipeline (após filtros de vazio/hard link).
    #[serde(default)]
    pub indexed_files: usize,
    /// Arquivos de tamanho zero ignorados (`--include-empty` reverte).
    #[serde(default)]
    pub skipped_empty: usize,
    /// Entradas que eram hard links de um arquivo já indexado.
    #[serde(default)]
    pub skipped_hard_links: usize,
    /// Entradas ocultas podadas (`--include-hidden` reverte). A varredura não
    /// desce em diretórios ocultos, então este é um contador de *entradas*.
    #[serde(default)]
    pub skipped_hidden: usize,
    /// Arquivos alterados entre a varredura e o hash completo.
    #[serde(default)]
    pub skipped_modified: usize,
    /// Arquivos que não puderam ser lidos/hasheados (permissão, sumiço, I/O).
    #[serde(default)]
    pub unreadable_files: usize,
    /// Erros devolvidos pelo `walkdir` (diretórios ilegíveis, loops, etc.).
    #[serde(default)]
    pub walk_errors: usize,
    /// Quantidade de tamanhos distintos observados.
    #[serde(default)]
    pub unique_sizes: usize,
    /// Total de bytes efetivamente lidos do disco pelas etapas de hash.
    /// É a métrica auditável do ganho do pipeline (independe de cache/wall clock).
    #[serde(default)]
    pub bytes_hashed: u64,
    #[serde(default)]
    pub duplicate_groups: Vec<DuplicateGroup>,
    #[serde(default)]
    pub total_recoverable_bytes: u64,
    #[serde(default)]
    pub elapsed_seconds: f64,
    #[serde(default)]
    pub format_version: u32,
}

impl ScanReport {
    pub fn new(target_path: PathBuf) -> Self {
        Self {
            target_path,
            scanned_files: 0,
            indexed_files: 0,
            skipped_empty: 0,
            skipped_hard_links: 0,
            skipped_hidden: 0,
            skipped_modified: 0,
            unreadable_files: 0,
            walk_errors: 0,
            unique_sizes: 0,
            bytes_hashed: 0,
            duplicate_groups: Vec::new(),
            total_recoverable_bytes: 0,
            elapsed_seconds: 0.0,
            format_version: REPORT_FORMAT_VERSION,
        }
    }

    /// Ordena grupos (maior espaço recuperável primeiro, desempate pelo hash) e
    /// os arquivos dentro de cada grupo. Garante saída determinística.
    pub fn sort_groups(&mut self) {
        self.duplicate_groups.sort_by(|a, b| {
            b.recoverable_space()
                .cmp(&a.recoverable_space())
                .then_with(|| a.full_hash.cmp(&b.full_hash))
        });
        for group in &mut self.duplicate_groups {
            group.files.sort();
        }
    }
}
