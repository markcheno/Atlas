// Deterministic body text, multi-page PDFs and valid EPUBs for synthetic
// fixtures (phase1-spec §11). Node built-ins only. Everything is a pure
// function of its arguments, so the same arguments give the same bytes on
// every machine. EPUB entries are stored, not deflated, for that reason:
// Node's bundled zlib can produce different deflate output on different
// CPUs and versions.

// ------------------------------------------------------------------ text

/** mulberry32: a small, fast, well-distributed 32-bit PRNG. */
export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const WORDS = `about above across after again against almost along already also although
always among another answer around asked away back became because become before began
behind being below between beyond both brought built called came carried case certain
change clear close common could country course cover current daily deep different
distant during each early earth either enough even evening every example except far
field figure final find first follow form found free friend full further gave given
great ground group grown half hand hard heard heavy held help high hold home hour
however idea important inside instead itself just keep kind known land large last
later least left less light line little long look made main make many mark matter
mean measure might mind more morning most move much must near never next night north
nothing notice number often once only open order other outside over own paper part
pass past perhaps piece place plain point possible present probably problem quite
rather reach read ready real record remain rest right river road room round same
second seen several shall short should show side simple since small sound south
space stand start still stood story strong such sure surface table taken thing
those though thought through together toward true turn under until upon usual
very voice watch water week well went west whole whose wide winter within without
word work world write year young`
  .split(/\s+/)
  .filter(Boolean);

const TOPICS = {
  Astronomy: ['orbital resonance', 'spectral line survey', 'tidal locking'],
  Engineering: ['load bearing truss', 'thermal expansion joint', 'fatigue crack growth'],
  Mathematics: ['prime number sieve', 'continued fraction', 'graph colouring bound'],
  History: ['river trade route', 'coastal fortress', 'printing press guild'],
  Software: ['garbage collection pause', 'lock free queue', 'incremental compiler'],
  Biology: ['cell membrane transport', 'seed dispersal', 'migratory songbird'],
};
/** Shared by every file, so questions can span documents. */
const SHARED = ['quiet harbour lights', 'library of lost maps', 'café crème'];
/** Joins a phrase so line wrapping never splits it. */
const GLUE = '\u0001';
const glue = (phrase) => phrase.replaceAll(' ', GLUE);

export const marker = (subject, i) => `${subject} marker ${i}`;

/**
 * About `kb` KiB of pseudo-text for file `i`, as paragraphs. The first
 * sentence carries the file's unique marker phrase; topic and shared
 * phrases recur. WinAnsi characters only.
 */
export function bodyText(subject, i, kb) {
  const next = rng(crc32(Buffer.from(`${subject} ${i}`, 'utf8')));
  const pick = (list) => list[Math.floor(next() * list.length)];
  const topics = TOPICS[subject] ?? TOPICS.Astronomy;
  const target = Math.max(1, kb) * 1024;
  const paragraphs = [];
  let length = 0;
  let sentence = 0;
  while (length < target) {
    const sentences = [];
    const count = 3 + Math.floor(next() * 4);
    for (let s = 0; s < count && length < target; s++, sentence++) {
      const words = [];
      const n = 8 + Math.floor(next() * 9);
      for (let w = 0; w < n; w++) words.push(pick(WORDS));
      if (sentence === 0) words.splice(1, 0, glue(marker(subject, i)));
      else if (sentence % 5 === 0) words.splice(2, 0, glue(pick(topics)));
      else if (sentence % 11 === 0) words.splice(3, 0, glue(pick(SHARED)));
      words[0] = words[0][0].toUpperCase() + words[0].slice(1);
      const text = `${words.join(' ')}.`;
      sentences.push(text);
      length += text.length + 1;
    }
    paragraphs.push(sentences.join(' '));
  }
  return paragraphs;
}

/** The text as a reader sees it (glue removed). */
export const plain = (s) => s.replaceAll(GLUE, ' ');

// ------------------------------------------------------------------- PDF

const isLatin1 = (s) => [...s].every((c) => c.codePointAt(0) <= 0xff);

const LINE_CHARS = 90;
const LINES_PER_PAGE = 34; // about 3,000 characters a page

/** Wraps paragraphs into lines, with a blank line between paragraphs. */
function wrap(paragraphs) {
  const lines = [];
  paragraphs.forEach((p, n) => {
    if (n > 0) lines.push('');
    let line = '';
    for (const word of p.split(' ')) {
      if (line && line.length + 1 + word.length > LINE_CHARS) {
        lines.push(line);
        line = word;
      } else {
        line = line ? `${line} ${word}` : word;
      }
    }
    if (line) lines.push(line);
  });
  return lines.map(plain);
}

const pdfString = (s) => `(${s.replace(/[\\()]/g, (c) => `\\${c}`)})`;

/** A PDF text string for metadata: literal when Latin-1, else UTF-16BE hex. */
function pdfTextString(s) {
  if (isLatin1(s)) return pdfString(s);
  const units = Buffer.from(s, 'utf16le').swap16();
  return `<FEFF${units.toString('hex').toUpperCase()}>`;
}

/**
 * A multi-page PDF (Helvetica, WinAnsiEncoding) with an Info dictionary and
 * a correct cross-reference table. Returns the bytes and the page count.
 * `untitled` leaves the title and author out of the Info dictionary and
 * sets the title in 24 pt at the top of the first page instead (when it is
 * Latin-1), as many papers do.
 */
