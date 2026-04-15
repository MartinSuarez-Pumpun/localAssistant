"use strict";
/**
 * make_docx.js — OLIV4600 Professional Document Generator v2
 *
 * Uso:  node make_docx.js <input.json> <output.docx>
 * JSON: { text, label, title, date }
 *
 * Soporta:
 *   ## Sección / ### Subsección      → encabezados jerárquicos
 *   **negrita**  *cursiva*  ***ambas*** → runs inline con formato real
 *   - item / 1. item                 → listas con viñeta naranja / numeradas
 *   | col | col |                    → tablas con cabecera azul
 *   --- / ***  solos                  → separador decorativo
 *   Líneas sólo-bold (**TEXTO**)     → sub-etiqueta en azul versalita
 */

const fs   = require("fs");
const path = require("path");

const {
  Document, Packer, Paragraph, TextRun, Header, Footer,
  AlignmentType, BorderStyle, ShadingType, WidthType,
  PageNumber, LevelFormat, Table, TableRow, TableCell,
  TabStopType, VerticalAlign,
} = require("docx");

// ── CLI ───────────────────────────────────────────────────────────────────────
const [,, inputFile, outputFile] = process.argv;
if (!inputFile || !outputFile) {
  console.error("Uso: node make_docx.js <input.json> <output.docx>");
  process.exit(1);
}

const { text = "", label = "Documento", title = "", date = "" } =
  JSON.parse(fs.readFileSync(inputFile, "utf8"));

const docTitle = title || label;
const docDate  = date  || new Date().toLocaleDateString("es-ES", {
  day: "2-digit", month: "long", year: "numeric",
});

// ── Paleta corporativa ────────────────────────────────────────────────────────
const C = {
  navy:   "002542",   // fondo sidebar / títulos principales
  blue:   "2E75B6",   // encabezados de sección / acento
  orange: "C45911",   // viñetas / líneas decorativas / sub-etiquetas
  light:  "EBF3FB",   // fondo cabecera suave / fondo tabla header
  grey:   "6B7280",   // texto secundario / fecha
  white:  "FFFFFF",
  black:  "1A1A2E",   // cuerpo de texto
  tblrow: "F7FAFD",   // fondo filas alternas de tabla
};

// ── Helpers de párrafo ────────────────────────────────────────────────────────

function spacer(pts = 0) {
  return new Paragraph({
    spacing: { before: pts, after: pts },
    children: [new TextRun("")],
  });
}

function hrBar(color, thickness = 4) {
  return new Paragraph({
    spacing: { before: 0, after: 0 },
    border: { bottom: { style: BorderStyle.SINGLE, size: thickness, color, space: 1 } },
    children: [new TextRun("")],
  });
}

/** Separador decorativo centrado "· · ·" */
function decorativeSep() {
  return new Paragraph({
    alignment: AlignmentType.CENTER,
    spacing: { before: 160, after: 160 },
    children: [
      new TextRun({ text: "· · ·", font: "Georgia", size: 22, color: C.orange }),
    ],
  });
}

// ── Parser de inline markdown ─────────────────────────────────────────────────

/**
 * Convierte una cadena con markdown inline en un array de TextRun.
 * Soporta ***bold+italic***, **bold**, *italic*, el resto es texto plano.
 */
function inlineRuns(text, base = {}) {
  const {
    font   = "Georgia",
    size   = 22,
    color  = C.black,
    bold   = false,
    italics = false,
  } = base;

  const runs   = [];
  const regex  = /\*\*\*([\s\S]*?)\*\*\*|\*\*([\s\S]*?)\*\*|\*([\s\S]*?)\*/g;
  let lastIdx  = 0;
  let match;

  const makeRun = (t, extraBold, extraItalic) =>
    new TextRun({ text: t, font, size, color,
                  bold: bold || extraBold,
                  italics: italics || extraItalic });

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIdx) {
      runs.push(makeRun(text.slice(lastIdx, match.index), false, false));
    }
    if (match[1] != null) runs.push(makeRun(match[1], true, true));   // ***
    else if (match[2] != null) runs.push(makeRun(match[2], true, false)); // **
    else if (match[3] != null) runs.push(makeRun(match[3], false, true)); // *
    lastIdx = regex.lastIndex;
  }
  if (lastIdx < text.length) {
    runs.push(makeRun(text.slice(lastIdx), false, false));
  }
  return runs.length ? runs : [makeRun(text, false, false)];
}

