//! Reject contradictory ZIP names before trusting or extracting central-directory entries.
use std::io::{self, Read, Seek, SeekFrom};

/// `header_start` and `central_name` come from the same ZipArchive entry. The
/// reader must have an independent cursor over the same immutable archive bytes.
/// ZIP local names are bounded by a u16 length; no payload or extra field is read.
/// Sizes/CRC can legitimately differ with data descriptors, so they are not
/// compared here. Existing ZipArchive decoding and caller limits still apply.
pub(crate) fn require_matching_name<R: Read + Seek>(
    reader: &mut R,
    header_start: u64,
    central_name: &[u8],
) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "zip_header_name_mismatch");
    reader.seek(SeekFrom::Start(header_start))?;
    let mut header = [0u8; 30];
    reader.read_exact(&mut header)?;
    if &header[..4] != b"PK\x03\x04" {
        return Err(invalid());
    }
    let name_length = usize::from(u16::from_le_bytes([header[26], header[27]]));
    if name_length != central_name.len() {
        return Err(invalid());
    }
    let mut local_name = vec![0; name_length];
    reader.read_exact(&mut local_name)?;
    if local_name != central_name {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn matching_names_preserve_stored_deflated_unicode_and_zip64_entries() {
        for method in [
            zip::CompressionMethod::Stored,
            zip::CompressionMethod::Deflated,
        ] {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer
                .start_file(
                    "root/資料.txt",
                    zip::write::SimpleFileOptions::default()
                        .compression_method(method)
                        .large_file(true),
                )
                .unwrap();
            writer.write_all(b"original bytes").unwrap();
            let bytes = writer.finish().unwrap().into_inner();
            let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
            let mut entry = archive.by_index(0).unwrap();
            require_matching_name(
                &mut Cursor::new(&bytes),
                entry.header_start(),
                entry.name_raw(),
            )
            .unwrap();
            let mut restored = Vec::new();
            entry.read_to_end(&mut restored).unwrap();
            assert_eq!(restored, b"original bytes");
        }
    }

    #[test]
    fn matching_names_preserve_stream_written_data_descriptors() {
        let mut writer = zip::ZipWriter::new_stream(Vec::new());
        writer
            .start_file("result.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"streamed payload").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert_ne!(u16::from_le_bytes([bytes[6], bytes[7]]) & 8, 0);
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut entry = archive.by_index(0).unwrap();
        require_matching_name(
            &mut Cursor::new(&bytes),
            entry.header_start(),
            entry.name_raw(),
        )
        .unwrap();
        let mut restored = Vec::new();
        entry.read_to_end(&mut restored).unwrap();
        assert_eq!(restored, b"streamed payload");
    }

    #[test]
    fn header_check_rejects_signature_length_bytes_and_truncation_at_exact_offset() {
        let mut bytes = vec![0u8; 7];
        bytes.extend_from_slice(b"PK\x03\x04");
        bytes.resize(7 + 30, 0);
        bytes[7 + 26] = 4;
        bytes.extend_from_slice(b"safe");
        require_matching_name(&mut Cursor::new(&bytes), 7, b"safe").unwrap();
        for (index, replacement) in [(7, 0), (7 + 26, 3), (7 + 30, b'X')] {
            let mut changed = bytes.clone();
            changed[index] = replacement;
            assert!(require_matching_name(&mut Cursor::new(changed), 7, b"safe").is_err());
        }
        for end in [7, 7 + 29, bytes.len() - 1] {
            assert!(require_matching_name(&mut Cursor::new(&bytes[..end]), 7, b"safe").is_err());
        }
        assert!(require_matching_name(&mut Cursor::new(bytes), u64::MAX, b"safe").is_err());
    }
}
