use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;
use arboard::Clipboard;
use clap::Parser;
use clap::Subcommand;
use ignore::WalkBuilder;
use inquire::MultiSelect;
use ra_ap_syntax::ast::HasGenericParams;
use ra_ap_syntax::{
    ast::{self, AstNode, HasName},
    SourceFile,
};
use serde::Deserialize;
use serde::Serialize;

#[derive(Parser, Debug, Default)]
#[command(name = "cargo-gpt")]
#[command(about = "Dump your crate contents into a format which can be passed to GPT")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Use interactive mode to select functions/methods
    #[arg(short, long)]
    functions: bool,

    /// Path to config file (defaults to ~/.config/cargo-gpt/config.toml)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Generate default config file and exit
    #[arg(long)]
    generate_config: bool,

    /// Include README files (README.md, README.txt, etc.)
    #[arg(long)]
    readme: bool,

    /// Include Cargo.toml
    #[arg(long)]
    toml: bool,

    /// Select all functions (no filtering/elision)
    #[arg(long)]
    all: bool,

    /// Include only the selected functions (excludes imports, structs, traits, etc.)
    #[arg(long)]
    only: bool,

    #[arg(long)]
    print: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run cargo check and copy error output to clipboard for GPT analysis
    Explain {
        /// Additional context to include with the error
        #[arg(short, long)]
        context: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Config {
    toml: Option<bool>,
    readme: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct SelectionHistory {
    /// Map of project root path to selected functions
    selections: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    display_name: String, // filepath::function_name
}

impl Default for Config {
    fn default() -> Self {
        Self {
            toml: None,
            readme: None,
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Handle subcommands first
    if let Some(command) = args.command {
        match command {
            Commands::Explain { context } => {
                return explain_cargo_errors(context);
            }
        }
    }

    if args.generate_config {
        generate_config_file(args.config.as_ref())?;
        return Ok(());
    }

    let root = std::env::current_dir().context("Failed to get current directory")?;

    if args.print {
        // Write directly to stdout
        let stdout = io::stdout();
        let mut stdout_lock = stdout.lock();

        if args.functions {
            let selected_functions = interactive_select_functions(&root, args.config.as_ref())?;
            if selected_functions.is_empty() {
                eprintln!("No functions selected.");
                return Ok(());
            }
            generate_output_with_selected_functions(
                &root,
                &selected_functions,
                &args,
                &mut stdout_lock,
            )?;
        } else {
            if args.only {
                eprintln!("--only flag requires --functions flag");
                return Ok(());
            }
            let extensions = determine_extensions(&args)?;
            read_dir_to_writer(&root, &root, &extensions, &mut stdout_lock)?;
        }
    } else {
        // Collect output in a string buffer for clipboard
        let output_buffer = if args.functions {
            let selected_functions = interactive_select_functions(&root, args.config.as_ref())?;
            if selected_functions.is_empty() {
                eprintln!("No functions selected.");
                return Ok(());
            }

            let mut buffer = Vec::new();
            generate_output_with_selected_functions(
                &root,
                &selected_functions,
                &args,
                &mut buffer,
            )?;
            String::from_utf8(buffer).context("Invalid UTF-8 in output")?
        } else {
            if args.only {
                eprintln!("--only flag requires --functions flag");
                return Ok(());
            }
            let extensions = determine_extensions(&args)?;
            let mut buffer = Vec::new();
            read_dir_to_writer(&root, &root, &extensions, &mut buffer)?;
            String::from_utf8(buffer).context("Invalid UTF-8 in output")?
        };

        let output_buffer = output_buffer.trim();

        if output_buffer.is_empty() {
            eprintln!("No content generated with the current selection.");
            return Ok(());
        }

        // Copy to clipboard
        Clipboard::new()
            .context("Failed to access clipboard")?
            .set_text(output_buffer)
            .context("Failed to copy to clipboard")?;

        eprintln!(
            "Content copied to clipboard! You can now paste it into your favorite AI assistant."
        );
    }

    Ok(())
}

fn determine_extensions(args: &Args) -> Result<HashSet<String>> {
    // Priority order:
    // 1. Command line arguments
    // 2. Config file
    // 3. Defaults

    let mut extensions = HashSet::new();

    extensions.insert("rs".to_string()); // Always include Rust files

    // Use config file extensions
    let config = load_config(args.config.as_ref())?;

    // Add specific files based on flags
    if args.all || args.readme || config.readme.is_some_and(|v| v) {
        extensions.insert("README.md".to_string());
    }

    if args.all || args.toml || config.toml.is_some_and(|v| v) {
        extensions.insert("Cargo.toml".to_string());
    }

    Ok(extensions)
}

fn load_config(config_path: Option<&PathBuf>) -> Result<Config> {
    let config_file = if let Some(path) = config_path {
        path.clone()
    } else {
        get_default_config_path()?
    };

    if !config_file.exists() {
        // Use default config if file doesn't exist
        return Ok(Config::default());
    }

    let config_content = fs::read_to_string(&config_file).context("Failed to read config file")?;

    toml::from_str(&config_content).context("Failed to parse config file")
}

fn explain_cargo_errors(additional_context: Option<String>) -> Result<()> {
    println!("Running cargo check...");

    // Run cargo check and capture both stdout and stderr
    let output = Command::new("cargo")
        .arg("check")
        .arg("--message-format=human")
        .output()
        .context("Failed to run cargo check - make sure you're in a Rust project directory")?;

    // Combine stdout and stderr
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined_output = format!("{}{}", stdout, stderr).trim().to_string();

    if combined_output.is_empty() {
        println!("✅ No errors to explain! cargo check completed successfully.");
        return Ok(());
    }

    // Create the prompt for GPT
    let mut prompt = String::from("Help me understand and fix these Rust compilation errors:\n\n");

    if let Some(context) = additional_context {
        prompt.push_str(&format!("Additional context: {}\n\n", context));
    }

    prompt.push_str("```\n");
    prompt.push_str(&combined_output);
    prompt.push_str("\n```\n\n");
    prompt.push_str("Please explain what's wrong and suggest how to fix it.");

    // Copy to clipboard
    let mut clipboard = Clipboard::new().context("Failed to access clipboard")?;

    clipboard
        .set_text(&prompt)
        .context("Failed to copy to clipboard")?;

    println!("📋 Error output copied to clipboard!");
    println!("You can now paste it into your favorite AI assistant.");

    if !combined_output.is_empty() {
        println!("\n--- Error Output ---");
        println!("{}", combined_output);
    }

    Ok(())
}

fn generate_config_file(config_path: Option<&PathBuf>) -> Result<()> {
    let config_file = if let Some(path) = config_path {
        path.clone()
    } else {
        get_default_config_path()?
    };

    // Create directory if it doesn't exist
    if let Some(parent) = config_file.parent() {
        fs::create_dir_all(parent).context("Failed to create config directory")?;
    }

    let default_config = Config::default();
    let config_content =
        toml::to_string_pretty(&default_config).context("Failed to serialize default config")?;

    fs::write(&config_file, config_content).context("Failed to write config file")?;

    println!("Generated config file at: {}", config_file.display());
    println!("You can edit this file to customize which file types to include.");

    Ok(())
}

fn get_default_config_path() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().context("Failed to get home directory")?;

    let config_dir = home_dir.join(".config").join("cargo-gpt");

    Ok(config_dir.join("config.toml"))
}

fn extract_functions_from_rust_file(file_path: &Path, root: &Path) -> Result<Vec<FunctionInfo>> {
    let content = fs::read_to_string(file_path).context("Failed to read file")?;
    let parsed = SourceFile::parse(&content, ra_ap_syntax::Edition::Edition2024);
    let syntax_tree = parsed.tree();

    let relative_path = file_path
        .strip_prefix(root)
        .unwrap_or(file_path)
        .display()
        .to_string();

    let mut functions = Vec::new();

    // Extract standalone functions
    for func in syntax_tree.syntax().descendants().filter_map(ast::Fn::cast) {
        if let Some(name) = func.name() {
            let function_name = name.text().to_string();
            let display_name = format!("{}::{}", relative_path, function_name);
            functions.push(FunctionInfo { display_name });
        }
    }

    // Extract methods from impl blocks
    for impl_block in syntax_tree
        .syntax()
        .descendants()
        .filter_map(ast::Impl::cast)
    {
        let type_name = impl_block
            .self_ty()
            .and_then(|ty| {
                // Try to get the type name - this is a simplified approach
                ty.syntax().first_token().map(|t| t.text().to_string())
            })
            .unwrap_or_else(|| "Unknown".to_string());

        for func in impl_block.syntax().descendants().filter_map(ast::Fn::cast) {
            if let Some(name) = func.name() {
                let function_name = name.text().to_string();
                let display_name = format!("{}::{}::{}", relative_path, type_name, function_name);
                functions.push(FunctionInfo { display_name });
            }
        }
    }

    // Extract methods from trait impl blocks
    for impl_block in syntax_tree
        .syntax()
        .descendants()
        .filter_map(ast::Impl::cast)
    {
        if let Some(trait_) = impl_block.trait_() {
            let trait_name = trait_
                .syntax()
                .last_token()
                .map(|t| t.text().to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            let type_name = impl_block
                .self_ty()
                .and_then(|ty| ty.syntax().first_token().map(|t| t.text().to_string()))
                .unwrap_or_else(|| "Unknown".to_string());

            for func in impl_block.syntax().descendants().filter_map(ast::Fn::cast) {
                if let Some(name) = func.name() {
                    let function_name = name.text().to_string();
                    let display_name = format!(
                        "{}::{}::{}::{}",
                        relative_path, type_name, trait_name, function_name
                    );
                    functions.push(FunctionInfo { display_name });
                }
            }
        }
    }

    Ok(functions)
}

fn interactive_select_functions(
    root: &Path,
    _config_path: Option<&PathBuf>,
) -> Result<Vec<String>> {
    // First, collect all Rust files and extract functions
    let extensions: HashSet<String> = vec!["rs".to_string()].into_iter().collect(); // Focus on Rust files for function extraction

    let mut rust_files = Vec::new();
    collect_files(root, &extensions, &mut rust_files)?;

    if rust_files.is_empty() {
        eprintln!("No Rust files found in the project");
        return Ok(Vec::new());
    }

    // Extract all functions from all Rust files
    let mut all_functions = Vec::new();
    for file_path in rust_files {
        match extract_functions_from_rust_file(&file_path, root) {
            Ok(mut functions) => all_functions.append(&mut functions),
            Err(e) => eprintln!("Warning: Failed to parse {}: {}", file_path.display(), e),
        }
    }

    if all_functions.is_empty() {
        eprintln!("No functions found in Rust files");
        return Ok(Vec::new());
    }

    // Sort functions for consistent ordering
    all_functions.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    let function_display_names: Vec<String> = all_functions
        .iter()
        .map(|f| f.display_name.clone())
        .collect();

    // Load previous selections
    let history = load_selection_history()?;
    let project_key = root.display().to_string();
    let previous_selections = history.selections.get(&project_key);

    // Determine default selections
    let default_selected: Vec<usize> = if let Some(prev_functions) = previous_selections {
        // Use previous selection
        function_display_names
            .iter()
            .enumerate()
            .filter_map(|(i, func)| {
                if prev_functions.contains(func) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    } else {
        // Select all functions by default
        (0..function_display_names.len()).collect()
    };

    let selected_names = MultiSelect::new(
        "Select functions/methods to include:",
        function_display_names.clone(),
    )
    .with_default(&default_selected)
    .with_vim_mode(true)
    .with_page_size(20) // Show 20 items at once instead of default (7)
    .with_help_message(
        "↑↓/jk: navigate, space: toggle, a: select all, i: invert, r: clear all, enter: confirm",
    )
    .prompt()
    .context("Failed to get user selection")?;

    // Save the selection
    save_selection_history(&project_key, &selected_names)?;

    Ok(selected_names)
}

// Updated to take a generic writer
fn generate_output_with_selected_functions<W: Write>(
    root: &Path,
    selected_functions: &[String],
    args: &Args,
    writer: &mut W,
) -> Result<()> {
    let extensions = determine_extensions(args)?;

    let mut processed_files = HashSet::new();

    // Collect all files
    let mut all_files = Vec::new();
    collect_files(root, &extensions, &mut all_files)?;

    for file_path in all_files {
        if processed_files.contains(&file_path) {
            continue;
        }
        processed_files.insert(file_path.clone());

        let relative_path = file_path
            .strip_prefix(root)
            .unwrap_or(&file_path)
            .display()
            .to_string();

        // For Rust files, we need to transform them based on selected functions
        if file_path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let content = fs::read_to_string(&file_path).context("Failed to read file")?;

            // Extract function names that should be kept for this specific file
            let functions_to_keep: Vec<String> = selected_functions
                .iter()
                .filter_map(|display_name| {
                    if display_name.starts_with(&format!("{}::", relative_path)) {
                        // Extract just the function name from display_name
                        display_name.split("::").last().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            let transformed_content = if args.only {
                // Extract only the selected functions
                extract_only_selected_functions(&content, &functions_to_keep)
            } else if functions_to_keep.is_empty() {
                // If no functions are selected, elide all function bodies
                transform_rust_file(&content, &[])
            } else {
                // Keep selected functions, elide others
                transform_rust_file(&content, &functions_to_keep)
            };

            if !transformed_content.trim().is_empty() {
                writeln!(writer, "// {}", relative_path)?;
                writeln!(writer, "{}", transformed_content)?;
            }
        } else if !args.only {
            // For non-Rust files, include them only if not using --only
            let file_content = fs::read_to_string(&file_path).context("Failed to read file")?;
            let content_with_newline = if file_content.ends_with('\n') {
                file_content
            } else {
                format!("{}\n", file_content)
            };
            writeln!(writer, "// {}", relative_path)?;
            write!(writer, "{}", content_with_newline)?;
            writeln!(writer)?;
        }
    }

    Ok(())
}

fn extract_only_selected_functions(source: &str, functions_to_keep: &[String]) -> String {
    if functions_to_keep.is_empty() {
        return String::new();
    }

    let parsed = SourceFile::parse(source, ra_ap_syntax::Edition::Edition2024);
    let root = parsed.tree();
    let mut result = String::new();

    // Extract standalone functions
    for func in root.syntax().descendants().filter_map(ast::Fn::cast) {
        if let Some(name) = func.name() {
            let func_name = name.text().to_string();
            if functions_to_keep.contains(&func_name) {
                let func_text = func.syntax().text().to_string();
                result.push_str(&func_text);
                result.push_str("\n\n");
            }
        }
    }

    // Extract methods from impl blocks
    for impl_block in root.syntax().descendants().filter_map(ast::Impl::cast) {
        let mut impl_functions = Vec::new();
        let mut has_selected_functions = false;

        // Check if this impl block has any selected functions
        for func in impl_block.syntax().descendants().filter_map(ast::Fn::cast) {
            if let Some(name) = func.name() {
                let func_name = name.text().to_string();
                if functions_to_keep.contains(&func_name) {
                    impl_functions.push(func);
                    has_selected_functions = true;
                }
            }
        }

        // If we have selected functions in this impl block, include them
        if has_selected_functions {
            // Add impl signature
            result.push_str("impl");
            if let Some(generic_params) = impl_block.generic_param_list() {
                result.push_str(&generic_params.syntax().text().to_string());
            }
            if let Some(self_ty) = impl_block.self_ty() {
                result.push_str(" ");
                result.push_str(&self_ty.syntax().text().to_string());
            }
            if let Some(trait_) = impl_block.trait_() {
                result.push_str(" for ");
                result.push_str(&trait_.syntax().text().to_string());
            }
            if let Some(where_clause) = impl_block.where_clause() {
                result.push_str(" ");
                result.push_str(&where_clause.syntax().text().to_string());
            }
            result.push_str(" {\n");

            // Add selected functions
            for func in impl_functions {
                let func_text = func.syntax().text().to_string();
                // Indent the function
                for line in func_text.lines() {
                    result.push_str("    ");
                    result.push_str(line);
                    result.push('\n');
                }
                result.push('\n');
            }

            result.push_str("}\n\n");
        }
    }

    result
}

fn transform_rust_file(source: &str, functions_to_keep: &[String]) -> String {
    let parsed = SourceFile::parse(source, ra_ap_syntax::Edition::Edition2024);
    let root = parsed.tree();

    let mut replacements = Vec::new();

    // Find all function bodies that need to be elided
    for func in root.syntax().descendants().filter_map(ast::Fn::cast) {
        if let Some(name) = func.name() {
            let func_name = name.text().to_string();
            // If this function is NOT in the keep list, elide its body
            if !functions_to_keep.contains(&func_name) {
                if let Some(body) = func.body() {
                    let range = body.syntax().text_range();
                    replacements.push((range, " { /* ... */ }".to_string()));
                }
            }
        }
    }

    // Sort replacements by position (reverse order to avoid offset issues)
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start()));

    // Apply replacements
    let mut result = source.to_string();
    for (range, replacement) in replacements {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        result.replace_range(start..end, &replacement);
    }

    result
}

fn collect_files(
    path: &Path,
    extensions: &HashSet<String>,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    let walk = WalkBuilder::new(path)
        .filter_entry(|e| {
            e.file_name()
                .to_str()
                .map(|s| !s.starts_with('.') && s != "target")
                .unwrap_or(false)
        })
        .build();

    for entry in walk.filter_map(Result::ok) {
        if entry
            .file_type()
            .context("Failed to get file type")?
            .is_file()
        {
            if should_include_file(entry.path(), extensions) {
                files.push(entry.path().to_path_buf());
            }
        }
    }

    Ok(())
}

fn load_selection_history() -> Result<SelectionHistory> {
    let history_path = get_selection_history_path()?;

    if !history_path.exists() {
        return Ok(SelectionHistory::default());
    }

    let history_content =
        fs::read_to_string(&history_path).context("Failed to read selection history")?;

    serde_json::from_str(&history_content)
        .context("Failed to parse selection history")
        .or_else(|_| Ok(SelectionHistory::default()))
}

fn save_selection_history(project_key: &str, selected_functions: &[String]) -> Result<()> {
    let history_path = get_selection_history_path()?;

    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).context("Failed to create config directory")?;
    }

    let mut history = load_selection_history().unwrap_or_default();
    history
        .selections
        .insert(project_key.to_string(), selected_functions.to_vec());

    let history_content =
        serde_json::to_string_pretty(&history).context("Failed to serialize selection history")?;

    fs::write(&history_path, history_content).context("Failed to write selection history")?;

    Ok(())
}

fn get_selection_history_path() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().context("Failed to get home directory")?;

    let config_dir = home_dir.join(".config").join("cargo-gpt");

    Ok(config_dir.join("history.json"))
}

fn should_include_file(path: &Path, extensions: &HashSet<String>) -> bool {
    // Handle special files by name
    if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
        if extensions.contains(filename) {
            return true;
        }
    }

    // Handle files by extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        return extensions.contains(ext);
    }

    false
}

fn read_file_to_writer<W: Write>(path: &Path, root: &Path, writer: &mut W) -> Result<()> {
    let file_content = fs::read_to_string(path).context("Failed to read file")?;
    let relative_path = path
        .strip_prefix(root)
        .context("Failed to strip prefix")?
        .display();

    writeln!(writer, "// {}", relative_path)?;
    write!(writer, "{}", file_content)?;
    if !file_content.ends_with('\n') {
        writeln!(writer)?;
    }
    writeln!(writer)?;

    Ok(())
}

fn read_dir_to_writer<W: Write>(
    path: &Path,
    root: &Path,
    extensions: &HashSet<String>,
    writer: &mut W,
) -> Result<()> {
    let walk = WalkBuilder::new(path)
        .filter_entry(|e| {
            e.file_name()
                .to_str()
                .map(|s| !s.starts_with('.') && s != "target" && s != "node_modules")
                .unwrap_or(false)
        })
        .build();

    for entry in walk.filter_map(Result::ok) {
        if entry
            .file_type()
            .context("Failed to get file type")?
            .is_file()
        {
            if should_include_file(entry.path(), extensions) {
                read_file_to_writer(entry.path(), root, writer)?;
            }
        }
    }

    Ok(())
}

