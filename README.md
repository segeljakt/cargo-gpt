# `cargo-gpt`

ChatGPT can now handle up to 25000 words of input, making it possible to analyze medium-sized software projects / Rust crates. This is a basic utility which writes all Rust source code in your crate plus markdown documentation and directory structure to standard output so it can be passed as input to ChatGPT.

## Installation

```sh
cargo install cargo-gpt
```

## Usage

```sh
cargo new --bin hello-world
cd hello-world
cargo gpt
```

## Output

    ```toml
    // Cargo.toml
    [package]
    name = "hello-world"
    version = "0.1.0"
    edition = "2021"

    # See more keys and their definitions at https://doc.rust-lang.org/cargo/reference/manifest.html

    [dependencies]
    ```
    ```rs
    // src/main.rs
    fn main() {
        println!("Hello, world!");
    }
    ```

### Copying to clipboard

You can also copy the output directly to clipboard using:

```sh
cd /path/to/crate

cargo gpt | pbcopy  # macOS
cargo gpt | setclip # Linux
cargo gpt | clip    # Windows
```

## Future Extensions

Any ideas for future extensions are welcome. Just open an issue or pull request :blush:
