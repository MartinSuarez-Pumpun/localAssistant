/**
 * render_docx.rs — Generador DOCX 100% Rust usando docx-rs.
 *
 * Paleta corporativa, portada con tabla, hrBars, headers/footers, parsing de
 * markdown (párrafos, títulos, listas, tablas, separadores, sub-etiquetas) y
 * estilos tipográficos (Georgia para cuerpo, Arial para títulos).
 */

use docx_rs::*;

// ── Paleta corporativa ────────────────────────────────────────────────────────

pub const C_NAVY:   &str = "002542";
pub const C_BLUE:   &str = "2E75B6";
pub const C_ORANGE: &str = "C45911";
pub const C_LIGHT:  &str = "EBF3FB";
pub const C_GREY:   &str = "6B7280";
pub const C_WHITE:  &str = "FFFFFF";
pub const C_BLACK:  &str = "1A1A2E";
pub const C_TBLROW: &str = "F7FAFD";
pub const C_TBLBORDER: &str = "D0DCE9";

const NUM_BULLETS: usize = 1;
const NUM_NUMBERS: usize = 2;

// ── Estilo base para runs de cuerpo ───────────────────────────────────────────

#[derive(Clone, Copy)]
struct RunStyle<'a> {
    font: &'a str,
    size: usize,
    color: &'a str,
    bold: bool,
    italic: bool,
}

impl<'a> Default for RunStyle<'a> {
    fn default() -> Self {
        Self { font: "Georgia", size: 22, color: C_BLACK, bold: false, italic: false }
    }
}

fn run_with_style(text: &str, style: RunStyle<'_>) -> Run {
    let mut r = Run::new()
        .add_text(text)
        .fonts(
            RunFonts::new()
                .ascii(style.font)
                .hi_ansi(style.font)
                .cs(style.font),
        )
        .size(style.size)
        .color(style.color);
    if style.bold { r = r.bold(); }
    if style.italic { r = r.italic(); }
    r
}

// ── Helpers de párrafo ────────────────────────────────────────────────────────

fn spacer(pts: u32) -> Paragraph {
    Paragraph::new()
        .line_spacing(LineSpacing::new().before(pts).after(pts))
        .add_run(Run::new().add_text(""))
}

/// Aplica un `ParagraphBorder` a un Paragraph vía su campo `property`,
/// porque docx-rs 0.4 no expone `set_border` directamente en `Paragraph`.
fn with_border(mut p: Paragraph, border: ParagraphBorder) -> Paragraph {
    p.property = p.property.set_border(border);
    p
}

/// Línea horizontal como párrafo con borde inferior.
fn hr_bar(color: &str, thickness: usize) -> Paragraph {
    let p = Paragraph::new()
        .line_spacing(LineSpacing::new().before(0).after(0))
        .add_run(Run::new().add_text(""));
    with_border(p, ParagraphBorder::new(ParagraphBorderPosition::Bottom)
        .val(BorderType::Single)
        .size(thickness)
        .color(color)
        .space(1))
}

/// Separador decorativo "· · ·" centrado en naranja.
fn decorative_sep() -> Paragraph {
    Paragraph::new()
        .align(AlignmentType::Center)
        .line_spacing(LineSpacing::new().before(160).after(160))
        .add_run(run_with_style(
            "· · ·",
            RunStyle { font: "Georgia", size: 22, color: C_ORANGE, ..Default::default() },
        ))
}

// ── Parser de inline markdown ─────────────────────────────────────────────────
//
// Reconoce ***bold+italic***, **bold**, *italic*. Devuelve una lista de runs.

