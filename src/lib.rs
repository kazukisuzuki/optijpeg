mod mozjpeg;

use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Optimized,
    Unchanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Optimization {
    pub original_size: u64,
    pub final_size: u64,
    pub status: Status,
}

impl Optimization {
    #[must_use]
    pub fn bytes_saved(self) -> u64 {
        self.original_size.saturating_sub(self.final_size)
    }
}

/// Losslessly optimizes one JPEG in place.
///
/// The image's quantized DCT coefficients are copied without decoding pixels,
/// so image quality is unchanged. An optimized progressive representation is
/// generated. The original is retained only when it has no metadata and is no
/// larger than that result.
pub fn optimize_file(path: &Path) -> Result<Optimization> {
    let original = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if original.is_empty() {
        bail!("input is empty");
    }

    let candidate = mozjpeg::transcode(&original).context("failed to create progressive JPEG")?;

    let original_size = original.len() as u64;

    if !should_replace(&original, &candidate) {
        return Ok(Optimization {
            original_size,
            final_size: original_size,
            status: Status::Unchanged,
        });
    }

    replace_file(path, &candidate)?;
    Ok(Optimization {
        original_size,
        final_size: candidate.len() as u64,
        status: Status::Optimized,
    })
}

fn should_replace(original: &[u8], candidate: &[u8]) -> bool {
    candidate.len() < original.len() || contains_metadata(original)
}

fn contains_metadata(jpeg: &[u8]) -> bool {
    let mut position = 0;

    while position + 1 < jpeg.len() {
        if jpeg[position] != 0xff {
            position += 1;
            continue;
        }

        let mut marker_position = position + 1;
        while marker_position < jpeg.len() && jpeg[marker_position] == 0xff {
            marker_position += 1;
        }
        let Some(&marker) = jpeg.get(marker_position) else {
            return false;
        };

        if marker == 0x00 {
            position = marker_position + 1;
            continue;
        }
        if (0xe0..=0xef).contains(&marker) || marker == 0xfe {
            return true;
        }

        let standalone = marker == 0x01 || (0xd0..=0xd9).contains(&marker);
        if standalone {
            position = marker_position + 1;
            continue;
        }

        let Some(length_bytes) = jpeg.get(marker_position + 1..marker_position + 3) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 {
            return false;
        }
        position = marker_position + 1 + length;
    }

    false
}

fn replace_file(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let permissions = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?
        .permissions();

    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create a temporary file in {}", parent.display()))?;
    temporary
        .write_all(contents)
        .context("failed to write the optimized JPEG")?;
    temporary
        .as_file()
        .sync_all()
        .context("failed to flush the optimized JPEG")?;
    temporary
        .as_file()
        .set_permissions(permissions)
        .context("failed to preserve file permissions")?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::should_replace;

    #[test]
    fn replaces_smaller_files_or_files_with_metadata() {
        let metadata_free = [0; 8];
        let with_metadata = [0xff, 0xd8, 0xff, 0xe1, 0, 2, 0xff, 0xd9];

        assert!(should_replace(&metadata_free, &[0; 7]));
        assert!(!should_replace(&metadata_free, &[0; 8]));
        assert!(should_replace(&with_metadata, &[0; 9]));
    }
}
