//! Ponto de entrada do binário.
//!
//! Aqui vive apenas a orquestração do CLI e o I/O de terminal: toda a lógica
//! está em `lib.rs` (e é exatamente a mesma que os testes exercitam).

use anyhow::Result;
use clap::Parser;
use console::style;
use dedup::cli::{Cli, Commands};
use dedup::pipeline::{DeduplicationEngine, EngineOptions};
use dedup::progress::Reporter;
use dedup::report;
use dedup::walker;
use dedup::{interactive, verify};
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();

    // `build_global` só falha se o pool já foi inicializado — ignorável aqui.
    if cli.threads > 0 {
        rayon::ThreadPoolBuilder::new()
            .num_threads(cli.threads)
            .build_global()
            .ok();
    }

    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{} {:#}", style("erro:").red().bold(), err);
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    // Comandos que varrem um diretório falham cedo se a raiz não é varrível —
    // senão uma raiz inexistente viraria um relatório vazio com exit 0.
    match &cli.command {
        Commands::Scan(args) => walker::ensure_scannable(&args.path)?,
        Commands::Interactive(args) => walker::ensure_scannable(&args.path)?,
        _ => {}
    }

    let reporter = if cli.no_progress {
        Reporter::hidden()
    } else {
        Reporter::stderr()
    };

    match cli.command {
        Commands::Scan(args) => {
            let engine = DeduplicationEngine::with_options(EngineOptions {
                include_empty: args.include_empty,
                include_hidden: args.include_hidden,
            });
            let scan = engine.run_with_progress(&args.path, &reporter);

            match (args.json, args.output) {
                // Manifesto para arquivo: nada de JSON misturado em stdout.
                (true, Some(path)) => {
                    report::save_report(&scan, &path)?;
                    eprintln!(
                        "{} {}",
                        style("manifesto salvo em:").green(),
                        path.display()
                    );
                }
                // Manifesto puro em stdout (progresso vai para stderr).
                (true, None) => println!("{}", report::to_json(&scan)?),
                (false, output) => {
                    report::print_human_report(&scan);
                    if let Some(path) = output {
                        report::save_report(&scan, &path)?;
                        println!(
                            "\n{} {}",
                            style("Manifesto salvo em:").green(),
                            path.display()
                        );
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Commands::Find(args) => {
            let scan = report::load_report(&args.report)?;
            let matches = report::find_groups(&scan, &args.hash);

            if matches.is_empty() {
                eprintln!(
                    "{}",
                    style(format!(
                        "nenhum grupo com o hash {} no manifesto {}",
                        args.hash,
                        args.report.display()
                    ))
                    .yellow()
                );
                return Ok(ExitCode::from(1));
            }

            println!(
                "{}",
                style(format!(
                    "{} grupo(s) para o hash {}:",
                    matches.len(),
                    args.hash
                ))
                .green()
                .bold()
            );
            for group in matches {
                println!(
                    "  hash {} | {} cada | {} cópias | recuperável: {}",
                    group.full_hash,
                    report::format_bytes(group.file_size),
                    group.files.len(),
                    report::format_bytes(group.recoverable_space())
                );
                for file in &group.files {
                    println!("    - {}", file.display());
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Commands::Interactive(args) => {
            // Vazios ficam de fora: apagar duplicatas vazias não recupera espaço.
            let engine = DeduplicationEngine::with_options(EngineOptions {
                include_empty: false,
                include_hidden: args.include_hidden,
            });
            let scan = engine.run_with_progress(&args.path, &reporter);
            let dry_run = !args.apply;
            interactive::run_interactive_resolution(&scan.duplicate_groups, dry_run)?;
            Ok(ExitCode::SUCCESS)
        }

        Commands::Verify(args) => {
            let summary = verify::verify_report_with_progress(&args.report, &reporter)?;
            if summary.has_problems() {
                // Código de saída != 0 permite usar `dedup verify` em CI/monitoração.
                Ok(ExitCode::from(1))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
    }
}