fn inline_runs(text: &str, base: RunStyle<'_>) -> Vec<Run> {
    let bytes = text.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    let mut buf = String::new();

    let flush = |buf: &mut String, runs: &mut Vec<Run>, st: RunStyle<'_>| {
        if !buf.is_empty() {
            runs.push(run_with_style(buf, st));
            buf.clear();
        }
    };

    while i < bytes.len() {
        // ***text*** → bold + italic
        if bytes[i..].starts_with(b"***") {
            if let Some(end) = find_closing(&text[i + 3..], "***") {
                flush(&mut buf, &mut runs, base);
                let inner = &text[i + 3..i + 3 + end];
                let mut st = base;
                st.bold = true;
                st.italic = true;
                runs.push(run_with_style(inner, st));
                i += 3 + end + 3;
                continue;
            }
        }
        // **text** → bold
        if bytes[i..].starts_with(b"**") {
            if let Some(end) = find_closing(&text[i + 2..], "**") {
                flush(&mut buf, &mut runs, base);
                let inner = &text[i + 2..i + 2 + end];
                let mut st = base;
                st.bold = true;
                runs.push(run_with_style(inner, st));
                i += 2 + end + 2;
                continue;
            }
        }
        // *text* → italic
        if bytes[i] == b'*' {
            if let Some(end) = find_closing(&text[i + 1..], "*") {
                // Evitar emparejar si el siguiente char es * (ya intentado antes).
                flush(&mut buf, &mut runs, base);
                let inner = &text[i + 1..i + 1 + end];
                let mut st = base;
                st.italic = true;
                runs.push(run_with_style(inner, st));
                i += 1 + end + 1;
                continue;
            }
        }

        // Avanzar un char unicode.
        let ch_len = next_char_len(&text[i..]);
        buf.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }

    flush(&mut buf, &mut runs, base);
    if runs.is_empty() {
        runs.push(run_with_style("", base));
    }
    runs
}

fn find_closing(haystack: &str, needle: &str) -> Option<usize> {
    haystack.find(needle)
}

fn next_char_len(s: &str) -> usize {
    s.chars().next().map(|c| c.len_utf8()).unwrap_or(1)
}

// ── Tabla markdown ────────────────────────────────────────────────────────────

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.len() >= 2
}

fn parse_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let inner = &t[1..t.len() - 1];
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn build_table(rows: &[&str]) -> Table {
    // rows[0] = cabecera, rows[1] = separador (---), rows[2..] = datos.
    let mut data: Vec<&&str> = rows.iter().enumerate()
        .filter(|(i, _)| *i != 1)
        .map(|(_, r)| r).collect();
    // Si no hay separador (solo 1 fila), data se queda igual.
    if rows.len() == 1 { data = rows.iter().collect(); }

    let mut table_rows: Vec<TableRow> = Vec::with_capacity(data.len());

    for (ri, row_str) in data.iter().enumerate() {
        let cells = parse_cells(row_str);
        let is_header = ri == 0;

        let row_cells: Vec<TableCell> = cells.iter().map(|cell_text| {
            let fill = if is_header {
                C_NAVY
            } else if ri % 2 == 0 {
                C_WHITE
            } else {
                C_TBLROW
            };

            let borders = TableCellBorders::new()
                .set(TableCellBorder::new(TableCellBorderPosition::Top)
                    .color(C_TBLBORDER).size(2))
                .set(TableCellBorder::new(TableCellBorderPosition::Bottom)
                    .color(C_TBLBORDER).size(2))
                .set(TableCellBorder::new(TableCellBorderPosition::Left)
                    .color(C_TBLBORDER).size(2))
                .set(TableCellBorder::new(TableCellBorderPosition::Right)
                    .color(C_TBLBORDER).size(2));

            let para = if is_header {
                let header_run = Run::new()
                    .add_text(cell_text)
                    .fonts(
                        RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"),
                    )
                    .size(18)
                    .bold()
                    .color(C_WHITE);
                Paragraph::new()
                    .align(AlignmentType::Center)
                    .line_spacing(LineSpacing::new().before(40).after(40))
                    .add_run(header_run)
            } else {
                let base = RunStyle {
                    font: "Georgia",
                    size: 20,
                    color: C_BLACK,
                    ..Default::default()
                };
                let mut p = Paragraph::new()
                    .align(AlignmentType::Left)
                    .line_spacing(LineSpacing::new().before(40).after(40));
                for r in inline_runs(cell_text, base) {
                    p = p.add_run(r);
                }
                p
            };

            TableCell::new()
                .shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(fill))
                .set_borders(borders)
                .add_paragraph(para)
        }).collect();

        table_rows.push(TableRow::new(row_cells));
    }

    // Layout fijo + grid explícito para que Word respete los anchos de columna
    // y rompa tokens largos (URLs, identificadores) dentro de la celda en vez
    // de desbordar hacia la derecha.
    let n_cols = table_rows
        .first()
        .map(|r| r.cells.len())
        .unwrap_or(1)
        .max(1);
    let total_dxa: usize = 9026;
    let col_dxa = total_dxa / n_cols;
    let grid: Vec<usize> = (0..n_cols).map(|_| col_dxa).collect();
    Table::new(table_rows)
        .set_grid(grid)
        .layout(TableLayoutType::Fixed)
        .width(total_dxa, WidthType::Dxa)
}

