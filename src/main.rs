use std::fs::File;
use std::io::BufWriter;
use std::io::Error;
use std::io::Write;
use std::path::Path;

use walkdir::WalkDir;

fn main() -> Result<(), Error> {
    let root = std::env::current_dir()?;
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout);
    read_dir(&root, &root, &mut writer)
}

fn read_dir(path: &Path, root: &Path, writer: &mut impl Write) -> Result<(), std::io::Error> {
    for entry in WalkDir::new(path).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            let Some(ext) = entry.path().extension() else { continue };
            let Some(ext) = ext.to_str() else { continue };
            if ext == "rs" || ext == "md" || entry.file_name() == "Cargo.toml" {
                read_file(entry.path(), root, ext, writer)?;
            }
        }
    }
    Ok(())
}

fn read_file(
    path: &Path,
    root: &Path,
    ext: &str,
    writer: &mut impl Write,
) -> Result<(), std::io::Error> {
    let mut file = File::open(path)?;
    writeln!(writer, "```{ext}")?;
    let path = path
        .strip_prefix(root)
        .expect("Failed to strip prefix")
        .display();
    writeln!(writer, "// {path}")?;
    std::io::copy(&mut file, writer)?;

    writeln!(writer, "```")?;
    Ok(())
}
