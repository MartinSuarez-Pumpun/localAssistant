/**
 * render_pdf.rs — Generador PDF 100% Rust usando printpdf.
 *
 * No tan elaborado como el DOCX (fuentes built-in Helvetica/Times, sin Georgia),
 * pero corporativo y legible: portada simple, header+footer en cada página,
 * parsing de markdown compartido con render_docx (párrafos, títulos, listas,
 * tablas simplificadas, separadores), paginación manual.
 */

use printpdf::{
    BuiltinFont, Color, IndirectFontRef, Line, Mm, PdfDocument, PdfDocumentReference,
    PdfLayerIndex, PdfLayerReference, PdfPageIndex, Point, Rgb,
};

use crate::routes::render_docx::{
    C_BLACK, C_BLUE, C_GREY, C_LIGHT, C_NAVY, C_ORANGE, C_TBLROW,
};

// ── Paleta (convertir hex a Rgb) ──────────────────────────────────────────────

fn hex_rgb(hex: &str) -> Color {
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    Color::Rgb(Rgb::new(r, g, b, None))
}

// ── Geometría de página (A4) ──────────────────────────────────────────────────

const PAGE_W_MM: f32 = 210.0;
const PAGE_H_MM: f32 = 297.0;
const MARGIN_MM: f32 = 20.0;
const HEADER_Y_MM: f32 = PAGE_H_MM - 12.0;
const FOOTER_Y_MM: f32 = 10.0;
const CONTENT_TOP_MM: f32 = PAGE_H_MM - MARGIN_MM - 5.0;
const CONTENT_BOTTOM_MM: f32 = MARGIN_MM + 5.0;
const CONTENT_W_MM: f32 = PAGE_W_MM - 2.0 * MARGIN_MM;

// ── Fuentes ───────────────────────────────────────────────────────────────────

struct Fonts {
    regular: IndirectFontRef,
    bold: IndirectFontRef,
    italic: IndirectFontRef,
    bold_italic: IndirectFontRef,
    title: IndirectFontRef,       // Times-Bold para títulos
    title_roman: IndirectFontRef, // Times-Roman para cuerpo "serif" opcional
}

fn load_fonts(doc: &PdfDocumentReference) -> Result<Fonts, String> {
    Ok(Fonts {
        regular:      doc.add_builtin_font(BuiltinFont::Helvetica).map_err(|e| e.to_string())?,
        bold:         doc.add_builtin_font(BuiltinFont::HelveticaBold).map_err(|e| e.to_string())?,
        italic:       doc.add_builtin_font(BuiltinFont::HelveticaOblique).map_err(|e| e.to_string())?,
        bold_italic:  doc.add_builtin_font(BuiltinFont::HelveticaBoldOblique).map_err(|e| e.to_string())?,
        title:        doc.add_builtin_font(BuiltinFont::TimesBold).map_err(|e| e.to_string())?,
        title_roman:  doc.add_builtin_font(BuiltinFont::TimesRoman).map_err(|e| e.to_string())?,
    })
}

// ── Estado del renderizador ───────────────────────────────────────────────────

struct Renderer<'a> {
    doc: &'a PdfDocumentReference,
    fonts: &'a Fonts,
    label: String,
    // página actual
    layer: PdfLayerReference,
    pages: Vec<(PdfPageIndex, PdfLayerIndex)>,
    y: f32, // posición vertical actual (mm, desde arriba)
}

impl<'a> Renderer<'a> {
    fn new(
        doc: &'a PdfDocumentReference,
        fonts: &'a Fonts,
        layer: PdfLayerReference,
        label: &str,
        page1: PdfPageIndex,
        layer1: PdfLayerIndex,
    ) -> Self {
        Self {
            doc, fonts, label: label.to_string(),
            layer,
            pages: vec![(page1, layer1)],
            y: CONTENT_TOP_MM,
        }
    }

    fn page_num(&self) -> usize { self.pages.len() }

    fn new_page(&mut self) {
        let (page_idx, layer_idx) = self.doc.add_page(Mm(PAGE_W_MM), Mm(PAGE_H_MM), "body");
        self.layer = self.doc.get_page(page_idx).get_layer(layer_idx);
        self.pages.push((page_idx, layer_idx));
        self.y = CONTENT_TOP_MM;
    }

    fn ensure_space(&mut self, needed_mm: f32) {
        if self.y - needed_mm < CONTENT_BOTTOM_MM {
            self.new_page();
        }
    }

    fn draw_header(&self) {
        // "OLIV4600 · {label}" en navy bold + grey; con línea inferior gris.
        self.layer.set_fill_color(hex_rgb(C_NAVY));
        self.layer.use_text("OLIV4600", 9.0, Mm(MARGIN_MM), Mm(HEADER_Y_MM), &self.fonts.bold);
        self.layer.set_fill_color(hex_rgb(C_GREY));
        self.layer.use_text(
            format!(" · {}", self.label),
            9.0,
            Mm(MARGIN_MM + text_width_mm("OLIV4600", 9.0, true)),
            Mm(HEADER_Y_MM),
            &self.fonts.regular,
        );
        // Línea gris
        draw_line(&self.layer, MARGIN_MM, HEADER_Y_MM - 2.0,
                  PAGE_W_MM - MARGIN_MM, HEADER_Y_MM - 2.0,
                  hex_rgb("CCCCCC"), 0.3);
    }

    fn draw_footer(&self, total_pages: usize) {
        draw_line(&self.layer, MARGIN_MM, FOOTER_Y_MM + 5.0,
                  PAGE_W_MM - MARGIN_MM, FOOTER_Y_MM + 5.0,
                  hex_rgb("CCCCCC"), 0.3);
        self.layer.set_fill_color(hex_rgb(C_GREY));
        self.layer.use_text(
            "OLIV4600 · Sovereign Intelligence · 100% Offline",
            8.0, Mm(MARGIN_MM), Mm(FOOTER_Y_MM), &self.fonts.regular,
        );
        let pg = format!("Página {} de {}", self.page_num(), total_pages);
        let w = text_width_mm(&pg, 8.0, false);
        self.layer.use_text(
            pg, 8.0,
            Mm(PAGE_W_MM - MARGIN_MM - w),
            Mm(FOOTER_Y_MM),
            &self.fonts.regular,
        );
    }
}

// Aproximación del ancho de texto para Helvetica built-in.
// Usamos un em ligeramente conservador (más ancho de lo real) para que el
// presupuesto de ancho de línea NUNCA subestime el ancho real: es mejor
// wrap-ear antes y dejar margen derecho de sobra que dejar que una línea
// se salga del área útil.
fn text_width_mm(s: &str, size_pt: f32, bold: bool) -> f32 {
    // Helvetica real promedia ~0.52 em regular / ~0.56 em bold, pero
    // cadenas con muchas mayúsculas anchas (M, W, acrónimos) superan eso.
    // Usamos 0.58 / 0.63 para asegurar que el presupuesto cubra el peor caso
    // realista y prevenir desbordes por margen derecho.
    let em = if bold { 0.72 } else { 0.65 };
    let char_mm = size_pt * em * 0.3528; // 1pt ≈ 0.3528mm
    s.chars().count() as f32 * char_mm
}

// Convierte pt → mm para spacing vertical.
fn pt_to_mm(pt: f32) -> f32 { pt * 0.3528 }

fn draw_line(layer: &PdfLayerReference, x1: f32, y1: f32, x2: f32, y2: f32, color: Color, thickness: f32) {
    layer.set_outline_color(color);
    layer.set_outline_thickness(thickness);
    let pts = vec![
        (Point::new(Mm(x1), Mm(y1)), false),
        (Point::new(Mm(x2), Mm(y2)), false),
    ];
    let line = Line { points: pts, is_closed: false };
    layer.add_line(line);
}

fn draw_filled_rect(layer: &PdfLayerReference, x: f32, y: f32, w: f32, h: f32, color: Color) {
    use printpdf::Rect;
    use printpdf::path::PaintMode;
    layer.set_fill_color(color);
    let rect = Rect::new(Mm(x), Mm(y), Mm(x + w), Mm(y + h))
        .with_mode(PaintMode::Fill);
    layer.add_rect(rect);
}

// ── Inline markdown (***, **, *) → segmentos con formato ──────────────────────

#[derive(Clone)]
struct Segment {
    text: String,
    bold: bool,
    italic: bool,
}

fn parse_inline(text: &str) -> Vec<Segment> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    let flush = |buf: &mut String, out: &mut Vec<Segment>, bold: bool, italic: bool| {
        if !buf.is_empty() {
            out.push(Segment { text: std::mem::take(buf), bold, italic });
        }
    };

    while i < bytes.len() {
        if bytes[i..].starts_with(b"***") {
            if let Some(end) = text[i + 3..].find("***") {
                flush(&mut buf, &mut out, false, false);
                out.push(Segment { text: text[i + 3..i + 3 + end].to_string(), bold: true, italic: true });
                i += 3 + end + 3;
                continue;
            }
        }
        if bytes[i..].starts_with(b"**") {
            if let Some(end) = text[i + 2..].find("**") {
                flush(&mut buf, &mut out, false, false);
                out.push(Segment { text: text[i + 2..i + 2 + end].to_string(), bold: true, italic: false });
                i += 2 + end + 2;
                continue;
            }
        }
        if bytes[i] == b'*' {
            if let Some(end) = text[i + 1..].find('*') {
                flush(&mut buf, &mut out, false, false);
                out.push(Segment { text: text[i + 1..i + 1 + end].to_string(), bold: false, italic: true });
                i += 1 + end + 1;
                continue;
            }
        }
        let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        buf.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    flush(&mut buf, &mut out, false, false);
    out
}

fn pick_font<'a>(fonts: &'a Fonts, bold: bool, italic: bool) -> &'a IndirectFontRef {
    match (bold, italic) {
        (true, true) => &fonts.bold_italic,
        (true, false) => &fonts.bold,
        (false, true) => &fonts.italic,
        (false, false) => &fonts.regular,
    }
}

// ── Word-wrap para segmentos (por palabras) ───────────────────────────────────

#[derive(Clone)]
struct Word {
    text: String,
    bold: bool,
    italic: bool,
    width_mm: f32,
    trailing_space: bool,
}

fn segments_to_words(segs: &[Segment], size_pt: f32) -> Vec<Word> {
    let mut words = Vec::new();
    for seg in segs {
        let mut chunks = seg.text.split(' ').peekable();
        while let Some(chunk) = chunks.next() {
            let trailing = chunks.peek().is_some();
            if chunk.is_empty() && !trailing { continue; }
            let w = text_width_mm(chunk, size_pt, seg.bold);
            words.push(Word {
                text: chunk.to_string(),
                bold: seg.bold,
                italic: seg.italic,
                width_mm: w,
                trailing_space: trailing,
            });
        }
    }
    words
}

/// Parte una palabra demasiado ancha en trozos (por caracteres) que quepan en
/// `max_width_mm`. Evita que un único token (URL larga, cadena sin espacios)
/// se salga del margen derecho. Preserva el estilo bold/italic de la palabra.
fn break_oversized_word(w: &Word, max_width_mm: f32, size_pt: f32) -> Vec<Word> {
    if w.width_mm <= max_width_mm || w.text.is_empty() {
        return vec![w.clone()];
    }
    let mut out: Vec<Word> = Vec::new();
    let mut buf = String::new();
    let mut buf_w = 0.0f32;
    for ch in w.text.chars() {
        let ch_s = ch.to_string();
        let ch_w = text_width_mm(&ch_s, size_pt, w.bold);
        if !buf.is_empty() && buf_w + ch_w > max_width_mm {
            out.push(Word {
                text: std::mem::take(&mut buf),
                bold: w.bold,
                italic: w.italic,
                width_mm: buf_w,
                trailing_space: false,
            });
            buf_w = 0.0;
        }
        buf.push(ch);
        buf_w += ch_w;
    }
    if !buf.is_empty() {
        out.push(Word {
            text: buf,
            bold: w.bold,
            italic: w.italic,
            width_mm: buf_w,
            trailing_space: w.trailing_space,
        });
    }
    // Si la palabra era 1 char más ancho que la línea (no debería), al menos
    // devolvemos algo. El último trozo hereda trailing_space original.
    if out.is_empty() { return vec![w.clone()]; }
    out
}

