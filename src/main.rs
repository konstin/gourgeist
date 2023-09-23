use camino::Utf8PathBuf;
use clap::Parser;
use std::process::ExitCode;
use std::time::Instant;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};
use virtualenv_rs::{create_venv, get_interpreter_info};

#[derive(Parser, Debug)]
struct Cli {
    path: Option<Utf8PathBuf>,
    #[clap(short, long)]
    python: Option<Utf8PathBuf>,
    #[clap(long)]
    bare: bool,
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let location = cli.path.unwrap_or(Utf8PathBuf::from(".venv-rs"));
    let base_python = cli
        .python
        .unwrap_or(Utf8PathBuf::from("/home/konsti/.local/bin/python3.11"));
    let data = get_interpreter_info(&base_python)?;

    create_venv(&location, &base_python, &data, cli.bare)?;

    Ok(())
}

fn main() -> ExitCode {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let start = Instant::now();
    let result = run();
    info!("Took {}ms", start.elapsed().as_millis());
    if let Err(err) = result {
        eprintln!("💥 virtualenv creator failed");
        for err in err.chain() {
            eprintln!("  Caused by: {}", err);
        }
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