// ── Parser de cuerpo (markdown → párrafos / tablas) ───────────────────────────

pub enum BodyBlock {
    Paragraph(Paragraph),
    Table(Table),
}

pub fn body_blocks(raw: &str) -> Vec<BodyBlock> {
    let lines: Vec<&str> = raw.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let mut out: Vec<BodyBlock> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Línea vacía
        if trimmed.is_empty() {
            out.push(BodyBlock::Paragraph(spacer(0)));
            i += 1;
            continue;
        }

        // Separador horizontal
        if is_hr_marker(trimmed) {
            out.push(BodyBlock::Paragraph(decorative_sep()));
            i += 1;
            continue;
        }

        // Tabla
        if is_table_row(trimmed) {
            let mut table_lines: Vec<&str> = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim()) {
                table_lines.push(lines[i]);
                i += 1;
            }
            if table_lines.len() >= 2 {
                out.push(BodyBlock::Paragraph(spacer(80)));
                out.push(BodyBlock::Table(build_table(&table_lines)));
                out.push(BodyBlock::Paragraph(spacer(80)));
            } else if !table_lines.is_empty() {
                // Una sola línea tipo tabla — renderizar como párrafo plano.
                out.push(BodyBlock::Paragraph(plain_paragraph(table_lines[0])));
            }
            continue;
        }

        // ## Encabezado
        if let Some(rest) = trimmed.strip_prefix("## ") {
            out.push(BodyBlock::Paragraph(spacer(120)));
            let h_run = Run::new()
                .add_text(rest)
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(26)
                .bold()
                .color(C_BLUE);
            out.push(BodyBlock::Paragraph(
                Paragraph::new()
                    .line_spacing(LineSpacing::new().before(0).after(60))
                    .add_run(h_run),
            ));
            out.push(BodyBlock::Paragraph(hr_bar(C_BLUE, 2)));
            out.push(BodyBlock::Paragraph(spacer(40)));
            i += 1;
            continue;
        }

        // ### Subtítulo
        if let Some(rest) = trimmed.strip_prefix("### ") {
            out.push(BodyBlock::Paragraph(spacer(80)));
            let base = RunStyle {
                font: "Arial", size: 22, color: C_NAVY, bold: true, italic: false,
            };
            let mut p = Paragraph::new()
                .line_spacing(LineSpacing::new().before(0).after(60));
            for r in inline_runs(rest, base) {
                p = p.add_run(r);
            }
            out.push(BodyBlock::Paragraph(p));
            i += 1;
            continue;
        }

        // Bullets: "- item" o "• item"
        if let Some(rest) = strip_bullet(trimmed) {
            let mut p = Paragraph::new()
                .numbering(NumberingId::new(NUM_BULLETS), IndentLevel::new(0))
                .line_spacing(LineSpacing::new().before(40).after(40).line(300));
            for r in inline_runs(&rest, RunStyle::default()) {
                p = p.add_run(r);
            }
            out.push(BodyBlock::Paragraph(p));
            i += 1;
            continue;
        }

        // Numerada: "1. item" o "1) item"
        if let Some(rest) = strip_numbered(trimmed) {
            let mut p = Paragraph::new()
                .numbering(NumberingId::new(NUM_NUMBERS), IndentLevel::new(0))
                .line_spacing(LineSpacing::new().before(40).after(40).line(300));
            for r in inline_runs(&rest, RunStyle::default()) {
                p = p.add_run(r);
            }
            out.push(BodyBlock::Paragraph(p));
            i += 1;
            continue;
        }

        // Línea sólo **BOLD** → sub-etiqueta
        if let Some(bold_text) = only_bold(trimmed) {
            let r = Run::new()
                .add_text(bold_text.to_uppercase())
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(18)
                .bold()
                .color(C_BLUE);
            out.push(BodyBlock::Paragraph(
                Paragraph::new()
                    .line_spacing(LineSpacing::new().before(140).after(20))
                    .add_run(r),
            ));
            i += 1;
            continue;
        }

        // Párrafo normal
        out.push(BodyBlock::Paragraph(plain_paragraph(trimmed)));
        i += 1;
    }

    out
}

fn plain_paragraph(text: &str) -> Paragraph {
    let mut p = Paragraph::new()
        .align(AlignmentType::Justified)
        .line_spacing(LineSpacing::new().before(0).after(160).line(340));
    for r in inline_runs(text, RunStyle::default()) {
        p = p.add_run(r);
    }
    p
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
            // Evitar confundir "**bold**" con lista "* bold".
            if prefix == &"* " && rest.starts_with('*') { return None; }
            return Some(rest.to_string());
        }
    }
    None
}