fn wrap_words(words: &[Word], max_width_mm: f32, space_mm: f32) -> Vec<Vec<Word>> {
    // Pre-paso: romper cualquier palabra individual más ancha que el área útil.
    // Deducimos el `size_pt` aproximando con la primera palabra no vacía; para
    // bloques mixtos (heading + cuerpo) esta función solo se llama por bloque
    // homogéneo, así que basta.
    let size_pt_est: f32 = {
        // width_mm ≈ n * size_pt * em * 0.3528  →  size_pt ≈ width_mm/(n*em*0.3528)
        // Con em 0.55 y size típicos (9-15pt) el estimador cae razonable.
        let probe = words.iter().find(|w| !w.text.is_empty() && w.width_mm > 0.0);
        match probe {
            Some(w) => {
                let n = w.text.chars().count().max(1) as f32;
                let em = if w.bold { 0.60 } else { 0.55 };
                (w.width_mm / (n * em * 0.3528)).clamp(6.0, 48.0)
            }
            None => 10.0,
        }
    };
    let expanded: Vec<Word> = words.iter()
        .flat_map(|w| break_oversized_word(w, max_width_mm, size_pt_est))
        .collect();

    let mut lines: Vec<Vec<Word>> = vec![];
    let mut current: Vec<Word> = vec![];
    let mut current_w = 0.0f32;

    for w in &expanded {
        let gap = if current.is_empty() { 0.0 } else { space_mm };
        if current_w + gap + w.width_mm > max_width_mm && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
        }
        let gap2 = if current.is_empty() { 0.0 } else { space_mm };
        current_w += gap2 + w.width_mm;
        current.push(w.clone());
    }
    if !current.is_empty() { lines.push(current); }
    lines
}

// ── Renderizado de bloques ────────────────────────────────────────────────────

impl<'a> Renderer<'a> {
    /// Renderiza un párrafo con inline markdown, auto-wrap + paginación.
    fn render_inline(&mut self, text: &str, size_pt: f32, color: Color, base_bold: bool) {
        let segs = parse_inline(text);
        // Aplicar base_bold a todos.
        let segs: Vec<Segment> = if base_bold {
            segs.into_iter().map(|mut s| { s.bold = true; s }).collect()
        } else { segs };

        let words = segments_to_words(&segs, size_pt);
        let space_mm = text_width_mm(" ", size_pt, false);
        let lines = wrap_words(&words, CONTENT_W_MM, space_mm);
        let line_height_mm = pt_to_mm(size_pt) * 1.35;

        for line in &lines {
            self.ensure_space(line_height_mm);
            let baseline_y = self.y - pt_to_mm(size_pt);
            let mut x = MARGIN_MM;
            for (i, w) in line.iter().enumerate() {
                if i > 0 { x += space_mm; }
                self.layer.set_fill_color(color.clone());
                self.layer.use_text(
                    w.text.clone(), size_pt, Mm(x), Mm(baseline_y),
                    pick_font(self.fonts, w.bold, w.italic),
                );
                x += w.width_mm;
            }
            self.y -= line_height_mm;
        }
    }

    fn render_bullet(&mut self, text: &str) {
        let size_pt = 10.0;
        let line_height_mm = pt_to_mm(size_pt) * 1.35;
        let indent_mm = 6.0;
        let segs = parse_inline(text);
        let words = segments_to_words(&segs, size_pt);
        let space_mm = text_width_mm(" ", size_pt, false);
        let avail = CONTENT_W_MM - indent_mm;
        let lines = wrap_words(&words, avail, space_mm);

        self.ensure_space(line_height_mm);
        // Bullet naranja
        let baseline_y = self.y - pt_to_mm(size_pt);
        self.layer.set_fill_color(hex_rgb(C_ORANGE));
        self.layer.use_text("•", size_pt, Mm(MARGIN_MM), Mm(baseline_y), &self.fonts.bold);

        for (li, line) in lines.iter().enumerate() {
            if li > 0 { self.ensure_space(line_height_mm); }
            let baseline_y = self.y - pt_to_mm(size_pt);
            let mut x = MARGIN_MM + indent_mm;
            for (i, w) in line.iter().enumerate() {
                if i > 0 { x += space_mm; }
                self.layer.set_fill_color(hex_rgb(C_BLACK));
                self.layer.use_text(
                    w.text.clone(), size_pt, Mm(x), Mm(baseline_y),
                    pick_font(self.fonts, w.bold, w.italic),
                );
                x += w.width_mm;
            }
            self.y -= line_height_mm;
        }
    }

