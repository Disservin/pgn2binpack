# pgn-binpack/pgn-binpack/README.md

# PGN Binpack

PGN Binpack is a Rust application designed to recursively search a directory for `.pgn.gz` files and create a binpack from those files. This project leverages the `sfbinpack` crate to efficiently manage and pack the collected data.

## Purpose

The main goal of this project is to provide a simple and efficient way to gather and compress PGN (Portable Game Notation) files, which are commonly used in chess applications. By packing these files into a single binary format, users can easily manage and distribute their chess game collections.

## Features

- Recursively searches specified directories for `.pgn.gz` files.
- Utilizes the `sfbinpack` crate to create a binpack from the collected files.
- Modular architecture for easy maintenance and extension.

## Usage

1. Clone the repository:

   ```
   git clone https://github.com/yourusername/pgn-binpack.git
   ```

2. Navigate to the project directory:

   ```
   cd pgn-binpack
   ```

3. Build the project:

   ```
   cargo build
   ```

4. Run the application, specifying the directory to search:

   ```
   cargo run -- <directory_path>
   ```

Replace `<directory_path>` with the path of the directory you want to search for `.pgn.gz` files.

## Contributing

Contributions are welcome! Please feel free to submit a pull request or open an issue for any suggestions or improvements.

## License

This project is licensed under the MIT License. See the LICENSE file for more details.