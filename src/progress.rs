//! Barras de progresso via `indicatif`.
//!
//! O reporter é *best-effort*: quando `stderr` não é um terminal, o
//! `ProgressDrawTarget::stderr()` do `indicatif` se comporta como um alvo
//! oculto, e `Reporter::hidden()` desliga tudo explicitamente (`--no-progress`).
//! Como o progresso vai para **stderr**, `sosia scan --json` pode ser redirecionado
//! para um arquivo/pipe sem contaminar o manifesto JSON.
//!
//! `indicatif::ProgressBar` é `Send + Sync`, então o mesmo `&Reporter` pode ser
//! usado de dentro das closures paralelas do Rayon.

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Template de barra usado nas etapas com total conhecido.
const STAGE_TEMPLATE: &str = "{spinner:.cyan} {msg:32} [{bar:28}] {pos}/{len} ({eta})";
/// Template usado na varredura, cujo total só é conhecido no final.
const WALK_TEMPLATE: &str = "{spinner:.cyan} {msg:32} {pos} arquivos";

#[derive(Clone)]
pub struct Reporter {
    inner: Option<Inner>,
}

#[derive(Clone)]
struct Inner {
    walk: ProgressBar,
    partial: ProgressBar,
    full: ProgressBar,
    verify: ProgressBar,
}

impl Reporter {
    /// Reporter que não produz saída alguma.
    pub fn hidden() -> Self {
        Self { inner: None }
    }

    /// Reporter escrevendo em `stderr` (oculto automaticamente quando não é TTY).
    pub fn stderr() -> Self {
        // `ProgressDrawTarget` não é `Clone`: cada barra cria o seu próprio alvo
        // (as quatro compartilham o mesmo stderr por baixo dos panos).
        Self {
            inner: Some(Inner {
                walk: bar(None, WALK_TEMPLATE),
                partial: bar(Some(0), STAGE_TEMPLATE),
                full: bar(Some(0), STAGE_TEMPLATE),
                verify: bar(Some(0), STAGE_TEMPLATE),
            }),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.inner.is_some()
    }

    pub fn start_walk(&self) {
        if let Some(inner) = &self.inner {
            inner.walk.reset_elapsed();
            inner.walk.set_message("varrendo o sistema de arquivos");
        }
    }

    pub fn inc_walk(&self) {
        if let Some(inner) = &self.inner {
            inner.walk.inc(1);
        }
    }

    pub fn finish_walk(&self, indexed_files: usize) {
        if let Some(inner) = &self.inner {
            inner.walk.set_message("varredura concluída");
            inner.walk.finish_with_message(format!(
                "varredura concluída: {} arquivo(s) indexado(s)",
                indexed_files
            ));
        }
    }

    pub fn start_partial(&self, total: usize) {
        self.start_stage(StageKind::Partial, total, "hash parcial (4 KiB)");
    }

    pub fn inc_partial(&self) {
        self.inc_stage(StageKind::Partial);
    }

    pub fn finish_partial(&self) {
        self.finish_stage(StageKind::Partial, "hash parcial");
    }

    pub fn start_full(&self, total: usize) {
        self.start_stage(StageKind::Full, total, "hash completo (BLAKE3)");
    }

    pub fn inc_full(&self) {
        self.inc_stage(StageKind::Full);
    }

    pub fn finish_full(&self) {
        self.finish_stage(StageKind::Full, "hash completo");
    }

    pub fn start_verify(&self, total: usize) {
        self.start_stage(StageKind::Verify, total, "verificando integridade");
    }

    pub fn inc_verify(&self) {
        self.inc_stage(StageKind::Verify);
    }

    pub fn finish_verify(&self) {
        self.finish_stage(StageKind::Verify, "verificação");
    }

    fn start_stage(&self, kind: StageKind, total: usize, message: &'static str) {
        if let Some(inner) = &self.inner {
            let stage = inner.select(kind);
            stage.set_length(total as u64);
            stage.reset_elapsed();
            stage.set_message(message);
        }
    }

    fn inc_stage(&self, kind: StageKind) {
        if let Some(inner) = &self.inner {
            inner.select(kind).inc(1);
        }
    }

    fn finish_stage(&self, kind: StageKind, label: &str) {
        if let Some(inner) = &self.inner {
            inner
                .select(kind)
                .finish_with_message(format!("{} concluído", label));
        }
    }
}

#[derive(Clone, Copy)]
enum StageKind {
    Partial,
    Full,
    Verify,
}

impl Inner {
    fn select(&self, kind: StageKind) -> &ProgressBar {
        match kind {
            StageKind::Partial => &self.partial,
            StageKind::Full => &self.full,
            StageKind::Verify => &self.verify,
        }
    }
}

/// `len = None` produz uma barra sem total (spinner + contador); `Some(0)` é
/// usado quando o total só é conhecido depois, via `set_length`.
fn bar(len: Option<u64>, template: &'static str) -> ProgressBar {
    let bar = ProgressBar::with_draw_target(len, ProgressDrawTarget::stderr());
    let style = ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-");
    bar.set_style(style);
    bar
}
