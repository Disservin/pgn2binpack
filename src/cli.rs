use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pgn2binpack")]
#[command(about = "Convert PGN chess files to binpack format", long_about = None)]
pub struct Cli {
    /// Directory to search for PGN files
    #[arg(value_name = "DIR")]
    pub input_dir: Option<PathBuf>,

    /// Output binpack file
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Number of threads to use (default: all CPU cores)
    #[arg(short, long)]
    pub threads: Option<usize>,

    /// Overwrite output file if it exists
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Use memory for intermediate storage (may use more RAM, but faster)
    #[arg(short, long, default_missing_value="true", default_value = "true", num_args=0..=1)]
    pub memory: bool,

    /// various analytics
    #[arg(short, long, num_args=0..=1, value_name = "FILE")]
    pub unique: Option<PathBuf>,

    #[arg(long, requires = "unique")]
    pub limit: Option<usize>,
}