export function textPdf({ title, author, paragraphs, untitled = false }) {
  const lines = wrap(paragraphs);
  const pages = [];
  for (let p = 0; p < lines.length; p += LINES_PER_PAGE) {
    pages.push(lines.slice(p, p + LINES_PER_PAGE));
  }
  if (pages.length === 0) pages.push([]);

  // 1 catalog, 2 pages, 3 font, 4 info, then a page and its contents each.
  const pageObj = (n) => 5 + n * 2;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    `<< /Type /Pages /Kids [${pages.map((_, n) => `${pageObj(n)} 0 R`).join(' ')}] /Count ${pages.length} >>`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
    untitled
      ? '<< /Producer (Atlas fixture generator) >>'
      : `<< /Title ${pdfTextString(title)} /Author ${pdfTextString(author)} /Producer (Atlas fixture generator) >>`,
  ];
  const heading =
    untitled && isLatin1(title) ? `BT /F1 24 Tf 72 760 Td ${pdfString(title)} Tj ET\n` : '';
  pages.forEach((pageLines, n) => {
    const body = pageLines.map((l) => `${pdfString(l)} Tj T*`).join('\n');
    const stream = `${n === 0 ? heading : ''}BT /F1 10 Tf 14 TL 72 740 Td\n${body}\nET`;
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents ${pageObj(n) + 1} 0 R /Resources << /Font << /F1 3 0 R >> >> >>`,
      `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    );
  });

  // Every character is Latin-1, so string length is byte length.
  let body = '%PDF-1.4\n%âãÏÓ\n';
  const offsets = [];
  objects.forEach((obj, i) => {
    offsets.push(body.length);
    body += `${i + 1} 0 obj\n${obj}\nendobj\n`;
  });
  const xref = body.length;
  body += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const off of offsets) body += `${String(off).padStart(10, '0')} 00000 n \n`;
  body += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R /Info 4 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  if (!isLatin1(body)) throw new Error('PDF text must be Latin-1');
  return { bytes: Buffer.from(body, 'latin1'), pages: pages.length };
}

// ------------------------------------------------------------------ EPUB

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

export function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** A zip of stored entries, in order, with fixed 1980-01-01 timestamps. */
export function storedZip(entries) {
  const DOS_DATE = (1 << 5) | 1; // 1980-01-01
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const nameBytes = Buffer.from(name, 'utf8');
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0, 6); // flags
    local.writeUInt16LE(0, 8); // stored
    local.writeUInt16LE(0, 10); // time
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28); // no extra field
    locals.push(local, nameBytes, data);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4); // made by
    central.writeUInt16LE(20, 6); // needed
    central.writeUInt16LE(0, 8);
    central.writeUInt16LE(0, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(DOS_DATE, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt32LE(offset, 42);
    centrals.push(central, nameBytes);
    offset += local.length + nameBytes.length + data.length;
  }
  const cd = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, cd, end]);
}

const xml = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

const CONTAINER = `<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>
`;

const xhtml = (title, body) => `<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="en">
<head><title>${xml(title)}</title></head>
<body>
${body}
</body>
</html>
`;

/**
 * A valid EPUB 3: `mimetype` first and stored, container, OPF with
 * dc:identifier, dc:title, dc:creator and dc:language, a navigation
 * document, and the paragraphs split into `chapters` XHTML files.
 */
export function textEpub({ id, title, author, paragraphs, chapters = 4, untitled = false }) {
  const per = Math.ceil(paragraphs.length / chapters);
  const parts = [];
  for (let c = 0; c < chapters && c * per < paragraphs.length; c++) {
    parts.push(paragraphs.slice(c * per, (c + 1) * per));
  }
  const files = parts.map((ps, c) => {
    const heading = `Chapter ${c + 1}`;
    const body = [`<h1>${heading}</h1>`, ...ps.map((p) => `<p>${xml(plain(p))}</p>`)].join('\n');
    return [`OEBPS/chapter-${String(c + 1).padStart(2, '0')}.xhtml`, heading, body];
  });
  const nav = xhtml(
    title,
    `<nav epub:type="toc"><ol>\n${files
      .map(([href, heading]) => `<li><a href="${href.slice(6)}">${heading}</a></li>`)
      .join('\n')}\n</ol></nav>`,
  );
  const opf = `<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id" xml:lang="en">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">urn:atlas:fixture:${xml(id)}</dc:identifier>
${untitled ? '' : `    <dc:title>${xml(title)}</dc:title>\n`}    <dc:creator>${xml(author)}</dc:creator>
    <dc:language>en</dc:language>
    <meta property="dcterms:modified">2026-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
${files.map(([href], c) => `    <item id="c${c + 1}" href="${href.slice(6)}" media-type="application/xhtml+xml"/>`).join('\n')}
  </manifest>
  <spine>
${files.map((_, c) => `    <itemref idref="c${c + 1}"/>`).join('\n')}
  </spine>
</package>
`;
  const utf8 = (s) => Buffer.from(s, 'utf8');
  const bytes = storedZip([
    ['mimetype', Buffer.from('application/epub+zip', 'ascii')],
    ['META-INF/container.xml', utf8(CONTAINER)],
    ['OEBPS/content.opf', utf8(opf)],
    ['OEBPS/nav.xhtml', utf8(nav)],
    ...files.map(([href, heading, body]) => [href, utf8(xhtml(heading, body))]),
  ]);
  return { bytes, chapters: files.length };
}

const AUTHORS = ['Ada Lin', 'Bo Okafor', 'Cleo Varga', 'Dev Rao', 'Elif Sand', 'Femi Hale'];
export const authorFor = (i) => AUTHORS[i % AUTHORS.length];

/**
 * The PDF the extraction worker opens with PDFium before it says hello
 * (src-tauri/src/services/extraction/selftest.pdf): two pages, with the
 * marker "Selftest marker 0" on page 1. A test keeps the committed copy in
 * sync with this function.
 */
export const selfTestPdf = () =>
  textPdf({
    title: 'Atlas PDFium self-test',
    author: 'Atlas',
    paragraphs: bodyText('Selftest', 0, 4),
  });
