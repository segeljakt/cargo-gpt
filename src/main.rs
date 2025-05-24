use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
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
use serde::Deserialize;
use serde::Serialize;

#[derive(Parser, Debug)]
#[command(name = "cargo-gpt")]
#[command(about = "Dump your crate contents into a format which can be passed to GPT")]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    /// File extensions to include (e.g., rs,toml,go)
    #[arg(short, long, value_delimiter = ',')]
    extensions: Option<Vec<String>>,

    /// Use interactive mode to select files
    #[arg(short, long)]
    interactive: bool,

    /// Path to config file (defaults to ~/.config/cargo-gpt/config.toml)
    #[arg(short, long)]
    config: Option<PathBuf>,

    /// Show default extensions and exit
    #[arg(long)]
    show_defaults: bool,

    /// Generate default config file and exit
    #[arg(long)]
    generate_config: bool,
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
    /// Default file extensions to include
    default_extensions: Vec<String>,
    /// Additional extensions to always include
    always_include: Option<Vec<String>>,
    /// Extensions to always exclude
    exclude: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct SelectionHistory {
    /// Map of project root path to selected files
    selections: HashMap<String, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_extensions: vec![
                "rs".to_string(),
                "toml".to_string(),
                "md".to_string(),
                "txt".to_string(),
            ],
            always_include: Some(vec!["Cargo.toml".to_string(), "README.md".to_string()]),
            exclude: None,
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

    if args.show_defaults {
        let config = Config::default();
        println!(
            "Default extensions: {}",
            config.default_extensions.join(", ")
        );
        return Ok(());
    }

    if args.generate_config {
        generate_config_file(args.config.as_ref())?;
        return Ok(());
    }

    let root = std::env::current_dir().context("Failed to get current directory")?;

    // Collect output in a string buffer
    let mut output_buffer = String::new();

    if args.interactive {
        let filepaths = interactive_select_files(&root, args.config.as_ref())?;
        if filepaths.is_empty() {
            eprintln!("No files selected.");
            return Ok(());
        }
        for filepath in filepaths {
            output_buffer.push_str(&read_file(&filepath, &root)?);
        }
    } else {
        let extensions = determine_extensions(&args)?;
        output_buffer = read_dir_to_string(&root, &root, &extensions)?;
    }

    let output_buffer = output_buffer.trim();

    if output_buffer.is_empty() {
        eprintln!("No files found matching the criteria.");
        return Ok(());
    }

    // Copy to clipboard

    Clipboard::new()
        .context("Failed to access clipboard")?
        .set_text(output_buffer)
        .context("Failed to copy to clipboard")?;

    eprintln!("Content copied to clipboard! You can now paste it into your favorite AI assistant.");

    Ok(())
}

fn determine_extensions(args: &Args) -> Result<HashSet<String>> {
    // Priority order:
    // 1. Command line arguments
    // 2. Config file
    // 3. Defaults

    if let Some(ref ext_list) = args.extensions {
        return Ok(ext_list.iter().cloned().collect());
    }

    let config = load_config(args.config.as_ref())?;

    // Use config file extensions
    let mut extensions: HashSet<String> = config.default_extensions.into_iter().collect();

    if let Some(always_include) = config.always_include {
        extensions.extend(always_include);
    }

    if let Some(exclude) = config.exclude {
        for ext in exclude {
            extensions.remove(&ext);
        }
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

fn interactive_select_files(root: &Path, config_path: Option<&PathBuf>) -> Result<Vec<PathBuf>> {
    // First, collect all files that match our criteria
    let config = load_config(config_path)?;
    let mut extensions: HashSet<String> = config.default_extensions.into_iter().collect();

    if let Some(always_include) = config.always_include {
        extensions.extend(always_include);
    }

    let mut available_files = Vec::new();
    collect_files(root, &extensions, &mut available_files)?;

    if available_files.is_empty() {
        eprintln!("No files found matching the configured extensions");
        eprintln!("You can generate a config file with: cargo gpt --generate-config");
        return Ok(Vec::new());
    }

    // Sort files for consistent ordering
    available_files.sort();

    // Convert to relative path strings for display
    let file_display_names: Vec<String> = available_files
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect();

    // Load previous selections
    let history = load_selection_history()?;
    let project_key = root.display().to_string();
    let previous_selections = history.selections.get(&project_key);

    // Determine default selections (all files by default, or previous selection)
    let default_selected: Vec<usize> = if let Some(prev_files) = previous_selections {
        // Use previous selection
        file_display_names
            .iter()
            .enumerate()
            .filter_map(|(i, file)| {
                if prev_files.contains(file) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    } else {
        // Select all files by default
        (0..file_display_names.len()).collect()
    };

    let selected_names = MultiSelect::new("Select files to include:", file_display_names.clone())
        .with_default(&default_selected)
        .with_vim_mode(true)
        .with_help_message(
            "j/k: navigate, space: toggle, enter: complete, h: clear all, l: select all",
        )
        .prompt()
        .context("Failed to get user selection")?;

    // Save the selection
    save_selection_history(&project_key, &selected_names)?;

    // Convert back to full paths
    let selected_paths: Vec<PathBuf> = selected_names
        .into_iter()
        .filter_map(|name| {
            file_display_names
                .iter()
                .position(|f| f == &name)
                .map(|index| available_files[index].clone())
        })
        .collect();

    Ok(selected_paths)
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

fn save_selection_history(project_key: &str, selected_files: &[String]) -> Result<()> {
    let history_path = get_selection_history_path()?;

    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent).context("Failed to create config directory")?;
    }

    let mut history = load_selection_history().unwrap_or_default();
    history
        .selections
        .insert(project_key.to_string(), selected_files.to_vec());

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

fn read_file(path: &Path, root: &Path) -> Result<String> {
    let file_content = fs::read_to_string(path).context("Failed to read file")?;
    let relative_path = path
        .strip_prefix(root)
        .context("Failed to strip prefix")?
        .display();

    // Ensure file content ends with newline, then add another for separation
    let content_with_newline = if file_content.ends_with('\n') {
        file_content
    } else {
        format!("{}\n", file_content)
    };

    Ok(format!("// {}\n{}\n", relative_path, content_with_newline))
}

fn read_dir_to_string(path: &Path, root: &Path, extensions: &HashSet<String>) -> Result<String> {
    let mut result = String::new();

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
                let file_content = read_file(entry.path(), root)?;
                result.push_str(&file_content);
            }
        }
    }

    Ok(result)
}
