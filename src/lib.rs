//! # `dedup` — engine de deduplicação de arquivos
//!
//! Árvore única de módulos: a biblioteca declara os módulos e o binário
//! (`src/main.rs`) consome exatamente este código via `use dedup::...`. Assim,
//! os testes de integração (`tests/`) exercitam o mesmo código que o binário
//! executa em produção — sem duas árvores de módulos compiladas em paralelo.

pub mod cli;
pub mod hasher;
pub mod interactive;
pub mod model;
pub mod pipeline;
pub mod progress;
pub mod report;
pub mod verify;
pub mod walker;