    fn render_numbered(&mut self, num: usize, text: &str) {
        let size_pt = 10.0;
        let line_height_mm = pt_to_mm(size_pt) * 1.35;
        let indent_mm = 8.0;
        let label = format!("{}.", num);
        let segs = parse_inline(text);
        let words = segments_to_words(&segs, size_pt);
        let space_mm = text_width_mm(" ", size_pt, false);
        let avail = CONTENT_W_MM - indent_mm;
        let lines = wrap_words(&words, avail, space_mm);

        self.ensure_space(line_height_mm);
        let baseline_y = self.y - pt_to_mm(size_pt);
        self.layer.set_fill_color(hex_rgb(C_NAVY));
        self.layer.use_text(label, size_pt, Mm(MARGIN_MM), Mm(baseline_y), &self.fonts.bold);

        for (li, line) in lines.iter().enumerate() {
            if li > 0 { self.ensure_space(line_height_mm); }
            let baseline_y = self.y - pt_to_mm(size_pt);
            let mut x = MARGIN_MM + indent_mm;
            for (i, w) in line.iter().enumerate() {
                if i > 0 { x += space_mm; }
                self.layer.set_fill_color(hex_rgb(C_BLACK));
                self.layer.use_text(
                    w.text.clone(), size_pt, Mm(x), Mm(baseline_y),
                    pick_font(self.fonts, w.bold, w.italic),
                );
                x += w.width_mm;
            }
            self.y -= line_height_mm;
        }
    }

    /// Renderiza un heading en bold wrappeando por palabras para no desbordar
    /// el margen derecho. Todas las líneas quedan justificadas a la izquierda.
    fn render_heading(&mut self, text: &str, size_pt: f32, color: Color, line_factor: f32) {
        let segs = vec![Segment { text: text.to_string(), bold: true, italic: false }];
        let words = segments_to_words(&segs, size_pt);
        let space_mm = text_width_mm(" ", size_pt, true);
        let lines = wrap_words(&words, CONTENT_W_MM, space_mm);
        let lh = pt_to_mm(size_pt) * line_factor;
        for line in &lines {
            self.ensure_space(lh);
            let baseline_y = self.y - pt_to_mm(size_pt);
            let mut x = MARGIN_MM;
            for (i, w) in line.iter().enumerate() {
                if i > 0 { x += space_mm; }
                self.layer.set_fill_color(color.clone());
                self.layer.use_text(
                    w.text.clone(), size_pt, Mm(x), Mm(baseline_y),
                    pick_font(self.fonts, w.bold, w.italic),
                );
                x += w.width_mm;
            }
            self.y -= lh;
        }
    }

    fn render_h2(&mut self, text: &str) {
        let size_pt = 15.0;
        self.ensure_space(pt_to_mm(size_pt) * 1.4 + 3.0);
        self.y -= 3.0; // espacio antes
        self.render_heading(text, size_pt, hex_rgb(C_BLUE), 1.4);
        // línea azul fina bajo el (último) renglón del heading
        draw_line(&self.layer, MARGIN_MM, self.y + 1.0,
                  PAGE_W_MM - MARGIN_MM, self.y + 1.0,
                  hex_rgb(C_BLUE), 0.5);
        self.y -= 2.0;
    }

    fn render_h3(&mut self, text: &str) {
        let size_pt = 12.0;
        self.ensure_space(pt_to_mm(size_pt) * 1.4 + 2.0);
        self.y -= 2.0;
        self.render_heading(text, size_pt, hex_rgb(C_NAVY), 1.4);
    }

    fn render_sublabel(&mut self, text: &str) {
        let size_pt = 9.0;
        self.ensure_space(pt_to_mm(size_pt) * 1.4 + 2.0);
        self.y -= 2.0;
        self.render_heading(&text.to_uppercase(), size_pt, hex_rgb(C_BLUE), 1.4);
    }

    fn render_sep(&mut self) {
        let size_pt = 11.0;
        let lh = pt_to_mm(size_pt) * 1.6;
        self.ensure_space(lh);
        let baseline_y = self.y - pt_to_mm(size_pt);
        let dots = "· · ·";
        let w = text_width_mm(dots, size_pt, false);
        let x = MARGIN_MM + (CONTENT_W_MM - w) / 2.0;
        self.layer.set_fill_color(hex_rgb(C_ORANGE));
        self.layer.use_text(dots, size_pt, Mm(x), Mm(baseline_y), &self.fonts.regular);
        self.y -= lh;
    }

    fn render_spacer(&mut self, mm: f32) {
        self.ensure_space(mm);
        self.y -= mm;
    }

