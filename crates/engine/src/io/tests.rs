use std::io::{Cursor, Read, Write};

use super::*;

#[test]
fn oversized_pdfs_are_rejected_after_only_the_header_read() {
    for (name, prefix) in [
        ("large.pdf", b"%PDF-1.7\n".as_slice()),
        ("renamed.txt", b"%PDF-1.7\n".as_slice()),
        ("no-extension", b" \t\r\n%PDF-1.7\n".as_slice()),
        ("large.PDF", b"invalid PDF".as_slice()),
    ] {
        let mut input = vec![0; 2048];
        input[..prefix.len()].copy_from_slice(prefix);
        let mut reader = Cursor::new(input);
        let error = read_document_stream(&mut reader, wordcraft_pdf::import::MAX_BYTES as u64 + 1, name).unwrap_err();
        assert!(error.contains("64 MiB import limit"), "{name}: {error}");
        assert_eq!(reader.position(), 1024, "{name}: no content beyond the sniffing prefix is buffered");
    }
}

#[test]
fn document_reads_keep_the_prefix_and_non_pdf_size_ceiling() {
    let bytes = b"%PDF-1.7\nsmall synthetic input";
    assert_eq!(read_document_stream(bytes.as_slice(), wordcraft_pdf::import::MAX_BYTES as u64, "boundary.pdf").unwrap(), bytes);
    let text = vec![b'x'; 2048];
    assert_eq!(read_document_stream(text.as_slice(), wordcraft_pdf::import::MAX_BYTES as u64 + 1, "large.txt").unwrap(), text);
    let mut reader = Cursor::new(b"plain text");
    assert!(read_document_stream(&mut reader, MAX_DOCUMENT + 1, "huge.txt").unwrap_err().contains("larger than 2 GB"));
    assert_eq!(reader.position(), 0, "a file above the general ceiling is not read");
}

#[test]
fn pdf_reads_stay_bounded_when_the_file_grows_after_metadata() {
    struct Counted<R> {
        inner: R,
        bytes: u64,
    }
    impl<R: Read> Read for Counted<R> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(out)?;
            self.bytes += n as u64;
            Ok(n)
        }
    }
    // An endless synthetic stream models a file growing after its metadata was checked.
    let mut reader = Counted { inner: b"%PDF-1.7\n".as_slice().chain(std::io::repeat(0)), bytes: 0 };
    let error = read_document_stream(&mut reader, 0, "grown.bin").unwrap_err();
    assert!(error.contains("64 MiB import limit"), "{error}");
    assert_eq!(reader.bytes, wordcraft_pdf::import::MAX_BYTES as u64 + 1);
}

#[test]
fn document_paths_enforce_the_pdf_limit_for_misleading_extensions() {
    for ext in ["pdf", "txt"] {
        let path = std::env::temp_dir().join(format!("wordcraft-pdf-bound-{}.{ext}", std::process::id()));
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b" \n%PDF-1.7\n").unwrap();
        file.set_len(wordcraft_pdf::import::MAX_BYTES as u64 + 1).unwrap();
        drop(file);
        let error = read_document(&path).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(error.contains("64 MiB import limit"), "{error}");
    }
}

#[test]
fn a_short_document_is_not_read_again_after_reaching_eof() {
    struct EndsThenGrows {
        prefix: Cursor<&'static [u8]>,
        reached_eof: bool,
    }
    impl Read for EndsThenGrows {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            assert!(!self.reached_eof, "do not resume after EOF with a possibly incomplete PDF header");
            let n = self.prefix.read(out)?;
            self.reached_eof = n == 0;
            Ok(n)
        }
    }
    let mut reader = EndsThenGrows { prefix: Cursor::new(b"%PDF"), reached_eof: false };
    assert_eq!(read_document_stream(&mut reader, 4, "growing.bin").unwrap(), b"%PDF");
    assert!(reader.reached_eof);
}
