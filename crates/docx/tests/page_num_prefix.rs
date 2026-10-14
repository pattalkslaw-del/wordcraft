//! Section page-number prefix (`page_num_prefix`) is stored in a namespaced `w:extLst` extension,
//! not the invalid legacy `w:pgNumPrefix` element.

use std::io::{Read, Write};

use wordcraft_doc::section::SectionProps;
use wordcraft_doc::{Document, InlineObject, Paragraph, PartKind, para_block};
use wordcraft_docx::{PAGE_NUM_PREFIX_EXT, read, write};

fn docx(body: &str) -> Vec<u8> {
    let root = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let doc = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [("_rels/.rels", root.as_bytes()), ("word/document.xml", doc.as_bytes())] {
        zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap().into_inner()
}

fn sect(doc: &Document) -> SectionProps {
    doc.sections().into_iter().next().map(|(_, s)| s.clone()).expect("a section")
}

fn doc_with_prefix(prefix: &str) -> Document {
    let mut d = Document::new();
    let field = InlineObject::Field { instr: "PAGE".into(), result: "1".into(), locked: false, code: false };
    let mut fp = Paragraph::new();
    fp.insert_text(0, prefix, &Default::default()).unwrap();
    fp.insert_object(prefix.len(), field, &Default::default()).unwrap();
    let id = d.add_part(PartKind::Footer, vec![para_block(fp)]);
    d.last_section.footers.default = Some(id);
    d.last_section.page_num_start = Some(1);
    d.last_section.page_num_prefix = Some(prefix.to_string());
    d
}

#[test]
fn page_num_prefix_writes_an_extension_not_pg_num_prefix() {
    let bytes = write(&doc_with_prefix("A")).expect("write");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut xml = String::new();
    zip.by_name("word/document.xml").unwrap().read_to_string(&mut xml).unwrap();
    assert!(!xml.contains("w:pgNumPrefix"), "must not emit invalid OOXML: {xml}");
    assert!(xml.contains(PAGE_NUM_PREFIX_EXT), "{xml}");
    assert!(xml.contains("wc:pgNumPrefix") && xml.contains("wc:val=\"A\""), "{xml}");
}

#[test]
fn page_num_prefix_round_trips_through_the_extension() {
    let doc = doc_with_prefix("Add. ");
    let back = read(&write(&doc).expect("write")).expect("read");
    assert_eq!(sect(&back).page_num_prefix.as_deref(), Some("Add. "));
}

#[test]
fn legacy_pg_num_prefix_still_reads() {
    let body = r#"<w:p><w:pPr><w:sectPr><w:pgNumType w:start="1"/><w:pgNumPrefix w:val="A"/></w:sectPr></w:pPr></w:p>"#;
    let d = read(&docx(body)).expect("read legacy prefix");
    assert_eq!(sect(&d).page_num_prefix.as_deref(), Some("A"));
}