    fn render_table(&mut self, rows: &[&str]) {
        // Parsear cabecera + filas; descartar la fila "---".
        let header: Vec<String> = parse_row(rows[0]);
        let data_rows: Vec<Vec<String>> = if rows.len() >= 2 && is_separator_row(rows[1]) {
            rows[2..].iter().map(|r| parse_row(r)).collect()
        } else {
            rows[1..].iter().map(|r| parse_row(r)).collect()
        };

        let n_cols = header.len().max(1);
        let col_w = CONTENT_W_MM / n_cols as f32;
        let size_pt = 9.0;
        let row_h = pt_to_mm(size_pt) * 2.0;

        // Cabecera
        self.ensure_space(row_h);
        let top_y = self.y;
        draw_filled_rect(&self.layer, MARGIN_MM, top_y - row_h, CONTENT_W_MM, row_h, hex_rgb(C_NAVY));
        let baseline_y = top_y - row_h + (row_h - pt_to_mm(size_pt)) / 2.0;
        self.layer.set_fill_color(hex_rgb("FFFFFF"));
        for (i, h) in header.iter().enumerate() {
            let x = MARGIN_MM + col_w * i as f32 + 2.0;
            self.layer.use_text(h.clone(), size_pt, Mm(x), Mm(baseline_y), &self.fonts.bold);
        }
        self.y -= row_h;

        // Filas
        for (ri, row) in data_rows.iter().enumerate() {
            self.ensure_space(row_h);
            let top_y = self.y;
            let fill = if ri % 2 == 0 { C_LIGHT } else { C_TBLROW };
            draw_filled_rect(&self.layer, MARGIN_MM, top_y - row_h, CONTENT_W_MM, row_h, hex_rgb(fill));
            let baseline_y = top_y - row_h + (row_h - pt_to_mm(size_pt)) / 2.0;
            self.layer.set_fill_color(hex_rgb(C_BLACK));
            for (i, cell) in row.iter().enumerate() {
                let x = MARGIN_MM + col_w * i as f32 + 2.0;
                // Recortar por caracteres unicode si el contenido excede el
                // ancho de columna. `String::pop()` es seguro (char-aware),
                // pero medimos por chars().count() para que el guard no use
                // bytes y así no iteremos eternamente con UTF-8 multibyte.
                let max_w = col_w - 4.0;
                let mut cell_s = cell.clone();
                while text_width_mm(&cell_s, size_pt, false) > max_w
                    && cell_s.chars().count() > 1
                {
                    cell_s.pop();
                }
                self.layer.use_text(cell_s, size_pt, Mm(x), Mm(baseline_y), &self.fonts.regular);
            }
            self.y -= row_h;
        }

        // Borde inferior fino
        draw_line(&self.layer, MARGIN_MM, self.y, PAGE_W_MM - MARGIN_MM, self.y,
                  hex_rgb("D0DCE9"), 0.3);
        self.y -= 2.0;
    }