// ── Parser de tablas markdown ─────────────────────────────────────────────────

function isTableRow(line) {
  return line.trim().startsWith("|") && line.trim().endsWith("|");
}

function parseCells(line) {
  return line.trim().slice(1, -1).split("|").map(c => c.trim());
}

function buildTable(rows) {
  // rows[0] = cabecera, rows[1] = separador (---), rows[2..] = datos
  const dataRows = rows.filter((_, i) => i !== 1); // quitar separador

  return new Table({
    width: { size: 9026, type: WidthType.DXA },
    margins: { top: 80, bottom: 80, left: 120, right: 120 },
    rows: dataRows.map((row, ri) => {
      const cells = parseCells(row);
      const isHeader = ri === 0;
      return new TableRow({
        tableHeader: isHeader,
        children: cells.map(cellText => new TableCell({
          shading: isHeader
            ? { fill: C.navy,  type: ShadingType.CLEAR }
            : ri % 2 === 0
              ? { fill: C.white,  type: ShadingType.CLEAR }
              : { fill: C.tblrow, type: ShadingType.CLEAR },
          margins: { top: 80, bottom: 80, left: 160, right: 160 },
          borders: {
            top:    { style: BorderStyle.SINGLE, size: 1, color: "D0DCE9" },
            bottom: { style: BorderStyle.SINGLE, size: 1, color: "D0DCE9" },
            left:   { style: BorderStyle.SINGLE, size: 1, color: "D0DCE9" },
            right:  { style: BorderStyle.SINGLE, size: 1, color: "D0DCE9" },
          },
          children: [new Paragraph({
            alignment: isHeader ? AlignmentType.CENTER : AlignmentType.LEFT,
            spacing: { before: 40, after: 40 },
            children: isHeader
              ? [new TextRun({ text: cellText, font: "Arial", size: 18,
                               bold: true, color: C.white })]
              : inlineRuns(cellText, { font: "Georgia", size: 20, color: C.black }),
          })],
        })),
      });
    }),
  });
}

// ── Cuerpo del documento ──────────────────────────────────────────────────────

