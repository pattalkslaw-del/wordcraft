//! Extract Unicode text from PDF bytes (for export verification).

use hayro_cmap::BfString;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

/// What one page yielded: its text, the text outside page furniture, and whether it drew a
/// picture of its own.
struct PageText {
    /// Every glyph drawn, page furniture (headers, footers, watermarks) included.
    all: String,
    /// Glyphs outside artifact-marked content: the document's own text, not its furniture.
    content: String,
    /// A picture outside artifact-marked content was drawn: a scan, photograph or figure.
    image: bool,
}

struct Extractor {
    all: String,
    content: String,
    image: bool,
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
    fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {
        // Page furniture can carry a logo or watermark image; that is not document content.
        if !self.in_artifact() {
            self.image = true;
        }
    }
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

/// Per page, the text inside and outside artifact-marked content, and whether the page drew a
/// picture of its own.
fn extract_pages(bytes: &[u8]) -> Result<Vec<PageText>, String> {
    if bytes.len() < 8 || !bytes.starts_with(b"%PDF") {
        return Err("not a PDF".into());
    }
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("{e:?}"))?;
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        let mut ex = Extractor { all: String::new(), content: String::new(), image: false, marks: Vec::new() };
        interpret_page(page, &mut ctx, &mut ex);
        out.push(PageText { all: ex.all, content: ex.content, image: ex.image });
    }
    Ok(out)
}

/// Unicode extracted from every page (empty strings for pages with no text), page furniture
/// included. [`has_extractable_text`] is what judges a PDF's text layer, leaving furniture out.
pub fn extract_page_text(bytes: &[u8]) -> Result<Vec<String>, String> {
    Ok(extract_pages(bytes)?.into_iter().map(|p| p.all).collect())
}

/// True when the PDF carries searchable document text on every page that has content.
///
/// A page counts as carrying document text when it has letters of its own outside artifact-marked
/// furniture. A lone page number is numeric and stays furniture, whether or not the producer
/// tagged it; a word — however short — is document text. There is no minimum document length, so
/// a fully extractable one-line document is searchable.
///
/// Coverage is what matters, not a single page: every page that draws a picture of its own (a
/// scan, photograph or figure, outside page furniture) must also carry such text, so a searchable
/// cover followed by scanned body pages is **not** searchable, while blank and furniture-only
/// pages are allowed.
pub fn has_extractable_text(bytes: &[u8]) -> bool {
    match extract_pages(bytes) {
        Ok(pages) => {
            let any_text = pages.iter().any(|p| has_substantive_text(&p.content));
            let every_content_page_is_text = pages.iter().all(|p| !p.image || has_substantive_text(&p.content));
            any_text && every_content_page_is_text
        }
        Err(_) => false,
    }
}

/// Does the page's own text (outside page furniture) contain letters? A page number or a numeric
/// stamp is incidental furniture; a word is the document's text.
fn has_substantive_text(content: &str) -> bool {
    content.chars().any(char::is_alphabetic)
}
