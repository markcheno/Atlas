"""The OCR spike's corpus (docs/spike-ocr.md, ADR 0020).

    uv run corpus.py select   # once, by the spike's author: chooses the pages
                              # and pins every revision and file in
                              # ../corpus/manifest.json
    uv run corpus.py make     # fetches what the manifest pins, checks every
                              # hash, and writes ../build/corpus/
    uv run corpus.py check    # the built corpus against the manifest
    uv run corpus.py check-page   # the installed-layout check's page (committed)

Reference text from Wikisource is CC BY-SA 4.0, so it is fetched, never
committed; what is committed is the manifest of pins and our own pages.
Every page becomes a one-page image-only PDF that the engines render
with PDFium, as the worker will.

Categories (ADR 0020): `book` (20), `two-column` (15), `degraded` (10),
`poor-layer` (5). A page appears in one category only.
"""

from __future__ import annotations

import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageFilter

import degrade as deg
import pdf
import wikisource as ws

HERE = Path(__file__).resolve().parent
BENCH = HERE.parent
MANIFEST = BENCH / "corpus" / "manifest.json"
OWN = BENCH / "corpus"
BUILD = BENCH / "build"
OUT = BUILD / "corpus"
SPIKES = BENCH.parent
XBENCH_CORPUS = SPIKES / "extraction-bench" / "corpus"

BOOKS = [
    ("Carroll - Alice's Adventures in Wonderland.djvu", "Lewis Carroll, Alice's Adventures in Wonderland (Cassell, 1907)"),
    ("The Time Machine (H. G. Wells, William Heinemann, 1895).djvu", "H. G. Wells, The Time Machine (Heinemann, 1895)"),
    ("Stevenson - Treasure Island.djvu", "R. L. Stevenson, Treasure Island"),
    ("The Jungle Book.djvu", "Rudyard Kipling, The Jungle Book"),
    ("Wind in the Willows (1913).djvu", "Kenneth Grahame, The Wind in the Willows (1913)"),
]
# Two-column pages: the 1911 Encyclopaedia Britannica.
COLUMNS = [
    ("EB1911 - Volume 03.djvu", "Encyclopaedia Britannica, 11th ed., vol. 3 (1910)"),
    ("EB1911 - Volume 10.djvu", "Encyclopaedia Britannica, 11th ed., vol. 10 (1910)"),
]
PER_BOOK = 4
DEGRADED_FROM_BOOKS = [2, 2, 2, 2, 2]
POOR_LAYER = 5
POOR_LAYER_WER = 0.10
COLUMN_PAGES = 5  # per volume
OWN_COLUMN_PAGES = 5
MIN_BOOK_CHARS = 700
MIN_COLUMN_CHARS = 3000
SCAN_LICENCE = "Public domain (Wikimedia Commons: the scan of a work published before 1929)"
TEXT_LICENCE = "CC BY-SA 4.0 (Wikisource contributors); fetched at build time, not committed"
OWN_LICENCE = "Generated for Atlas from public-domain text (Project Gutenberg); no rights reserved"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def words(text: str) -> list[str]:
    return re.findall(r"[0-9a-z]+", text.lower())


def wer(hypothesis: str, reference: str) -> float:
    """Word error rate, for choosing poor text layers (the scorer's own
    measure decides everything else)."""
    h, r = words(hypothesis), words(reference)
    if not r:
        return 0.0 if not h else 1.0
    prev = list(range(len(r) + 1))
    for i, x in enumerate(h, 1):
        cur = [i] + [0] * len(r)
        for j, y in enumerate(r, 1):
            cur[j] = min(prev[j - 1] + (x != y), prev[j] + 1, cur[j - 1] + 1)
        prev = cur
    return prev[-1] / len(r)