    fn render_cover(&mut self, title: &str, label: &str, date: &str) {
        // Bloque navy con marca
        let brand_h = 24.0;
        let brand_y = CONTENT_TOP_MM - brand_h;
        draw_filled_rect(&self.layer, MARGIN_MM, brand_y, 55.0, brand_h, hex_rgb(C_NAVY));
        self.layer.set_fill_color(hex_rgb("FFFFFF"));
        self.layer.use_text("OLIV4600", 16.0, Mm(MARGIN_MM + 4.0), Mm(brand_y + brand_h - 10.0), &self.fonts.bold);
        self.layer.set_fill_color(hex_rgb("66A6EA"));
        self.layer.use_text("Sovereign Intelligence", 9.0, Mm(MARGIN_MM + 4.0), Mm(brand_y + brand_h - 18.0), &self.fonts.regular);

        // Bloque derecho light con label + fecha
        let rx = MARGIN_MM + 55.0;
        let rw = CONTENT_W_MM - 55.0;
        draw_filled_rect(&self.layer, rx, brand_y, rw, brand_h, hex_rgb(C_LIGHT));
        let label_up = label.to_uppercase();
        let label_size = 11.0;
        let lw = text_width_mm(&label_up, label_size, true);
        self.layer.set_fill_color(hex_rgb(C_ORANGE));
        self.layer.use_text(&label_up, label_size, Mm(rx + rw - lw - 4.0), Mm(brand_y + brand_h - 10.0), &self.fonts.bold);
        let date_size = 10.0;
        let dw = text_width_mm(date, date_size, false);
        self.layer.set_fill_color(hex_rgb(C_GREY));
        self.layer.use_text(date, date_size, Mm(rx + rw - dw - 4.0), Mm(brand_y + brand_h - 18.0), &self.fonts.regular);

        self.y = brand_y - 4.0;
        // hrBar azul grueso
        draw_line(&self.layer, MARGIN_MM, self.y, PAGE_W_MM - MARGIN_MM, self.y, hex_rgb(C_BLUE), 2.5);
        self.y -= 10.0;

        // Título grande
        let size_pt = 26.0;
        let baseline_y = self.y - pt_to_mm(size_pt);
        self.layer.set_fill_color(hex_rgb(C_NAVY));
        // Word-wrap del título.
        let max_w = CONTENT_W_MM;
        let mut line = String::new();
        let mut y = baseline_y;
        for word in title.split_whitespace() {
            let tentative = if line.is_empty() { word.to_string() } else { format!("{} {}", line, word) };
            if text_width_mm(&tentative, size_pt, true) > max_w && !line.is_empty() {
                self.layer.use_text(&line, size_pt, Mm(MARGIN_MM), Mm(y), &self.fonts.title);
                y -= pt_to_mm(size_pt) * 1.2;
                line = word.to_string();
            } else {
                line = tentative;
            }
        }
        if !line.is_empty() {
            self.layer.use_text(&line, size_pt, Mm(MARGIN_MM), Mm(y), &self.fonts.title);
            y -= pt_to_mm(size_pt) * 1.2;
        }
        self.y = y - 2.0;
        // hrBar naranja
        draw_line(&self.layer, MARGIN_MM, self.y, PAGE_W_MM - MARGIN_MM, self.y, hex_rgb(C_ORANGE), 1.5);
        self.y -= 8.0;
    }
}

fn parse_row(line: &str) -> Vec<String> {
    let t = line.trim();
    if !t.starts_with('|') || !t.ends_with('|') { return vec![t.to_string()]; }
    let inner = &t[1..t.len() - 1];
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn is_separator_row(line: &str) -> bool {
    let t = line.trim();
    if !t.starts_with('|') || !t.ends_with('|') { return false; }
    t.chars().all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.len() >= 2
}

fn is_hr_marker(s: &str) -> bool {
    if s.len() < 3 { return false; }
    let c = s.chars().next().unwrap();
    if c != '-' && c != '*' && c != '_' { return false; }
    s.chars().all(|ch| ch == c)
}

fn strip_bullet(s: &str) -> Option<String> {
    for prefix in &["- ", "• ", "* "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if prefix == &"* " && rest.starts_with('*') { return None; }
            return Some(rest.to_string());
        }
    }
    None
}

fn strip_numbered(s: &str) -> Option<(usize, String)> {
    let mut end_digit = 0;
    let mut saw_digit = false;
    for (idx, ch) in s.char_indices() {
        if ch.is_ascii_digit() {
            end_digit = idx + ch.len_utf8();
            saw_digit = true;
        } else {
            break;
        }
    }
    if !saw_digit { return None; }
    let n: usize = s[..end_digit].parse().ok()?;
    let rest = &s[end_digit..];
    if let Some(r) = rest.strip_prefix(". ") { return Some((n, r.to_string())); }
    if let Some(r) = rest.strip_prefix(") ") { return Some((n, r.to_string())); }
    None
}

fn only_bold(s: &str) -> Option<&str> {
    if let Some(inner) = s.strip_prefix("**").and_then(|s| s.strip_suffix("**")) {
        if !inner.contains('*') && !inner.is_empty() {
            return Some(inner);
        }
    }
    None
}

// ── API pública ───────────────────────────────────────────────────────────────

