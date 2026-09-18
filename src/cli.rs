//! Definição da interface de linha de comando (Clap v4, derive).

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "dedup",
    version,
    about = "Engine ultrarrápida de deduplicação de arquivos",
    long_about = "Indexa um diretório, identifica arquivos com conteúdo idêntico \
                  (BLAKE3) e reporta o espaço recuperável.\n\n\
                  Para consultar (`find`) ou auditar (`verify`) um resultado, \
                  gere primeiro um manifesto com `scan --json --output <ARQ>`."
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        default_value_t = 0,
        value_name = "N",
        help = "Número de threads do Rayon (0 = detecta automaticamente)"
    )]
    pub threads: usize,

    #[arg(
        long,
        global = true,
        help = "Desativa as barras de progresso (elas já ficam ocultas quando stderr não é um terminal)"
    )]
    pub no_progress: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Varre um diretório e identifica duplicatas")]
    Scan(ScanArgs),

    #[command(about = "Consulta um manifesto JSON já gerado por `scan --json`")]
    Find(FindArgs),

    #[command(about = "Resolve duplicatas em um prompt interativo (simulação por padrão)")]
    Interactive(InteractiveArgs),

    #[command(about = "Valida a integridade dos arquivos citados em um manifesto")]
    Verify(VerifyArgs),
}

#[derive(Args)]
pub struct ScanArgs {
    #[arg(help = "Diretório alvo da varredura")]
    pub path: PathBuf,

    #[arg(long, help = "Imprime o manifesto em JSON em vez do resumo humano")]
    pub json: bool,

    #[arg(
        short,
        long,
        value_name = "ARQ",
        help = "Salva o manifesto JSON no arquivo indicado"
    )]
    pub output: Option<PathBuf>,

    #[arg(
        long,
        help = "Inclui arquivos de tamanho zero (todos os vazios são idênticos)"
    )]
    pub include_empty: bool,

    #[arg(
        long,
        help = "Inclui arquivos e diretórios ocultos (prefixo `.`, excluídos por padrão)"
    )]
    pub include_hidden: bool,
}

#[derive(Args)]
pub struct FindArgs {
    #[arg(
        value_name = "MANIFESTO",
        help = "Caminho do manifesto JSON gerado por `scan --json --output`"
    )]
    pub report: PathBuf,

    #[arg(
        value_name = "HASH",
        help = "Hash BLAKE3 completo ou prefixo com 8+ caracteres hexadecimais"
    )]
    pub hash: String,
}

#[derive(Args)]
pub struct InteractiveArgs {
    #[arg(help = "Diretório alvo da varredura")]
    pub path: PathBuf,

    #[arg(
        long,
        help = "Efetivamente apaga os arquivos não preservados (sem esta flag, apenas simula)"
    )]
    pub apply: bool,

    #[arg(
        long,
        help = "Inclui arquivos e diretórios ocultos (prefixo `.`, excluídos por padrão)"
    )]
    pub include_hidden: bool,
}

#[derive(Args)]
pub struct VerifyArgs {
    #[arg(
        value_name = "MANIFESTO",
        help = "Caminho do manifesto JSON gerado por `scan --json --output`"
    )]
    pub report: PathBuf,
}