fn strip_numbered(s: &str) -> Option<String> {
    let mut iter = s.char_indices();
    let mut end_digit = 0;
    let mut saw_digit = false;
    while let Some((idx, ch)) = iter.next() {
        if ch.is_ascii_digit() {
            end_digit = idx + ch.len_utf8();
            saw_digit = true;
        } else {
            break;
        }
    }
    if !saw_digit { return None; }
    let rest = &s[end_digit..];
    if rest.starts_with(". ") {
        return Some(rest[2..].to_string());
    }
    if rest.starts_with(") ") {
        return Some(rest[2..].to_string());
    }
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

// ── Portada ───────────────────────────────────────────────────────────────────

fn build_cover(label: &str, title: &str, date: &str) -> Vec<BodyBlock> {
    // Tabla 2 celdas: izq navy "OLIV4600", der light con label + fecha.
    let no_border = |c: TableCell| {
        c.set_borders(TableCellBorders::new()
            .set(TableCellBorder::new(TableCellBorderPosition::Top).border_type(BorderType::Nil).color("auto"))
            .set(TableCellBorder::new(TableCellBorderPosition::Bottom).border_type(BorderType::Nil).color("auto"))
            .set(TableCellBorder::new(TableCellBorderPosition::Left).border_type(BorderType::Nil).color("auto"))
            .set(TableCellBorder::new(TableCellBorderPosition::Right).border_type(BorderType::Nil).color("auto")))
    };

    // Izquierda: "OLIV4600" + subtítulo
    let left_line1 = Paragraph::new()
        .align(AlignmentType::Left)
        .line_spacing(LineSpacing::new().before(0).after(0))
        .add_run(
            Run::new().add_text("OLIV")
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(24).bold().color(C_WHITE),
        )
        .add_run(
            Run::new().add_text("4600")
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(24).bold().color("66A6EA"),
        );
    let left_line2 = Paragraph::new()
        .line_spacing(LineSpacing::new().before(0).after(0))
        .add_run(
            Run::new().add_text("Sovereign Intelligence")
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(14).color("66A6EA"),
        );

    let left_cell = no_border(TableCell::new()
        .shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(C_NAVY))
        .vertical_align(VAlignType::Center)
        .width(2000, WidthType::Dxa)
        .add_paragraph(left_line1)
        .add_paragraph(left_line2));

    // Derecha: label uppercase naranja + fecha gris
    let right_label = Paragraph::new()
        .align(AlignmentType::Right)
        .line_spacing(LineSpacing::new().before(0).after(0))
        .add_run(
            Run::new().add_text(label.to_uppercase())
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(18).bold().color(C_ORANGE),
        );
    let right_date = Paragraph::new()
        .align(AlignmentType::Right)
        .line_spacing(LineSpacing::new().before(0).after(0))
        .add_run(
            Run::new().add_text(date)
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(16).color(C_GREY),
        );
    let right_cell = no_border(TableCell::new()
        .shading(Shading::new().shd_type(ShdType::Clear).color("auto").fill(C_LIGHT))
        .vertical_align(VAlignType::Center)
        .width(7026, WidthType::Dxa)
        .add_paragraph(right_label)
        .add_paragraph(right_date));

    let cover_table = Table::new(vec![TableRow::new(vec![left_cell, right_cell])])
        .set_grid(vec![2000, 7026])
        .width(9026, WidthType::Dxa)
        // Desactivar bordes de tabla
        .set_borders(TableBorders::with_empty());

    // Título principal
    let title_para = Paragraph::new()
        .line_spacing(LineSpacing::new().before(240).after(60))
        .add_run(
            Run::new().add_text(title)
                .fonts(RunFonts::new().ascii("Georgia").hi_ansi("Georgia").cs("Georgia"))
                .size(52).bold().color(C_NAVY),
        );

    vec![
        BodyBlock::Table(cover_table),
        BodyBlock::Paragraph(spacer(40)),
        BodyBlock::Paragraph(hr_bar(C_BLUE, 10)),
        BodyBlock::Paragraph(spacer(20)),
        BodyBlock::Paragraph(title_para),
        BodyBlock::Paragraph(hr_bar(C_ORANGE, 6)),
        BodyBlock::Paragraph(spacer(60)),
    ]
}

// ── Header / Footer ───────────────────────────────────────────────────────────