pub fn build_pdf_bytes(
    text: &str,
    label: &str,
    title: &str,
    date: &str,
) -> Result<Vec<u8>, String> {
    // Crear doc + primera página.
    let (doc, page1, layer1) = PdfDocument::new(title, Mm(PAGE_W_MM), Mm(PAGE_H_MM), "cover");
    let fonts = load_fonts(&doc)?;
    let layer = doc.get_page(page1).get_layer(layer1);

    let mut r = Renderer::new(&doc, &fonts, layer, label, page1, layer1);

    // Portada (sin header/footer visible — es página 1 pero no queremos ensuciarla).
    // Por simplicidad dibujamos footer en todas pero dejamos header fuera de la portada
    // para que se vea limpia.
    r.render_cover(title, label, date);

    // Cuerpo
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            r.render_spacer(2.0);
            i += 1;
            continue;
        }

        if is_hr_marker(trimmed) {
            r.render_sep();
            i += 1;
            continue;
        }

        if is_table_row(trimmed) {
            let mut tlines: Vec<&str> = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim()) {
                tlines.push(lines[i]);
                i += 1;
            }
            if tlines.len() >= 2 {
                r.render_spacer(2.0);
                r.render_table(&tlines);
                r.render_spacer(2.0);
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("## ") {
            r.render_h2(rest);
            i += 1;
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("### ") {
            r.render_h3(rest);
            i += 1;
            continue;
        }

        if let Some(rest) = strip_bullet(trimmed) {
            r.render_bullet(&rest);
            i += 1;
            continue;
        }

        if let Some((n, rest)) = strip_numbered(trimmed) {
            r.render_numbered(n, &rest);
            i += 1;
            continue;
        }

        if let Some(bold) = only_bold(trimmed) {
            r.render_sublabel(bold);
            i += 1;
            continue;
        }

        // Párrafo normal
        r.render_inline(trimmed, 10.5, hex_rgb(C_BLACK), false);
        r.render_spacer(2.0);
        i += 1;
    }

    let total_pages = r.pages.len();

    // Iteramos las páginas reales (índices cacheados) para pintar header+footer.
    // La portada (índice 0) recibe solo footer; el resto, header + footer.
    for (p_idx, (page_ix, layer_ix)) in r.pages.iter().enumerate() {
        let page = doc.get_page(*page_ix);
        let layer = page.get_layer(*layer_ix);
        // Footer siempre
        layer.set_outline_color(hex_rgb("CCCCCC"));
        layer.set_outline_thickness(0.3);
        let l = Line {
            points: vec![
                (Point::new(Mm(MARGIN_MM), Mm(FOOTER_Y_MM + 5.0)), false),
                (Point::new(Mm(PAGE_W_MM - MARGIN_MM), Mm(FOOTER_Y_MM + 5.0)), false),
            ],
            is_closed: false,
        };
        layer.add_line(l);
        layer.set_fill_color(hex_rgb(C_GREY));
        layer.use_text(
            "OLIV4600 · Sovereign Intelligence · 100% Offline",
            8.0, Mm(MARGIN_MM), Mm(FOOTER_Y_MM), &fonts.regular,
        );
        let pg = format!("Página {} de {}", p_idx + 1, total_pages);
        let w = text_width_mm(&pg, 8.0, false);
        layer.use_text(
            pg.clone(), 8.0,
            Mm(PAGE_W_MM - MARGIN_MM - w),
            Mm(FOOTER_Y_MM),
            &fonts.regular,
        );

        // Header solo para páginas ≥ 2 (la 1 es la portada)
        if p_idx >= 1 {
            layer.set_fill_color(hex_rgb(C_NAVY));
            layer.use_text("OLIV4600", 9.0, Mm(MARGIN_MM), Mm(HEADER_Y_MM), &fonts.bold);
            layer.set_fill_color(hex_rgb(C_GREY));
            let offset = text_width_mm("OLIV4600", 9.0, true);
            layer.use_text(
                format!(" · {}", label),
                9.0,
                Mm(MARGIN_MM + offset),
                Mm(HEADER_Y_MM),
                &fonts.regular,
            );
            layer.set_outline_color(hex_rgb("CCCCCC"));
            layer.set_outline_thickness(0.3);
            let l = Line {
                points: vec![
                    (Point::new(Mm(MARGIN_MM), Mm(HEADER_Y_MM - 2.0)), false),
                    (Point::new(Mm(PAGE_W_MM - MARGIN_MM), Mm(HEADER_Y_MM - 2.0)), false),
                ],
                is_closed: false,
            };
            layer.add_line(l);
        }
    }

    // Serializar
    doc.save_to_bytes().map_err(|e| format!("pdf save: {e}"))
}

// Permitir que title_roman se use opcionalmente en el futuro; suprime warning.
#[allow(dead_code)]
fn _use_title_roman(fonts: &Fonts) -> &IndirectFontRef { &fonts.title_roman }
