//! xtask for managing BATMAN release artifacts.
//!
//! This crate provides utilities for building, packaging, and publishing
//! BATMAN release artifacts. It is invoked via `cargo xtask`.

use std::path::PathBuf;
use std::process::Command;

/// Builds the BATMAN runtime for all target platforms.
pub fn build_all_targets() -> Result<(), String> {
    let targets = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ];

    for target in targets {
        println!("Building for {target}...");
        let status = Command::new("cargo")
            .args(["build", "--release", "--bin", "batcave", "--target", target])
            .status()
            .map_err(|e| format!("failed to build for {target}: {e}"))?;

        if !status.success() {
            return Err(format!("build failed for {target}"));
        }
    }

    Ok(())
}

/// Packages the built artifacts for distribution.
pub fn package_artifacts(output_dir: &PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("failed to create output directory: {e}"))?;

    let targets = [
        ("aarch64-apple-darwin", "batcave-aarch64-apple-darwin"),
        ("x86_64-apple-darwin", "batcave-x86_64-apple-darwin"),
        ("x86_64-unknown-linux-gnu", "batcave-x86_64-unknown-linux-gnu"),
        ("x86_64-pc-windows-msvc", "batcave-x86_64-pc-windows-msvc.exe"),
    ];

    for (target, artifact_name) in targets {
        let source = PathBuf::from("target")
            .join(target)
            .join("release")
            .join(if target.contains("windows") {
                "batcave.exe"
            } else {
                "batcave"
            });

        let dest = output_dir.join(artifact_name);

        if source.exists() {
            std::fs::copy(&source, &dest)
                .map_err(|e| format!("failed to copy {source:?} to {dest:?}: {e}"))?;
            println!("Packaged {artifact_name}");
        } else {
            eprintln!("Warning: source artifact not found at {source:?}");
        }
    }

    Ok(())
}

/// Generates a checksum file for all artifacts.
pub fn generate_checksums(output_dir: &PathBuf) -> Result<(), String> {
    let mut checksums = String::new();

    for entry in std::fs::read_dir(output_dir)
        .map_err(|e| format!("failed to read output directory: {e}"))?
    {
        let entry = entry.map_err(|e| format!("failed to read entry: {e}"))?;
        let path = entry.path();

        if path.is_file() {
            let checksum = sha256_of_file(&path)?;
            checksums.push_str(&format!("{checksum}  {}\n", path.file_name().unwrap().to_string_lossy()));
        }
    }

    let checksums_path = output_dir.join("SHA256SUMS");
    std::fs::write(&checksums_path, checksums)
        .map_err(|e| format!("failed to write checksums file: {e}"))?;

    Ok(())
}

/// Computes the SHA-256 checksum of a file.
fn sha256_of_file(path: &PathBuf) -> Result<String, String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("failed to open file: {e}"))?;

    let mut hasher = sha2::Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|e| format!("failed to read file: {e}"))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: cargo xtask release <command>");
        eprintln!("Commands:");
        eprintln!("  build      Build for all targets");
        eprintln!("  package    Package artifacts");
        eprintln!("  checksum   Generate checksums");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "build" => {
            if let Err(e) = build_all_targets() {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        "package" => {
            let output_dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("dist"));
            if let Err(e) = package_artifacts(&output_dir) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        "checksum" => {
            let output_dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("dist"));
            if let Err(e) = generate_checksums(&output_dir) {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            std::process::exit(1);
        }
    }
}
