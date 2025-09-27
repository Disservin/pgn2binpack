# PGN to Binpack Converter

Converts PGN files (currently fishtest format) to concatenated binpack format.

## Requirements

- Recent Rust toolchain

## Supported Formats

- `.pgn` files
- `.pgn.gz` files (decompressed on-the-fly using `MultiGzDecoder`)

## Installation

```bash
cargo build --release
```

## Usage

```bash
./target/release/pgn-binpack [INPUT] -o [OUTPUT]
```

### Examples

```bash
# Convert folder of PGN files
./target/release/pgn-binpack pgns -o output.binpack

# Convert single PGN file
./target/release/pgn-binpack 123456.pgn -o output.binpack

# Run without building
cargo run -r -- pgns -o output.binpack
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
