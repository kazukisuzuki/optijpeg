use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Parser;
use lexical_sort::{PathSort, natural_lexical_cmp};
use optijpeg::{Status, optimize_file};
use rayon::prelude::*;
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Optimize JPEG files losslessly in place",
    after_help = "Direct file arguments may use any extension. Recursive searches process .jpg and .jpeg files."
)]
struct Cli {
    /// JPEG files to optimize in place
    #[arg(
        value_name = "JPEG",
        required_unless_present = "recursive",
        conflicts_with = "recursive"
    )]
    files: Vec<PathBuf>,

    /// Recursively optimize JPEG files below one or more paths
    #[arg(
        short = 'r',
        long = "recursive",
        value_name = "PATH",
        num_args = 1..
    )]
    recursive: Vec<PathBuf>,
}

#[derive(Default)]
struct Summary {
    optimized: usize,
    unchanged: usize,
    failed: usize,
    bytes_saved: u64,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (files, discovery_errors) = discover_files(&cli.files, &cli.recursive);
    let summary = Summary {
        failed: discovery_errors.len(),
        ..Summary::default()
    };

    for error in discovery_errors {
        eprintln!("[ERROR] {error:#}");
    }

    let results: Vec<_> = files
        .into_par_iter()
        .map(|path| {
            let result = optimize_file(&path);
            (path, result)
        })
        .collect();

    let mut summary = summary;
    for (path, result) in results {
        match result {
            Ok(result) => match result.status {
                Status::Optimized => {
                    summary.optimized += 1;
                    summary.bytes_saved += result.bytes_saved();
                    if result.final_size <= result.original_size {
                        let percent =
                            result.bytes_saved() as f64 * 100.0 / result.original_size as f64;
                        println!(
                            "[OK] {}: {} -> {} bytes ({percent:.2}% saved, progressive)",
                            path.display(),
                            result.original_size,
                            result.final_size,
                        );
                    } else {
                        println!(
                            "[OK] {}: {} -> {} bytes ({} bytes larger, metadata removed, progressive)",
                            path.display(),
                            result.original_size,
                            result.final_size,
                            result.final_size - result.original_size,
                        );
                    }
                }
                Status::Unchanged => {
                    summary.unchanged += 1;
                    println!("[SKIP] {}: already optimal", path.display());
                }
            },
            Err(error) => {
                summary.failed += 1;
                eprintln!("[ERROR] {}: {error:#}", path.display());
            }
        }
    }

    println!(
        "Done: {} optimized, {} unchanged, {} failed, {} bytes saved",
        summary.optimized, summary.unchanged, summary.failed, summary.bytes_saved
    );

    if summary.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn discover_files(direct: &[PathBuf], recursive: &[PathBuf]) -> (Vec<PathBuf>, Vec<anyhow::Error>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();

    for path in direct {
        match validate_direct_file(path) {
            Ok(path) => push_unique(path, &mut files, &mut seen),
            Err(error) => errors.push(error),
        }
    }

    for root in recursive {
        if !root.is_dir() {
            errors.push(anyhow::anyhow!(
                "recursive path is not a directory: {}",
                root.display()
            ));
            continue;
        }

        for entry in WalkDir::new(root).follow_links(false).sort_by_file_name() {
            match entry {
                Ok(entry) if entry.file_type().is_file() && is_jpeg_path(entry.path()) => {
                    push_unique(entry.into_path(), &mut files, &mut seen);
                }
                Ok(_) => {}
                Err(error) => errors.push(error.into()),
            }
        }
    }

    files.path_sort(natural_lexical_cmp);
    (files, errors)
}

fn validate_direct_file(path: &Path) -> Result<PathBuf> {
    if path.is_dir() {
        bail!(
            "{} is a directory; use -r {} for recursive processing",
            path.display(),
            path.display()
        );
    }
    if !path.is_file() {
        bail!("file does not exist: {}", path.display());
    }
    Ok(path.to_path_buf())
}

fn push_unique(path: PathBuf, files: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let identity = path.canonicalize().unwrap_or_else(|_| path.clone());
    if seen.insert(identity) {
        files.push(path);
    }
}

fn is_jpeg_path(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg")
    })
}