fn build_header(label: &str) -> Header {
    // "OLIV4600\t{label}" con borde inferior gris.
    let p = with_border(Paragraph::new(),
        ParagraphBorder::new(ParagraphBorderPosition::Bottom)
            .val(BorderType::Single)
            .size(2)
            .color("CCCCCC")
            .space(6))
        .add_tab(Tab::new().val(TabValueType::Right).pos(9026))
        .add_run(
            Run::new().add_text("OLIV4600")
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(16).bold().color(C_NAVY),
        )
        .add_run(Run::new().add_tab())
        .add_run(
            Run::new().add_text(label)
                .fonts(RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial"))
                .size(16).color(C_GREY),
        );
    Header::new().add_paragraph(p)
}

fn build_footer() -> Footer {
    // "OLIV4600 · Sovereign Intelligence · 100% Offline\tPágina N de T"
    let grey_font = RunFonts::new().ascii("Arial").hi_ansi("Arial").cs("Arial");

    let p = with_border(Paragraph::new(),
        ParagraphBorder::new(ParagraphBorderPosition::Top)
            .val(BorderType::Single)
            .size(2)
            .color("CCCCCC")
            .space(6))
        .add_tab(Tab::new().val(TabValueType::Right).pos(9026))
        .add_run(
            Run::new().add_text("OLIV4600 · Sovereign Intelligence · 100% Offline")
                .fonts(grey_font.clone())
                .size(16).color(C_GREY),
        )
        .add_run(Run::new().add_tab())
        .add_run(
            Run::new().add_text("Página ")
                .fonts(grey_font.clone()).size(16).color(C_GREY),
        )
        .add_page_num(PageNum::new())
        .add_run(
            Run::new().add_text(" de ")
                .fonts(grey_font.clone()).size(16).color(C_GREY),
        )
        .add_num_pages(NumPages::new());

    Footer::new().add_paragraph(p)
}

// ── Numbering (bullets + numeradas) ───────────────────────────────────────────

fn add_numbering(doc: Docx) -> Docx {
    let bullet_level = Level::new(
        0,
        Start::new(1),
        NumberFormat::new("bullet"),
        LevelText::new("•"),
        LevelJc::new("left"),
    )
    .indent(Some(720), Some(SpecialIndentType::Hanging(360)), None, None)
    .fonts(RunFonts::new().ascii("Symbol").hi_ansi("Symbol").cs("Symbol"))
    .color(C_ORANGE);

    let num_level = Level::new(
        0,
        Start::new(1),
        NumberFormat::new("decimal"),
        LevelText::new("%1."),
        LevelJc::new("left"),
    )
    .indent(Some(720), Some(SpecialIndentType::Hanging(360)), None, None)
    .bold()
    .color(C_NAVY);

    doc.add_abstract_numbering(
        AbstractNumbering::new(NUM_BULLETS).add_level(bullet_level),
    )
    .add_numbering(Numbering::new(NUM_BULLETS, NUM_BULLETS))
    .add_abstract_numbering(
        AbstractNumbering::new(NUM_NUMBERS).add_level(num_level),
    )
    .add_numbering(Numbering::new(NUM_NUMBERS, NUM_NUMBERS))
}

// ── API pública ───────────────────────────────────────────────────────────────

pub fn build_docx_bytes(
    text: &str,
    label: &str,
    title: &str,
    date: &str,
) -> Result<Vec<u8>, String> {
    let mut doc = Docx::new()
        .page_size(11906, 16838)
        .page_margin(
            PageMargin::new()
                .top(1134).right(1134).bottom(1134).left(1134)
                .header(567).footer(567),
        )
        .default_fonts(RunFonts::new().ascii("Georgia").hi_ansi("Georgia").cs("Georgia"))
        .default_size(22)
        .header(build_header(label))
        .footer(build_footer());

    doc = add_numbering(doc);

    // Portada
    for block in build_cover(label, title, date) {
        doc = match block {
            BodyBlock::Paragraph(p) => doc.add_paragraph(p),
            BodyBlock::Table(t) => doc.add_table(t),
        };
    }

    // Cuerpo
    for block in body_blocks(text) {
        doc = match block {
            BodyBlock::Paragraph(p) => doc.add_paragraph(p),
            BodyBlock::Table(t) => doc.add_table(t),
        };
    }

    let mut buf: Vec<u8> = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        doc.build().pack(cursor).map_err(|e| format!("docx pack: {e}"))?;
    }
    Ok(buf)
}