def spread(items: list[int], n: int) -> list[int]:
    """`n` items evenly spaced through `items`."""
    if n >= len(items):
        return list(items)
    return [items[round(i * (len(items) - 1) / (n - 1))] if n > 1 else items[len(items) // 2] for i in range(n)]


# ------------------------------------------------------------------ select


def candidates(f: ws.Fetcher, file: str, min_chars: int, want: int, used: set[int]):
    """Validated body pages of `file` with at least `min_chars` of text,
    walked from evenly spaced starting points, as (page, revision, text)."""
    pages = ws.validated_pages(f, file)
    body = pages[len(pages) // 10 : len(pages) - len(pages) // 20]
    for page in spread(body, max(want * 4, 1)):
        if page in used:
            continue
        rev = ws.latest_revision(f, file, page)
        if rev["quality"] != 4:
            continue
        text = ws.revision_text(f, rev["revid"])
        if len(text) < min_chars:
            continue
        used.add(page)
        yield page, rev, text


def scan_entry(f: ws.Fetcher, file: str, page: int, meta: dict) -> dict:
    info = meta["pages"][page]
    url = ws.scan_url(f, file, page, info["width"])
    jpeg = f.get(url)
    w, h, _ = pdf.jpeg_size(jpeg)
    # Commons serves a page at a standard width at most the original's
    # (1920 px for most of these scans, 3840 for one), so the image's
    # resolution is the scan's scaled by the width served over the width
    # scanned. The PDF page is sized by it, so the page keeps its real size.
    scanned = info["dpi"] or 300
    ppi = round(scanned * w / info["width"], 2)
    return {"url": url, "sha256": sha256(jpeg), "bytes": len(jpeg), "width": w, "height": h,
            "ppi": ppi, "scannedPpi": scanned, "scannedWidth": info["width"]}


def wikisource_page(f, file, page, rev, text, meta, source, category, **extra) -> dict:
    return {
        "id": f"{category}-{re.sub(r'[^a-z0-9]+', '-', file.lower()).strip('-')[:28]}-p{page}",
        "category": category,
        "source": source,
        "scan": {"file": file, "page": page, "commonsSha1": meta["sha1"], **scan_entry(f, file, page, meta), "licence": SCAN_LICENCE},
        "truth": {"kind": "wikisource", "revid": rev["revid"], "revSha1": rev["revSha1"], "sha256": sha256(text.encode()), "chars": len(text), "licence": TEXT_LICENCE},
        **extra,
    }


def select() -> None:
    f = ws.Fetcher(BUILD / "cache")
    pages: list[dict] = []
    for (file, source), n_degraded in zip(BOOKS, DEGRADED_FROM_BOOKS):
        print(f"select: {source}", file=sys.stderr)
        meta = ws.file_pages(f, file)
        used: set[int] = set()
        found = candidates(f, file, MIN_BOOK_CHARS, PER_BOOK + n_degraded + 3, used)
        book = []
        for page, rev, text in found:
            book.append((page, rev, text))
            if len(book) == PER_BOOK + n_degraded:
                break
        for i, (page, rev, text) in enumerate(book):
            if i < PER_BOOK:
                pages.append(wikisource_page(f, file, page, rev, text, meta, source, "book"))
            else:
                recipe = deg.RECIPES[len([p for p in pages if p["category"] == "degraded"]) % len(deg.RECIPES)]
                pages.append(wikisource_page(f, file, page, rev, text, meta, source, "degraded",
                                             degrade={"recipe": recipe, "seed": 4200 + page}))
        # Pages whose DjVu carries a poor OCR layer.
        for page, rev, text in candidates(f, file, MIN_BOOK_CHARS, 6, used):
            layer = meta["pages"].get(page, {}).get("layer") or ""
            score = wer(layer, text)
            if score >= POOR_LAYER_WER:
                pages.append(wikisource_page(f, file, page, rev, text, meta, source, "poor-layer",
                                             layer={"sha256": sha256(layer.encode()), "selectionWer": round(score, 4)}))
    poor = [p for p in pages if p["category"] == "poor-layer"]
    poor.sort(key=lambda p: -p["layer"]["selectionWer"])
    for p in poor[POOR_LAYER:]:
        pages.remove(p)
    for file, source in COLUMNS:
        print(f"select: {source}", file=sys.stderr)
        meta = ws.file_pages(f, file)
        used: set[int] = set()
        n = 0
        for page, rev, text in candidates(f, file, MIN_COLUMN_CHARS, COLUMN_PAGES * 2, used):
            # Text articles only: no tables or formulae.
            if "|" in text or text.count("=") > 2:
                continue
            pages.append(wikisource_page(f, file, page, rev, text, meta, source, "two-column"))
            n += 1
            if n == COLUMN_PAGES:
                break
    manifest = {
        "version": 1,
        "comment": "The OCR spike's corpus (docs/spike-ocr.md). Selected by generator/corpus.py select; built by make.",
        "pages": pages,
        "own": own_pages(),
    }
    MANIFEST.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    counts = {}
    for p in pages + manifest["own"]:
        counts[p["category"]] = counts.get(p["category"], 0) + 1
    print(f"select: {counts}", file=sys.stderr)


# ------------------------------------------------------------- own pages


def own_pages() -> list[dict]:
    """Our own pages: the five two-column "papers", typeset here. (Slice
    0.9's synthetic scan isn't used: its reference text covers its three
    pages together, and every page here is scored on its own.)"""
    return [
        {"id": f"two-column-own-{i + 1}", "category": "two-column", "source": "typeset two-column paper (Project Gutenberg text)",
         "own": {"kind": "typst-paper", "index": i}, "licence": OWN_LICENCE}
        for i in range(OWN_COLUMN_PAGES)
    ]


def gutenberg_paragraphs(name: str) -> list[str]:
    text = (XBENCH_CORPUS / "text" / name).read_text(encoding="utf-8")
    start = text.index("\n", text.index("*** START OF")) + 1
    body = text[start : text.index("*** END OF")].replace("\r\n", "\n")
    paras = [re.sub(r"\s+", " ", p.replace("_", "")).strip() for p in re.split(r"\n\s*\n", body)]
    return [p for p in paras if p and not p.isupper()]


def typ(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def paper(index: int, paragraphs: int = 14) -> tuple[str, str]:
    """A two-column paper page: its typst source and its exact text, in
    reading order (head, title, abstract, the columns, footnote, folio).
    `paragraphs` is how many body paragraphs to try; [`paper_png`] drops
    them from the end until the page fits."""
    source = ("pg11.txt", "pg1232.txt")[index % 2]
    paras = [p for p in gutenberg_paragraphs(source) if len(p) >= 120 and not p.startswith("[")]
    start = 30 + index * 23
    abstract = paras[start]
    body = paras[start + 1 : start + 1 + paragraphs]
    title = ("On the Habits of Rabbits in Wonderland", "Observations on the Arts of Government",
             "A Study of Croquet with Flamingoes", "Notes on Princes and Their Counsellors",
             "The Trial of the Knave of Hearts")[index]
    head = f"Journal of Public-Domain Studies, vol. {index + 3}"
    folio = str(101 + index * 7)
    note = "This text is in the public domain; it was typeset for the Atlas OCR spike."
    src = f"""
#set page(width: 8.5in, height: 11in, margin: (x: 0.75in, top: 0.9in, bottom: 0.9in),
  header: align(center, text(size: 9pt, {typ(head)})),
  footer: align(center, text(size: 9pt, {typ(folio)})))
#set text(font: "Libertinus Serif", size: 10pt, hyphenate: false)
#set par(justify: false, leading: 0.6em, spacing: 0.9em)
#align(center, text(size: 16pt, weight: "bold", {typ(title)}))
#v(0.4em)
#block(inset: (x: 0.4in), text(size: 9pt, [#strong("Abstract.") #{typ(abstract)}]))
#v(0.6em)
#columns(2, gutter: 0.3in)[
{chr(10).join("#" + typ(p) + chr(10) for p in body)}
#text(size: 8pt, {typ(note)})
]
"""
    truth = "\n".join([head, title, "Abstract. " + abstract, *body, note, folio]) + "\n"
    return src, truth


def typeset_pngs(src: str, ppi: int) -> list[Image.Image]:
    tools = json.loads((SPIKES / ".tools" / "paths.json").read_text())
    with tempfile.TemporaryDirectory() as d:
        (Path(d) / "page.typ").write_text(src, encoding="utf-8")
        subprocess.run([tools["typst"]["bin"], "compile", "--format", "png", "--ppi", str(ppi), "page.typ", "page-{p}.png"],
                       cwd=d, check=True)
        files = sorted(Path(d).glob("page-*.png"), key=lambda p: int(p.stem.split("-")[1]))
        return [Image.open(p).convert("L") for p in files]


def paper_png(index: int, ppi: int = 300) -> tuple[Image.Image, str]:
    """The paper's page, filled but on one page, and its text."""
    for n in range(14, 0, -1):
        src, truth = paper(index, n)
        pages = typeset_pngs(src, ppi)
        if len(pages) == 1:
            return pages[0], truth
    raise RuntimeError("no paper fits on one page")


def print_and_scan(im: Image.Image, seed: int) -> Image.Image:
    """A light print-and-scan look: a small tilt, softness, some specks."""
    import numpy as np

    rng = np.random.default_rng(seed)
    im = im.rotate(float(rng.uniform(-0.5, 0.5)), resample=Image.Resampling.BICUBIC, fillcolor=255)
    im = im.filter(ImageFilter.GaussianBlur(0.5))
    a = np.asarray(im, dtype=np.int16).copy()
    a += rng.normal(0, 6, a.shape).astype(np.int16)
    return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8), "L")


# -------------------------------------------------------------------- make


def make() -> None:
    manifest = json.loads(MANIFEST.read_text())
    f = ws.Fetcher(BUILD / "cache")
    if OUT.exists():
        shutil.rmtree(OUT)
    for d in ("pages", "truth", "layer"):
        (OUT / d).mkdir(parents=True)
    jobs = []
    file_meta: dict[str, dict] = {}
    for p in manifest["pages"]:
        scan, truth = p["scan"], p["truth"]
        text = ws.revision_text(f, truth["revid"])
        if sha256(text.encode()) != truth["sha256"]:
            sys.exit(f"make: {p['id']}: the reference text changed (sha256); run select again and review")
        jpeg = f.get(scan["url"])
        if sha256(jpeg) != scan["sha256"]:
            sys.exit(f"make: {p['id']}: the scan changed (sha256 {sha256(jpeg)})")
        ppi = scan["ppi"]
        if "degrade" in p:
            im, ppi = deg.degrade(Image.open(__import__("io").BytesIO(jpeg)), ppi, p["degrade"]["recipe"], p["degrade"]["seed"])
            jpeg = deg.to_jpeg(im, 95)
        (OUT / "pages" / f"{p['id']}.pdf").write_bytes(pdf.image_pdf(jpeg, ppi))
        (OUT / "truth" / f"{p['id']}.txt").write_text(text, encoding="utf-8")
        if "layer" in p:
            meta = file_meta.setdefault(scan["file"], ws.file_pages(f, scan["file"]))
            layer = meta["pages"][scan["page"]].get("layer") or ""
            if sha256(layer.encode()) != p["layer"]["sha256"]:
                sys.exit(f"make: {p['id']}: the file's text layer changed")
            (OUT / "layer" / f"{p['id']}.txt").write_text(layer, encoding="utf-8")
        jobs.append({"id": p["id"], "pdf": f"pages/{p['id']}.pdf", "page": 1, "category": p["category"]})
    for p in manifest["own"]:
        own = p["own"]
        if own["kind"] == "typst-paper":
            im, truth = paper_png(own["index"])
            im = print_and_scan(im, 77 + own["index"])
            (OUT / "pages" / f"{p['id']}.pdf").write_bytes(pdf.image_pdf(deg.to_jpeg(im, 90), 300))
            (OUT / "truth" / f"{p['id']}.txt").write_text(truth, encoding="utf-8")
            jobs.append({"id": p["id"], "pdf": f"pages/{p['id']}.pdf", "page": 1, "category": p["category"]})
    with open(OUT / "jobs.jsonl", "w", encoding="utf-8") as out:
        for j in jobs:
            out.write(json.dumps(j, ensure_ascii=False) + "\n")
    counts: dict[str, int] = {}
    for j in jobs:
        counts[j["category"]] = counts.get(j["category"], 0) + 1
    print(f"make: {len(jobs)} pages in {OUT}: {counts}", file=sys.stderr)


def check() -> None:
    manifest = json.loads(MANIFEST.read_text())
    bad = []
    for p in manifest["pages"]:
        t = OUT / "truth" / f"{p['id']}.txt"
        if not (OUT / "pages" / f"{p['id']}.pdf").exists() or not t.exists() or sha256(t.read_bytes()) != p["truth"]["sha256"]:
            bad.append(p["id"])
    for p in manifest["own"]:
        if not (OUT / "pages" / f"{p['id']}.pdf").exists():
            bad.append(p["id"])
    if bad:
        sys.exit(f"check: {len(bad)} pages missing or changed: {bad[:5]}")
    print(f"check: {len(manifest['pages']) + len(manifest['own'])} pages match the manifest", file=sys.stderr)


def check_page() -> None:
    """The page every engine reads in the installed-layout check
    (`--self-check`): one column of Alice, typeset and given a light
    print-and-scan look, committed with its exact text (our own page)."""
    paras = [p for p in gutenberg_paragraphs("pg11.txt") if len(p) >= 120]
    for n in range(6, 0, -1):
        body = paras[60 : 60 + n]
        src = f"""
#set page(width: 6in, height: 8in, margin: 0.6in)
#set text(font: "Libertinus Serif", size: 11pt, hyphenate: false)
#set par(justify: false, leading: 0.6em, spacing: 0.9em)
{chr(10).join("#" + typ(p) + chr(10) for p in body)}
"""
        pages = typeset_pngs(src, 300)
        if len(pages) == 1:
            break
    im = print_and_scan(pages[0], 4242)
    out = OWN / "check"
    out.mkdir(parents=True, exist_ok=True)
    (out / "page.pdf").write_bytes(pdf.image_pdf(deg.to_jpeg(im, 80), 300))
    (out / "page.txt").write_text("\n".join(body) + "\n", encoding="utf-8")
    print(f"check-page: {len(body)} paragraphs, {(out / 'page.pdf').stat().st_size} bytes", file=sys.stderr)


if __name__ == "__main__":
    {"select": select, "make": make, "check": check, "check-page": check_page}[sys.argv[1] if len(sys.argv) > 1 else "make"]()