function bodyParagraphs(raw) {
  const lines  = raw.split(/\r?\n/);
  const result = [];
  let i = 0;

  while (i < lines.length) {
    const line    = lines[i];
    const trimmed = line.trim();

    // ── Línea vacía ─────────────────────────────────────────────────────────
    if (!trimmed) {
      result.push(spacer());
      i++; continue;
    }

    // ── Separador horizontal --- o *** solos ────────────────────────────────
    if (/^(-{3,}|\*{3,}|_{3,})$/.test(trimmed)) {
      result.push(decorativeSep());
      i++; continue;
    }

    // ── Tabla markdown ──────────────────────────────────────────────────────
    if (isTableRow(trimmed)) {
      const tableLines = [];
      while (i < lines.length && isTableRow(lines[i].trim() || lines[i])) {
        tableLines.push(lines[i]);
        i++;
      }
      if (tableLines.length >= 2) {
        result.push(spacer(80));
        result.push(buildTable(tableLines));
        result.push(spacer(80));
      }
      continue;
    }

    // ── Encabezados markdown ## / ### ───────────────────────────────────────
    const h2 = trimmed.match(/^##\s+(.*)/);
    const h3 = trimmed.match(/^###\s+(.*)/);

    if (h2) {
      const htxt = h2[1];
      result.push(spacer(120));
      result.push(new Paragraph({
        spacing: { before: 0, after: 60 },
        children: [new TextRun({
          text: htxt, font: "Arial", size: 26, bold: true, color: C.blue,
        })],
      }));
      result.push(hrBar(C.blue, 2));
      result.push(spacer(40));
      i++; continue;
    }

    if (h3) {
      result.push(spacer(80));
      result.push(new Paragraph({
        spacing: { before: 0, after: 60 },
        children: inlineRuns(h3[1], { font: "Arial", size: 22, bold: true, color: C.navy }),
      }));
      i++; continue;
    }

    // ── Listas ──────────────────────────────────────────────────────────────
    const bullet = trimmed.match(/^[-•]\s+(.*)/);
    const num    = trimmed.match(/^(\d+)[.)]\s+(.*)/);

    if (bullet) {
      result.push(new Paragraph({
        numbering: { reference: "oliv-bullets", level: 0 },
        spacing: { before: 40, after: 40, line: 300 },
        children: inlineRuns(bullet[1]),
      }));
      i++; continue;
    }

    if (num) {
      result.push(new Paragraph({
        numbering: { reference: "oliv-numbers", level: 0 },
        spacing: { before: 40, after: 40, line: 300 },
        children: inlineRuns(num[2]),
      }));
      i++; continue;
    }

    // ── Línea sólo-bold: **TEXTO** → sub-etiqueta en azul ──────────────────
    const soloB = trimmed.match(/^\*\*([^*]+)\*\*$/);
    if (soloB) {
      result.push(new Paragraph({
        spacing: { before: 140, after: 20 },
        children: [new TextRun({
          text: soloB[1].toUpperCase(),
          font: "Arial", size: 18, bold: true,
          color: C.blue, characterSpacing: 50,
        })],
      }));
      i++; continue;
    }

    // ── Párrafo normal con inline markdown ──────────────────────────────────
    result.push(new Paragraph({
      alignment: AlignmentType.JUSTIFIED,
      spacing: { before: 0, after: 160, line: 340 },
      children: inlineRuns(trimmed),
    }));
    i++;
  }

  return result;
}

// ── Cabecera del documento (portada) ─────────────────────────────────────────

const coverBlock = [
  new Table({
    width: { size: 9026, type: WidthType.DXA },
    columnWidths: [2000, 7026],
    rows: [new TableRow({ children: [
      // Celda izquierda — marca OLIV4600
      new TableCell({
        shading: { fill: C.navy, type: ShadingType.CLEAR },
        margins: { top: 220, bottom: 220, left: 280, right: 280 },
        verticalAlign: VerticalAlign.CENTER,
        borders: { top: { style: BorderStyle.NONE }, bottom: { style: BorderStyle.NONE },
                   left: { style: BorderStyle.NONE }, right: { style: BorderStyle.NONE } },
        children: [
          new Paragraph({ alignment: AlignmentType.LEFT, spacing: { before: 0, after: 0 }, children: [
            new TextRun({ text: "OLIV", font: "Arial", size: 24, bold: true, color: C.white }),
            new TextRun({ text: "4600", font: "Arial", size: 24, bold: true, color: "66A6EA" }),
          ]}),
          new Paragraph({ spacing: { before: 0, after: 0 }, children: [
            new TextRun({ text: "Sovereign Intelligence", font: "Arial", size: 14, color: "66A6EA" }),
          ]}),
        ],
      }),
      // Celda derecha — tipo + fecha
      new TableCell({
        shading: { fill: C.light, type: ShadingType.CLEAR },
        margins: { top: 220, bottom: 220, left: 400, right: 280 },
        verticalAlign: VerticalAlign.CENTER,
        borders: { top: { style: BorderStyle.NONE }, bottom: { style: BorderStyle.NONE },
                   left: { style: BorderStyle.NONE }, right: { style: BorderStyle.NONE } },
        children: [
          new Paragraph({ alignment: AlignmentType.RIGHT, spacing: { before: 0, after: 0 }, children: [
            new TextRun({ text: label.toUpperCase(), font: "Arial", size: 18,
              bold: true, color: C.orange, characterSpacing: 80 }),
          ]}),
          new Paragraph({ alignment: AlignmentType.RIGHT, spacing: { before: 0, after: 0 }, children: [
            new TextRun({ text: docDate, font: "Arial", size: 16, color: C.grey }),
          ]}),
        ],
      }),
    ]})],
  }),

  spacer(40),
  hrBar(C.blue, 10),
  spacer(20),

  // Título principal
  new Paragraph({
    spacing: { before: 240, after: 60 },
    children: [new TextRun({
      text: docTitle, font: "Georgia", size: 52, bold: true, color: C.navy,
    })],
  }),
  hrBar(C.orange, 6),
  spacer(60),
];

// ── Encabezado de página (p. 2+) ─────────────────────────────────────────────

const pageHeader = new Header({
  children: [new Paragraph({
    border: { bottom: { style: BorderStyle.SINGLE, size: 2, color: "CCCCCC", space: 6 } },
    tabStops: [{ type: TabStopType.RIGHT, position: 9026 }],
    children: [
      new TextRun({ text: "OLIV4600", font: "Arial", size: 16, bold: true, color: C.navy }),
      new TextRun({ text: "\t" }),
      new TextRun({ text: label, font: "Arial", size: 16, color: C.grey }),
    ],
  })],
});

// ── Pie de página ─────────────────────────────────────────────────────────────

const pageFooter = new Footer({
  children: [new Paragraph({
    border: { top: { style: BorderStyle.SINGLE, size: 2, color: "CCCCCC", space: 6 } },
    tabStops: [{ type: TabStopType.RIGHT, position: 9026 }],
    children: [
      new TextRun({ text: "OLIV4600 · Sovereign Intelligence · 100% Offline",
                    font: "Arial", size: 16, color: C.grey }),
      new TextRun({ text: "\t" }),
      new TextRun({ text: "Página ", font: "Arial", size: 16, color: C.grey }),
      new TextRun({ children: [PageNumber.CURRENT], font: "Arial", size: 16, color: C.grey }),
      new TextRun({ text: " de ", font: "Arial", size: 16, color: C.grey }),
      new TextRun({ children: [PageNumber.TOTAL_PAGES], font: "Arial", size: 16, color: C.grey }),
    ],
  })],
});

// ── Documento completo ────────────────────────────────────────────────────────

const doc = new Document({
  title:   docTitle,
  subject: label,
  creator: "OLIV4600",

  styles: {
    default: {
      document: { run: { font: "Georgia", size: 22, color: C.black } },
    },
  },

  numbering: {
    config: [
      {
        reference: "oliv-bullets",
        levels: [{
          level: 0, format: LevelFormat.BULLET, text: "•",
          alignment: AlignmentType.LEFT,
          style: {
            paragraph: { indent: { left: 720, hanging: 360 } },
            run: { font: "Symbol", color: C.orange },
          },
        }],
      },
      {
        reference: "oliv-numbers",
        levels: [{
          level: 0, format: LevelFormat.DECIMAL, text: "%1.",
          alignment: AlignmentType.LEFT,
          style: {
            paragraph: { indent: { left: 720, hanging: 360 } },
            run: { bold: true, color: C.navy },
          },
        }],
      },
    ],
  },

  sections: [{
    properties: {
      page: {
        size: { width: 11906, height: 16838 }, // A4
        margin: { top: 1134, right: 1134, bottom: 1134, left: 1134 }, // ~2 cm
      },
    },
    headers: { default: pageHeader },
    footers: { default: pageFooter },
    children: [...coverBlock, ...bodyParagraphs(text)],
  }],
});

// ── Escribir ──────────────────────────────────────────────────────────────────

Packer.toBuffer(doc)
  .then(buf => { fs.writeFileSync(outputFile, buf); process.exit(0); })
  .catch(err => { console.error("Error generando DOCX:", err); process.exit(1); });
