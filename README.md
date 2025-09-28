# PGN to Binpack Converter

Converts PGN files (currently fishtest format) to concatenated binpack format.

## Requirements

- Recent Rust toolchain

## Supported Formats

- `.pgn` files
- `.pgn.gz` files (decompressed on-the-fly using `MultiGzDecoder`)

## Usage

```
cargo install --git https://github.com/Disservin/pgn2binpack.git
pgn2binpack --help
```

```bash
Convert PGN chess files to binpack format

Usage: pgn-binpack.exe [OPTIONS] [DIR]

Arguments:
  [DIR]  Directory to search for PGN files

Options:
  -o, --output <OUTPUT>    Output binpack file
  -t, --threads <THREADS>  Number of threads to use (default: all CPU cores)
  -f, --force              Overwrite output file if it exists
  -m, --memory [<MEMORY>]  Use memory for intermediate storage (may use more RAM, but faster) [default: true] [possible values: true, false]
  -u, --unique [<FILE>]    Count unique positions in a binpack file
      --limit <LIMIT>      Limit the number of entries processed (only with --unique or --view)
  -v, --view <VIEW>        View contents of a binpack file
  -h, --help               Print help
```

### Examples

```bash
# Convert folder of PGN files
./target/release/pgn-binpack pgns -o output.binpack

# Convert single PGN file
./target/release/pgn-binpack 123456.pgn -o output.binpack

# Run without building
cargo run -r -- pgns -o output.binpack

# View binpack contents
cargo run -r -- --view ./fishpack32.binpack  | less

# View limited amount of binpack content
cargo run -r -- --view ./fishpack32.binpack --limit 10
```

## Options

### Performance

- `--threads N` - Limit thread count (default: all CPU cores)
- `--memory false` - Use temporary files instead of in-memory processing (for limited memory systems)

### Analytics

- `--unique [FILE]` - Count unique positions by Zobrist hash
- `--limit N` - Limit unique position count to reduce memory usage

## Memory Modes

- **Default (in-memory)**: Faster, requires more memory
- **Temporary files** (`--memory false`): Slower, uses less memory

## Status

Experimental - may contain bugs
