//! Cálculo de hash com streaming e contabilização de I/O.
//!
//! Toda função devolve um [`HashOutcome`] com o digest em hex e a quantidade
//! de bytes realmente lida do disco. Esse contador alimenta
//! `ScanReport::bytes_hashed`, a métrica auditável do projeto: ela não depende
//! de page cache, de wall clock ou de hardware.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

/// Quantidade de bytes lidos na etapa de hash parcial (filtro barato).
pub const PARTIAL_HASH_SIZE: usize = 4 * 1024;

/// Capacidade do buffer usado no streaming do hash completo.
pub const FULL_HASH_BUFFER_SIZE: usize = 64 * 1024;

/// Resultado de um cálculo de hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashOutcome {
    /// Digest BLAKE3 em hexadecimal (64 caracteres).
    pub hex: String,
    /// Bytes efetivamente lidos do arquivo durante o cálculo.
    pub bytes_read: u64,
}

impl HashOutcome {
    pub fn new(hex: String, bytes_read: u64) -> Self {
        Self { hex, bytes_read }
    }

    pub fn into_hex(self) -> String {
        self.hex
    }
}

/// Calcula o BLAKE3 dos **primeiros** [`PARTIAL_HASH_SIZE`] bytes do arquivo.
///
/// `Read::read` não garante preencher o buffer mesmo havendo dados
/// disponíveis — é uma garantia da trait, não uma promessa da implementação.
/// Em FUSE, NFS ou após um `EINTR`, uma única chamada pode devolver menos bytes
/// do que o pedido, o que faria o hash parcial cobrir uma quantidade variável de
/// bytes e produzir falsos "iguais". O laço abaixo preenche o buffer por
/// completo (ou até o EOF), tratando `Interrupted` explicitamente.
pub fn compute_partial_hash<P: AsRef<Path>>(path: P) -> Result<HashOutcome> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .with_context(|| format!("falha ao abrir para hash parcial: {}", path.display()))?;

    let mut buffer = [0u8; PARTIAL_HASH_SIZE];
    let mut filled = 0usize;

    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break, // EOF: arquivo menor que o prefixo pedido
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("falha ao ler para hash parcial: {}", path.display()))
            }
        }
    }

    Ok(HashOutcome::new(
        blake3::hash(&buffer[..filled]).to_hex().to_string(),
        filled as u64,
    ))
}

/// Calcula o BLAKE3 do arquivo inteiro em streaming.
///
/// O arquivo nunca é carregado inteiro na RAM: `io::copy` alimenta um
/// `BufReader` de 64 KiB direto no `blake3::Hasher` (que implementa
/// `io::Write`). `io::copy` também retenta em `ErrorKind::Interrupted`, e o
/// valor devolvido é o total de bytes lidos.
pub fn compute_full_hash<P: AsRef<Path>>(path: P) -> Result<HashOutcome> {
    let path = path.as_ref();
    let file = File::open(path)
        .with_context(|| format!("falha ao abrir para hash completo: {}", path.display()))?;

    let mut reader = BufReader::with_capacity(FULL_HASH_BUFFER_SIZE, file);
    let mut hasher = blake3::Hasher::new();
    let bytes_read = io::copy(&mut reader, &mut hasher)
        .with_context(|| format!("falha ao ler para hash completo: {}", path.display()))?;

    Ok(HashOutcome::new(
        hasher.finalize().to_hex().to_string(),
        bytes_read,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn partial_hash_reads_exactly_the_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.bin");
        let small = dir.path().join("small.bin");
        std::fs::File::create(&big)
            .unwrap()
            .write_all(&vec![0xAB_u8; PARTIAL_HASH_SIZE * 3])
            .unwrap();
        std::fs::File::create(&small)
            .unwrap()
            .write_all(b"curto")
            .unwrap();

        let big_outcome = compute_partial_hash(&big).unwrap();
        assert_eq!(big_outcome.bytes_read, PARTIAL_HASH_SIZE as u64);

        let small_outcome = compute_partial_hash(&small).unwrap();
        assert_eq!(small_outcome.bytes_read, 5);

        // Mesmo prefixo de 4 KiB + cauda diferente => hash parcial idêntico.
        let other = dir.path().join("other.bin");
        let mut payload = vec![0xAB_u8; PARTIAL_HASH_SIZE];
        payload.extend_from_slice(b"cauda completamente diferente");
        std::fs::File::create(&other)
            .unwrap()
            .write_all(&payload)
            .unwrap();
        assert_eq!(
            compute_partial_hash(&other).unwrap().hex,
            big_outcome.hex,
            "o hash parcial deve cobrir exatamente os primeiros 4 KiB"
        );
    }

    #[test]
    fn full_hash_matches_known_blake3_vector() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("abc.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"abc")
            .unwrap();

        let outcome = compute_full_hash(&path).unwrap();
        // Vetor de teste oficial do BLAKE3 para "abc".
        assert_eq!(
            outcome.hex,
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        assert_eq!(outcome.bytes_read, 3);
    }

    #[test]
    fn full_hash_streams_files_larger_than_the_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("grande.bin");
        let size = FULL_HASH_BUFFER_SIZE * 4 + 123;
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(&vec![0x5A_u8; size]).unwrap();
        drop(file);

        let outcome = compute_full_hash(&path).unwrap();
        assert_eq!(outcome.bytes_read, size as u64);
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(compute_partial_hash(dir.path().join("nao-existe")).is_err());
        assert!(compute_full_hash(dir.path().join("nao-existe")).is_err());
    }
}
