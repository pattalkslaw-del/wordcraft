//! Extract Unicode text from PDF bytes (for export verification).

use hayro_cmap::BfString;
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;

struct Extractor(String);

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
        match g.as_unicode() {
            Some(BfString::Char(c)) => self.0.push(c),
            Some(BfString::String(s)) => self.0.push_str(&s),
            None => self.0.push('\u{FFFD}'),
        }
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: kurbo::Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// Unicode extracted from every page (empty strings for pages with no text).
pub fn extract_page_text(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.len() < 8 || !bytes.starts_with(b"%PDF") {
        return Err("not a PDF".into());
    }
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| format!("{e:?}"))?;
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(kurbo::Affine::IDENTITY, kurbo::Rect::new(0.0, 0.0, 1.0, 1.0), &cache, pdf.xref(), InterpreterSettings::default());
        let mut ex = Extractor(String::new());
        interpret_page(page, &mut ctx, &mut ex);
        out.push(ex.0);
    }
    Ok(out)
}

/// True when the PDF carries extractable document text (not merely font resources or images).
pub fn has_extractable_text(bytes: &[u8]) -> bool {
    match extract_page_text(bytes) {
        Ok(pages) => pages.iter().any(|p| p.chars().any(|c| c.is_alphanumeric())),
        Err(_) => false,
    }
}
