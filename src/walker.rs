//! Varredura resiliente do sistema de arquivos.
//!
//! Decisões deliberadas (todas expostas ao usuário ou reversíveis por flag):
//!
//! * **Symlinks não são seguidos** (`follow_links(false)`): evita recursão
//!   infinita e evita contar o mesmo conteúdo duas vezes por caminhos
//!   simbólicos. Consequência: um symlink para um arquivo não entra no índice.
//! * **Entradas ocultas** (prefixo `.`) são podadas por padrão, o que reduz o
//!   recall de forma silenciosa. Por isso `--include-hidden` existe e o
//!   relatório humano mostra quantas entradas foram ignoradas.
//! * **Hard links** são indexados uma única vez por `(device, inode)`.
//! * **Erros** (permissão negada, diretório ilegível) são contabilizados e a
//!   varredura continua.
//!
//! A varredura é **sequencial**: ela é limitada por metadados (I/O + syscalls) e
//! paralelizar o `walkdir` costuma trocar CPU por *thrashing* de disco. O ganho
//! paralelo do projeto está nas etapas de hash, que são CPU-bound. Este é um
//! trade-off consciente, documentado no README.

use crate::model::{FileEntry, FileId};
use std::collections::HashSet;
use std::path::Path;
use walkdir::{DirEntry, WalkDir};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Opções de varredura.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkOptions {
    /// Inclui arquivos de tamanho zero (todos os vazios são idênticos).
    pub include_empty: bool,
    /// Inclui arquivos e diretórios ocultos (prefixo `.`).
    pub include_hidden: bool,
}

/// Contadores observados durante a varredura.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkStats {
    /// Arquivos regulares encontrados, antes de qualquer filtro.
    pub scanned_files: usize,
    /// Arquivos efetivamente indexados.
    pub indexed_files: usize,
    pub skipped_empty: usize,
    pub skipped_hard_links: usize,
    /// Entradas ocultas podadas (a poda de um diretório oculto conta uma vez).
    pub skipped_hidden: usize,
    /// Erros devolvidos pelo `walkdir`.
    pub walk_errors: usize,
}

/// Resultado da varredura: entradas indexadas + estatísticas.
#[derive(Debug, Default)]
pub struct WalkOutcome {
    pub entries: Vec<FileEntry>,
    pub stats: WalkStats,
}

/// Varre `root` sem relatar progresso.
pub fn scan_directory<P: AsRef<Path>>(root: P, options: WalkOptions) -> WalkOutcome {
    scan_directory_with(root, options, || {})
}

/// Valida que `root` existe e é um diretório.
///
/// Sem isto, uma raiz inexistente aparece como um único erro de iteração do
/// `walkdir` (`walk_errors == 1`) e o relatório sai *vazio com sucesso* — um
/// erro de digitação do usuário mascarado como "nenhuma duplicata". O CLI
/// chama esta função antes de varrer para falhar com exit 2 e mensagem clara.
pub fn ensure_scannable<P: AsRef<Path>>(root: P) -> anyhow::Result<()> {
    let root = root.as_ref();
    let metadata = std::fs::metadata(root)
        .map_err(|err| anyhow::anyhow!("não foi possível acessar {}: {}", root.display(), err))?;
    if !metadata.is_dir() {
        anyhow::bail!("{} não é um diretório", root.display());
    }
    Ok(())
}

/// Varre `root` chamando `on_file` para cada arquivo regular encontrado
/// (usado para alimentar a barra de progresso da varredura).
pub fn scan_directory_with<P, F>(root: P, options: WalkOptions, mut on_file: F) -> WalkOutcome
where
    P: AsRef<Path>,
    F: FnMut(),
{
    let mut outcome = WalkOutcome::default();
    let mut seen_hard_links: HashSet<FileId> = HashSet::new();
    let mut skipped_hidden = 0usize;

    let walker = WalkDir::new(root).follow_links(false).into_iter();

    // `filter_entry` devolvendo `false` para um diretório também impede descer
    // nele — é o que queremos para diretórios ocultos. O `depth() == 0` é
    // essencial: o PREDICADO TAMBÉM É APLICADO À RAIZ, então sem essa condição
    // `dedup scan /tmp/.backup` devolveria zero arquivos (e `tempdir()`, que
    // cria diretórios `.tmpXXXX`, quebraria silenciosamente).
    let filtered = walker.filter_entry(|entry| {
        if options.include_hidden || entry.depth() == 0 {
            return true;
        }
        if is_hidden(entry) {
            skipped_hidden += 1;
            return false;
        }
        true
    });

    for entry in filtered {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                outcome.stats.walk_errors += 1;
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        outcome.stats.scanned_files += 1;
        on_file();

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                outcome.stats.walk_errors += 1;
                continue;
            }
        };

        let size = metadata.len();
        if size == 0 && !options.include_empty {
            outcome.stats.skipped_empty += 1;
            continue;
        }

        let file_id = get_file_id(&metadata);

        // Mesmo `(device, inode)` já visto => hard link: os blocos no disco são
        // compartilhados, apagar um caminho não libera espaço. Indexamos só o
        // primeiro (o caminho "canônico" passa a ser o primeiro visitado).
        if let Some(id) = file_id.clone() {
            if !seen_hard_links.insert(id) {
                outcome.stats.skipped_hard_links += 1;
                continue;
            }
        }

        outcome.entries.push(FileEntry::new(
            entry.path().to_path_buf(),
            size,
            metadata.modified().ok(),
            file_id,
        ));
    }

    outcome.stats.skipped_hidden = skipped_hidden;
    outcome.stats.indexed_files = outcome.entries.len();
    outcome
}

fn is_hidden(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(unix)]
fn get_file_id(metadata: &std::fs::Metadata) -> Option<FileId> {
    Some(FileId {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn get_file_id(_metadata: &std::fs::Metadata) -> Option<FileId> {
    None
}
