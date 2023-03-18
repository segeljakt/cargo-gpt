use std::fs::File;
use std::io::BufWriter;
use std::io::Error;
use std::io::Write;
use std::path::Path;

use ignore::WalkBuilder;

fn main() -> Result<(), Error> {
    let root = std::env::current_dir()?;
    let stdout = std::io::stdout();
    let mut writer = BufWriter::new(stdout);
    read_dir(&root, &root, &mut writer)
}

fn read_dir(path: &Path, root: &Path, writer: &mut impl Write) -> Result<(), std::io::Error> {
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
            .expect("Failed to get file type")
            .is_file()
        {
            let Some(ext) = entry.path().extension() else { continue };
            let Some(ext) = ext.to_str() else { continue };
            if ext == "rs" || ext == "md" || entry.file_name() == "Cargo.toml" {
                read_file(entry.path(), root, writer)?;
            }
        }
    }
    Ok(())
}

fn read_file(path: &Path, root: &Path, writer: &mut impl Write) -> Result<(), std::io::Error> {
    let mut file = File::open(path)?;
    let path = path
        .strip_prefix(root)
        .expect("Failed to strip prefix")
        .display();
    writeln!(writer, "// {path}")?;
    std::io::copy(&mut file, writer)?;
    Ok(())
}
