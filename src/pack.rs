//! Writes the zip file, then re-opens it to prove the EPUB rules were met.

use crate::validate::{MIMETYPE_CONTENT, MIMETYPE_FILE};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

// Rust note: `Box<dyn Error>` means "any kind of error". It lets one function
// return I/O errors, zip errors and our own text errors, and lets `?` convert
// each of them automatically.
type BoxError = Box<dyn Error>;

/// Build the EPUB at `output_path` from `files` (paths relative to `folder`).
///
/// The zip is first written to a temporary file and only renamed to its final
/// name once it has passed verification, so a failure never leaves a broken
/// .epub behind.
pub fn build(folder: &Path, files: &[String], output_path: &Path) -> Result<(), BoxError> {
    let temp_path = output_path.with_extension("epub.tmp");

    let result = write_zip(folder, files, &temp_path).and_then(|()| verify_zip(&temp_path, files));
    if let Err(error) = result {
        let _ = fs::remove_file(&temp_path); // best effort; ignore cleanup errors
        return Err(error);
    }

    fs::rename(&temp_path, output_path)?;
    Ok(())
}

fn write_zip(folder: &Path, files: &[String], zip_path: &Path) -> Result<(), BoxError> {
    let mut zip = ZipWriter::new(File::create(zip_path)?);

    // With default features off, every entry gets a fixed 1980-01-01 timestamp,
    // which makes the output byte-for-byte reproducible.
    let stored_options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated_options =
        SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    for path in files {
        if path == MIMETYPE_FILE {
            // Must be uncompressed ("stored"), and contain exactly these bytes.
            zip.start_file(path.as_str(), stored_options)?;
            zip.write_all(MIMETYPE_CONTENT.as_bytes())?;
        } else {
            zip.start_file(path.as_str(), deflated_options)?;
            let mut source_file = File::open(folder.join(path))?;
            io::copy(&mut source_file, &mut zip)?;
        }
    }

    let mut finished_file = zip.finish()?;
    finished_file.flush()?;
    finished_file.sync_all()?; // make sure it really reached the disk
    Ok(())
}

/// Re-open the finished zip and check the ZIP-level EPUB requirements.
fn verify_zip(zip_path: &Path, expected_names: &[String]) -> Result<(), BoxError> {
    let mut archive = ZipArchive::new(File::open(zip_path)?)?;

    if archive.len() != expected_names.len() {
        return Err(format!(
            "verification failed: archive has {} entries, expected {}",
            archive.len(),
            expected_names.len()
        )
        .into());
    }

    verify_mimetype_local_header(zip_path)?;

    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;

        if entry.name() != expected_names[index] {
            return Err(format!(
                "verification failed: entry {index} is `{}`, expected `{}`",
                entry.name(),
                expected_names[index]
            )
            .into());
        }

        let compression = entry.compression();
        if compression != CompressionMethod::Stored && compression != CompressionMethod::Deflated {
            return Err(format!(
                "verification failed: `{}` uses unsupported compression {compression:?}",
                entry.name()
            )
            .into());
        }
    }
    Ok(())
}

/// Size of the fixed part of a ZIP "local file header".
const LOCAL_HEADER_SIZE: usize = 30;

/// Check the very start of the zip file on disk.
///
/// We read the raw bytes ourselves instead of asking the `zip` library, so the
/// check does not depend on how the library reports things. This is also what
/// many tools do to recognise an EPUB: `PK`, then the name `mimetype` at byte
/// 30, then `application/epub+zip` at byte 38.
fn verify_mimetype_local_header(zip_path: &Path) -> Result<(), BoxError> {
    let bytes_needed = LOCAL_HEADER_SIZE + MIMETYPE_FILE.len() + MIMETYPE_CONTENT.len();
    let mut start_of_file = vec![0u8; bytes_needed];
    File::open(zip_path)?.read_exact(&mut start_of_file)?;

    check_mimetype_header_bytes(&start_of_file)
        .map_err(|problem| format!("verification failed: {problem}").into())
}

