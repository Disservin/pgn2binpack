# PGN to Binpack Converter

Converts PGN files to concatenated binpack format for chess position storage.

## Requirements

- Recent Rust toolchain

## Supported Formats

- `.pgn` files
- `.pgn.gz` files (decompressed on-the-fly)

## Installation

```bash
cargo install --git https://github.com/Disservin/pgn2binpack.git
pgn2binpack --help
```

## Usage

```bash
Convert PGN chess files to binpack format

Usage: pgn-binpack.exe [OPTIONS] [DIR]

Arguments:
  [DIR]  Directory to search for PGN files

Options:
  -o, --output <OUTPUT>    Output binpack file
  -t, --threads <THREADS>  Number of threads to use (default: all CPU cores)
  -f, --force              Overwrite output file if it exists
  -m, --memory [<MEMORY>]  Use memory for intermediate storage [default: true]
  -u, --unique [<FILE>]    Count unique positions in a binpack file
      --limit <LIMIT>      Limit entries processed (with --unique or --view)
  -v, --view <VIEW>        View contents of a binpack file
  -h, --help               Print help
```

## Examples

### Basic Conversion

```bash
# Convert all files in "pgns" directory
pgn-binpack pgns -o output.binpack

# Convert single file
pgn-binpack game.pgn -o output.binpack

# Force overwrite existing output
pgn-binpack pgns -o output.binpack --force
```

### Analysis

```bash
# View binpack contents
pgn-binpack --view output.binpack

# View first 100 positions
pgn-binpack --view output.binpack --limit 100

# Count unique positions
pgn-binpack --unique output.binpack
```

## Performance

- **Memory mode** (default): Faster processing, higher RAM usage
- **Disk mode** (`--memory false`): Lower RAM usage, slower processing
- **Threading**: Defaults to all CPU cores, tune with `--threads`

## Status

Experimental - may contain bugs
