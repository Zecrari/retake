use std::io::IsTerminal;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process;

use clap::{Args, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use retake::config::{self, Config, Mode, Timing};

#[derive(Parser)]
#[command(
    name = "retake",
    version,
    about = "Record LLM API calls once, replay them deterministically in tests.",
    after_help = "Examples:\n  retake record -- npm test      Record real API calls while your tests run\n  retake replay -- npm test      Replay them offline (fails on unrecorded calls)\n  retake auto -- pytest          Replay what's recorded, record what's new\n  retake check                   Validate cassettes and scan for leaked secrets"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// More logging (-v, -vv). RUST_LOG overrides this.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Only log errors.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Forward calls to the real API and save them, replacing existing cassettes.
    Record(RunArgs),
    /// Serve calls from cassettes. Unrecorded calls fail; nothing reaches the network.
    Replay(RunArgs),
    /// Replay recorded calls and record anything new.
    Auto(RunArgs),
    /// Validate cassettes and scan them for leaked secrets.
    Check(CheckArgs),
}

#[derive(Args)]
struct RunArgs {
    /// Directory where cassettes are stored.
    #[arg(long, env = "RETAKE_CASSETTES", default_value = "cassettes")]
    cassettes: PathBuf,

    /// Cassette used when a request has no x-retake-session header.
    #[arg(short, long, env = "RETAKE_SESSION", default_value = "default")]
    session: String,

    /// Port to listen on (0 picks a free one).
    #[arg(short, long, env = "RETAKE_PORT", default_value_t = 0)]
    port: u16,

    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,

    /// Add or override an upstream, e.g. `ollama=http://localhost:11434`. Repeatable.
    #[arg(long = "upstream", value_name = "NAME=URL", value_parser = config::parse_upstream)]
    upstreams: Vec<(String, String)>,

    /// JSON field to ignore when matching requests, as a dotted path (e.g. `metadata.user_id`). Repeatable.
    #[arg(long = "ignore", value_name = "FIELD")]
    ignore: Vec<String>,

    /// How to pace replayed streams.
    #[arg(long, value_enum, default_value_t = Timing::Fast)]
    timing: Timing,

    /// Store event timing when recording, for `--timing realtime`. Makes cassette diffs noisier.
    #[arg(long)]
    record_timing: bool,

    /// Also record 429 and 5xx responses (skipped by default so SDK retries don't end up in cassettes).
    #[arg(long)]
    record_errors: bool,

    /// Command to run with the proxy. Without one, the proxy runs until Ctrl+C.
    #[arg(last = true, value_name = "COMMAND")]
    command: Vec<String>,
}

#[derive(Args)]
struct CheckArgs {
    /// Cassette files or directories.
    #[arg(default_value = "cassettes")]
    paths: Vec<PathBuf>,

    /// Redact detected secrets in place instead of failing.
    #[arg(long)]
    fix: bool,
}

fn main() {
    let cli = Cli::parse();
    init_logging(cli.verbose, cli.quiet);

    let result = match cli.command {
        Command::Record(args) => run(Mode::Record, args),
        Command::Replay(args) => run(Mode::Replay, args),
        Command::Auto(args) => run(Mode::Auto, args),
        Command::Check(args) => retake::check::check(&args.paths, args.fix).map(|clean| if clean { 0 } else { 1 }),
    };
    match result {
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("retake: error: {e:#}");
            process::exit(2);
        }
    }
}

fn run(mode: Mode, args: RunArgs) -> anyhow::Result<i32> {
    let mut config = Config::new(mode, args.cassettes);
    config.default_session = args.session;
    config.upstreams.extend(args.upstreams);
    config.ignore_fields = args.ignore;
    config.timing = args.timing;
    config.record_timing = args.record_timing;
    config.record_errors = args.record_errors;
    let bind = SocketAddr::new(args.host, args.port);

    tokio::runtime::Runtime::new()?.block_on(retake::runner::run(config, bind, args.command))
}

fn init_logging(verbose: u8, quiet: bool) {
    let level = match (quiet, verbose) {
        (true, _) => "error",
        (false, 0) => "info",
        (false, 1) => "debug",
        (false, _) => "trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(format!("retake={level}")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none())
        .without_time()
        .init();
}