/// Checks the first bytes of a zip: the local file header of the first entry,
/// which must be an uncompressed `mimetype` with no extra field.
///
/// Local file header layout (numbers are little-endian):
///   bytes  0..4   signature "PK\x03\x04"
///   bytes  6..8   flags (bit 3 means "sizes come after the data")
///   bytes  8..10  compression method (0 = stored)
///   bytes 26..28  file name length
///   bytes 28..30  extra field length
///   bytes 30..    file name, then the extra field, then the file data
fn check_mimetype_header_bytes(bytes: &[u8]) -> Result<(), String> {
    let bytes_needed = LOCAL_HEADER_SIZE + MIMETYPE_FILE.len() + MIMETYPE_CONTENT.len();
    if bytes.len() < bytes_needed {
        return Err("the archive is too short to contain a `mimetype` entry".into());
    }

    let read_u16 = |offset: usize| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);

    if &bytes[0..4] != b"PK\x03\x04" {
        return Err("the file does not start with a ZIP local file header".into());
    }

    let flags = read_u16(6);
    if flags & (1 << 3) != 0 {
        return Err("`mimetype` uses a data descriptor (flag bit 3 is set)".into());
    }
    if read_u16(8) != 0 {
        return Err("`mimetype` is compressed".into());
    }

    let name_length = usize::from(read_u16(26));
    let name_end = LOCAL_HEADER_SIZE + MIMETYPE_FILE.len();
    let name_is_mimetype = name_length == MIMETYPE_FILE.len()
        && &bytes[LOCAL_HEADER_SIZE..name_end] == MIMETYPE_FILE.as_bytes();
    if !name_is_mimetype {
        return Err("`mimetype` is not the first entry".into());
    }

    let extra_field_length = read_u16(28);
    if extra_field_length != 0 {
        return Err(format!(
            "`mimetype` has a ZIP extra field ({extra_field_length} bytes)"
        ));
    }

    let content = &bytes[name_end..name_end + MIMETYPE_CONTENT.len()];
    if content != MIMETYPE_CONTENT.as_bytes() {
        return Err("`mimetype` content is wrong".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first 58 bytes of a correct EPUB zip.
    fn good_header() -> Vec<u8> {
        let mut bytes = vec![0u8; LOCAL_HEADER_SIZE];
        bytes[0..4].copy_from_slice(b"PK\x03\x04");
        bytes[26..28].copy_from_slice(&8u16.to_le_bytes()); // name length
        bytes.extend_from_slice(b"mimetype");
        bytes.extend_from_slice(b"application/epub+zip");
        bytes
    }

    /// Run the check on `good_header()` after changing it, and return the error.
    fn error_after(change: impl FnOnce(&mut Vec<u8>)) -> String {
        let mut bytes = good_header();
        change(&mut bytes);
        check_mimetype_header_bytes(&bytes).expect_err("the check should have failed")
    }

    #[test]
    fn a_correct_header_passes() {
        assert_eq!(check_mimetype_header_bytes(&good_header()), Ok(()));
    }

    #[test]
    fn a_compressed_mimetype_is_rejected() {
        let problem = error_after(|bytes| bytes[8] = 8); // 8 = deflate
        assert!(problem.contains("compressed"), "{problem}");
    }

    #[test]
    fn an_extra_field_is_rejected() {
        let problem = error_after(|bytes| bytes[28] = 4);
        assert!(problem.contains("extra field"), "{problem}");
    }

    #[test]
    fn a_data_descriptor_is_rejected() {
        let problem = error_after(|bytes| bytes[6] = 0b1000);
        assert!(problem.contains("data descriptor"), "{problem}");
    }

    #[test]
    fn a_different_first_entry_is_rejected() {
        let problem = error_after(|bytes| bytes[30] = b'X'); // "Ximetype"
        assert!(problem.contains("not the first entry"), "{problem}");
    }

    #[test]
    fn wrong_mimetype_content_is_rejected() {
        let problem = error_after(|bytes| bytes[38] = b'X');
        assert!(problem.contains("content is wrong"), "{problem}");
    }

    #[test]
    fn a_file_that_is_not_a_zip_is_rejected() {
        let problem = error_after(|bytes| bytes[0] = b'X');
        assert!(problem.contains("local file header"), "{problem}");
    }

    #[test]
    fn a_truncated_file_is_rejected() {
        let truncated = &good_header()[..20];
        assert!(check_mimetype_header_bytes(truncated).is_err());
    }
}
