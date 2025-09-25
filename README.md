Convert fishtest (for now) pgn files to a single concatenated binpack file.
Requires a recent rust toolchain.

Will work in parallel on all threads and for now create temporary files
in the working directory named
`<output>.thread_<n>` which are concatenated at the end to `<output>`.

not yet tested much, there might be some bugs

```bash
cargo build --release
./target/release/pgn-binpack --help
```

example, if you have a folder `pgns` with fishtest pgn files

```bash
./target/release/pgn-binpack pgns -o test2.binpack
```

or just one pgn file

```bash
./target/release/pgn-binpack 123456.pgn -o test
```

as always you can also run with `cargo run -r -- ...` instead doing a separate build step.
