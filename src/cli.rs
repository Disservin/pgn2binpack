use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pgn2binpack")]
#[command(about = "Convert PGN chess files to binpack format", long_about = None)]
pub struct Cli {
    /// Directory to search for PGN files
    #[arg(value_name = "DIR")]
    pub input_dir: PathBuf,

    /// Output binpack file
    #[arg(short, long, default_value = "output.binpack")]
    pub output: PathBuf,

    /// Number of threads to use (default: all CPU cores)
    #[arg(short, long)]
    pub threads: Option<usize>,

    /// Overwrite output file if it exists
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Use memory for intermediate storage (may use more RAM, but faster)
    // #[arg(short = 'm', long, default_value = "true")]
    #[arg(short, long, default_missing_value="true", num_args=0..=1)]
    pub memory: bool,
}
