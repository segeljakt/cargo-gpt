# `cargo-gpt`

This is a basic utility which writes Rust source code in your crate plus markdown documentation and directory structure to standard output so it can be passed as input to ChatGPT.

## Installation

```sh
cargo install --git https://github.com/segeljakt/cargo-gpt
```

## Example usage

```sh
# Create a basic crate
cargo new --bin hello-world
cd hello-world

# Copy code to clipboard
cargo gpt
```

## Output

The following output is copied to clipboard:

```rs
// src/main.rs
fn main() {
    println!("Hello, world!");
}
```

## Advanced Features

Assume you have this project:

```rs
fn main() {
    interesting_function();
}

fn interesting_function() {
    println!("This is an interesting function!");
}

fn not_interesting_function() {
    println!("This is not an interesting function!");
}
```

You can filter the output to only include the interesting function:

```sh
cargo-gpt-test ❯ cargo gpt --functions
? Select functions/methods to include:
> [x] src/main.rs::interesting_function
  [x] src/main.rs::main
  [ ] src/main.rs::not_interesting_function
[↑↓/jk: navigate, space: toggle, a: select all, i: invert, r: clear all, enter: confirm]
```

Will copy the following to clipboard:

```rs
// src/main.rs
fn main() {
    interesting_function();
}

fn interesting_function() {
    println!("This is an interesting function!");
}

fn not_interesting_function()  { /* ... */ }
```

### More options

```sh
cargo gpt --print # Prints to stdout instead of copying to clipboard
cargo gpt --readme --toml # Includes README.md and Cargo.toml
cargo gpt --functions --only # Only include the selected functions
cargo gpt --all # Include all functions in all .rs files
cargo gpt explain # Run cargo check and copy the output to clipboard if there are errors
```



## Future Extensions

Any ideas for future extensions are welcome. Just open an issue or pull request :blush:
