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

    /// Count unique positions in a binpack file
    #[arg(short, long, num_args=0..=1, value_name = "FILE")]
    pub unique: Option<PathBuf>,

    /// Limit the number of entries processed (only with --unique or --view)
    #[arg(long, requires("unique"), requires("view"))]
    pub limit: Option<usize>,

    /// View contents of a binpack file
    #[arg(short, long)]
    pub view: Option<PathBuf>,

    /// Rescore entries in a binpack file using a UCI engine
    #[arg(long, value_name = "FILE")]
    pub rescore: Option<PathBuf>,

    /// Output binpack file that will receive rescored entries
    #[arg(long, value_name = "FILE")]
    pub rescore_output: Option<PathBuf>,

    /// Path to the UCI engine binary
    #[arg(long, value_name = "PATH")]
    pub engine: Option<PathBuf>,

    /// Depth to search when rescoring (default: 5000)
    #[arg(long, value_name = "NODES")]
    pub rescore_nodes: Option<usize>,
}
