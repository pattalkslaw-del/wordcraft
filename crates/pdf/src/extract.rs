//! Extract Unicode text from PDF bytes (for export verification).

use hayro_cmap::BfString;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

struct Extractor {
    /// Every glyph drawn, page furniture (headers, footers, watermarks) included.
    all: String,
    /// Glyphs outside artifact-marked content: the document's own text, not its furniture.
    content: String,
    /// Marked-content nesting, innermost last; `true` where the sequence is an artifact.
    marks: Vec<bool>,
}

impl Extractor {
    fn in_artifact(&self) -> bool {
        self.marks.iter().any(|artifact| *artifact)
    }
}

impl Device<'_> for Extractor {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &kurbo::BezPath, _: kurbo::Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(&mut self, g: &Glyph<'_>, _: kurbo::Affine, _: kurbo::Affine, _: &Paint<'_>, mode: &GlyphDrawMode) {
        if matches!(mode, GlyphDrawMode::Stroke(_)) {
            return;
        }
        let artifact = self.in_artifact();
        match g.as_unicode() {
            Some(BfString::Char(c)) => {
                self.all.push(c);
                if !artifact {
                    self.content.push(c);
                }
            }
            Some(BfString::String(s)) => {
                self.all.push_str(&s);
                if !artifact {
                    self.content.push_str(&s);
                }
            }
            None => {
                self.all.push('\u{FFFD}');
                if !artifact {
                    self.content.push('\u{FFFD}');
                }
            }
        }
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    // Headers, footers and watermarks are written inside an `Artifact` marked-content sequence.
    fn begin_marked_content(&mut self, tag: &[u8], _: Option<i32>) {
        self.marks.push(tag == b"Artifact");
    }
    fn end_marked_content(&mut self) {
        self.marks.pop();
    }
}

/// Per page, the text inside and outside artifact-marked content.
fn extract_pages(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    if bytes.len() < 8 || !bytes.starts_with(b"%PDF") {
        return Err("not a PDF".into());
    }
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("{e:?}"))?;
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        let mut ex = Extractor { all: String::new(), content: String::new(), marks: Vec::new() };
        interpret_page(page, &mut ctx, &mut ex);
        out.push((ex.all, ex.content));
    }
    Ok(out)
}

/// Unicode extracted from every page (empty strings for pages with no text), page furniture
/// included. [`has_extractable_text`] is what judges a PDF's text layer, leaving furniture out.
pub fn extract_page_text(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(extract_pages(bytes)?.into_iter().map(|(all, _)| all).collect())
}

/// Fewest characters of a page's own text (outside artifact page furniture) before the PDF counts
/// as carrying a real text layer. A page number or a short running head is far under it; a page of
/// body text is far over it.
const MIN_SUBSTANTIVE_CHARS: usize = 24;

/// True when the PDF carries substantive, searchable document text, not merely font resources,
/// images, or incidental text such as a scanner's page numbers. A scan with an extractable page
/// number on every page has no page with enough of its own text and so is not searchable.
pub fn has_extractable_text(bytes: &[u8]) -> bool {
    match extract_pages(bytes) {
        Ok(pages) => pages.iter().any(|(_, content)| content.chars().filter(|c| c.is_alphanumeric()).count() >= MIN_SUBSTANTIVE_CHARS),
        Err(_) => false,
    }
}
