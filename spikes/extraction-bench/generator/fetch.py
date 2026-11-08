"""Fetches the redistributable part of the spike corpus (docs/spike-0.9.md).

    uv run fetch.py            # download into ../corpus and update the manifest

Each file is recorded in corpus/manifest.json with its source URL, license
and SHA-256. Only public-domain or CC BY 4.0 material is listed here, and
Project Gutenberg files are kept byte-for-byte as published (with their
license text). Run once; the results are committed, so benchmark runs and
CI need no network.
"""

import hashlib
import html
import json
import re
import sys
import time
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

HERE = Path(__file__).resolve().parent
CORPUS = HERE.parent / "corpus"
CACHE = HERE / "build" / "cache"

GUTENBERG = "Public domain in the USA; distributed unmodified under the Project Gutenberg License included in the file"
US_GOV = "Public domain: work of the U.S. Government (17 U.S.C. § 105)"
CC_BY = "CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), as stated on the arXiv abstract page"

# (corpus path, URL, license, note)
GUTENBERG_BOOKS = [
    (11, "Alice's Adventures in Wonderland (English prose, images)"),
    (1065, "The Raven (poetry, illustrations)"),
    (4650, "Candide (French, accented text)"),
    (7337, "道德經 / Tao Te Ching (Chinese)"),
    (1232, "The Prince (notes)"),
]

FEDERAL_REGISTER = [
    ("2026-22817", "2026/11/09", "Airworthiness Directives; The Boeing Company Airplanes (3 pages, three columns)"),
    ("2026-22772", "2026/11/09", "Single Family Housing Guaranteed Loan Program (3 pages, three columns)"),
]

IA_SCAN = {
    "identifier": "alicesadventures00carrrich",
    "pages": [20, 21, 22, 23],
    "note": "Alice's Adventures in Wonderland, New York: Lovell, Coryell & Co., ca. 1894; scan by University of California Libraries via Internet Archive",
    "license": "Public domain (published ca. 1894; Internet Archive possible-copyright-status NOT_IN_COPYRIGHT)",
}


def get(url: str) -> bytes:
    CACHE.mkdir(parents=True, exist_ok=True)
    cached = CACHE / hashlib.sha256(url.encode()).hexdigest()
    if cached.exists():
        return cached.read_bytes()
    req = urllib.request.Request(url, headers={"User-Agent": "atlas-spike-corpus/0.1 (one-time fetch)"})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                data = r.read()
            cached.write_bytes(data)
            time.sleep(1.0)  # be polite to every host
            return data
        except Exception as e:  # noqa: BLE001
            if attempt == 3:
                raise
            print(f"  retry {url}: {e}", file=sys.stderr)
            time.sleep(3 * (attempt + 1))
    raise RuntimeError("unreachable")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_manifest() -> dict:
    path = CORPUS / "manifest.json"
    if path.exists():
        return json.loads(path.read_text())
    return {"version": 1, "files": {}, "documents": []}


def save_manifest(m: dict) -> None:
    m["documents"].sort(key=lambda d: d["id"])
    m["files"] = dict(sorted(m["files"].items()))
    (CORPUS / "manifest.json").write_text(json.dumps(m, indent=2, ensure_ascii=False) + "\n")


def put(m: dict, rel: str, data: bytes, source: str, license_: str, note: str = "") -> None:
    path = CORPUS / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    m["files"][rel] = {"sha256": sha256(data), "bytes": len(data), "source": source, "license": license_, "note": note}


def document(m: dict, doc: dict) -> None:
    m["documents"] = [d for d in m["documents"] if d["id"] != doc["id"]] + [doc]


class ArticleText(HTMLParser):
    """Visible text of an arXiv HTML (LaTeXML) article, in document order.

    Math is replaced by its alttext; figures' images, scripts, styles and
    navigation are skipped. Block elements end with a newline.
    """

    BLOCK = {"p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "figcaption", "section", "td", "th", "br", "dd", "dt"}
    SKIP = {"script", "style", "nav", "header", "footer", "button", "svg"}

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.out: list[str] = []
        self.skip = 0
        self.math = 0
        self.inside = False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "article":
            self.inside = True
        if not self.inside:
            return
        if tag in self.SKIP or "ltx_page_footer" in (a.get("class") or ""):
            self.skip += 1
        elif tag == "math":
            self.math += 1
            if self.skip == 0 and a.get("alttext"):
                self.out.append(" " + a["alttext"] + " ")
        elif tag in self.BLOCK:
            self.out.append("\n")

    def handle_endtag(self, tag):
        if tag == "article":
            self.inside = False
        if not self.inside:
            return
        if tag in self.SKIP and self.skip:
            self.skip -= 1
        elif tag == "math" and self.math:
            self.math -= 1
        elif tag in self.BLOCK:
            self.out.append("\n")

    def handle_data(self, data):
        if self.inside and self.skip == 0 and self.math == 0:
            self.out.append(data)

    def text(self) -> str:
        t = html.unescape("".join(self.out))
        t = re.sub(r"[ \t ]+", " ", t)
        return re.sub(r"\n\s*\n+", "\n\n", t).strip() + "\n"


def fetch_gutenberg(m: dict) -> None:
    for num, note in GUTENBERG_BOOKS:
        epub_url = f"https://www.gutenberg.org/cache/epub/{num}/pg{num}-images-3.epub"
        txt_url = f"https://www.gutenberg.org/cache/epub/{num}/pg{num}.txt"
        print(f"gutenberg {num}")
        put(m, f"epub/pg{num}.epub", get(epub_url), epub_url, GUTENBERG, note)
        put(m, f"text/pg{num}.txt", get(txt_url), txt_url, GUTENBERG, f"plain-text edition of {note}")
        document(m, {
            "id": f"epub-pg{num}", "path": f"epub/pg{num}.epub", "kind": "epub", "expect": "text",
            "truth": {"path": f"text/pg{num}.txt", "kind": "gutenberg"}, "note": note,
        })


def fetch_federal_register(m: dict) -> None:
    for doc_no, date, note in FEDERAL_REGISTER:
        y, mo, d = date.split("/")
        pdf_url = f"https://www.govinfo.gov/content/pkg/FR-{y}-{mo}-{d}/pdf/{doc_no}.pdf"
        txt_url = f"https://www.federalregister.gov/documents/full_text/text/{date}/{doc_no}.txt"
        print(f"federal register {doc_no}")
        put(m, f"pdf/real/fr-{doc_no}.pdf", get(pdf_url), pdf_url, US_GOV, note)
        page = get(txt_url).decode("utf-8", "replace")
        pre = re.search(r"<pre>([\s\S]*?)</pre>", page)
        text = html.unescape(re.sub(r"<[^>]+>", "", pre.group(1) if pre else page))
        put(m, f"truth/fr-{doc_no}.txt", text.encode(), txt_url, US_GOV,
            "Federal Register plain text of the same document (the <pre> block, unescaped)")
        document(m, {
            "id": f"pdf-fr-{doc_no}", "path": f"pdf/real/fr-{doc_no}.pdf", "kind": "pdf-real", "expect": "text",
            "layout": "three-column", "truth": {"path": f"truth/fr-{doc_no}.txt", "kind": "reference"}, "note": note,
        })


def fetch_arxiv(m: dict, ids: list[str]) -> None:
    for arxiv_id in ids:
        abs_page = get(f"https://arxiv.org/abs/{arxiv_id}").decode("utf-8", "replace")
        lic = re.search(r"creativecommons\.org/licenses/[a-z-]+/[0-9.]+|arxiv\.org/licenses/[a-z-]+/[0-9.]+", abs_page)
        if not lic or lic.group(0) != "creativecommons.org/licenses/by/4.0":
            raise SystemExit(f"{arxiv_id}: not CC BY 4.0 ({lic.group(0) if lic else 'no license link'})")
        title = html.unescape(re.search(r'<meta name="citation_title" content="([^"]+)"', abs_page).group(1))
        pdf_url = f"https://arxiv.org/pdf/{arxiv_id}"
        html_url = f"https://arxiv.org/html/{arxiv_id}"
        print(f"arxiv {arxiv_id}: {title[:60]}")
        safe = arxiv_id.replace("/", "_")
        put(m, f"pdf/real/arxiv-{safe}.pdf", get(pdf_url), pdf_url, CC_BY, title)
        parser = ArticleText()
        parser.feed(get(html_url).decode("utf-8", "replace"))
        put(m, f"truth/arxiv-{safe}.txt", parser.text().encode(), html_url, CC_BY,
            "visible text of arXiv's HTML rendering (math as LaTeX alt text)")
        document(m, {
            "id": f"pdf-arxiv-{safe}", "path": f"pdf/real/arxiv-{safe}.pdf", "kind": "pdf-real", "expect": "text",
            "layout": "paper", "truth": {"path": f"truth/arxiv-{safe}.txt", "kind": "reference"}, "note": title,
        })


def fetch_scan(m: dict) -> None:
    # Page images only; make.py turns them into an image-only PDF.
    out = HERE / "build" / "scan"
    out.mkdir(parents=True, exist_ok=True)
    for n in IA_SCAN["pages"]:
        url = f"https://archive.org/download/{IA_SCAN['identifier']}/page/n{n}_w1000.jpg"
        (out / f"n{n:03}.jpg").write_bytes(get(url))
    m.setdefault("sources", {})["ia-scan"] = {
        "urls": [f"https://archive.org/download/{IA_SCAN['identifier']}/page/n{n}_w1000.jpg" for n in IA_SCAN["pages"]],
        "item": f"https://archive.org/details/{IA_SCAN['identifier']}",
        "license": IA_SCAN["license"],
        "note": IA_SCAN["note"],
    }


if __name__ == "__main__":
    manifest = load_manifest()
    fetch_gutenberg(manifest)
    fetch_federal_register(manifest)
    fetch_scan(manifest)
    arxiv = [a for a in sys.argv[1:] if not a.startswith("-")]
    if arxiv:
        fetch_arxiv(manifest, arxiv)
    save_manifest(manifest)
    print(f"manifest: {len(manifest['files'])} files, {len(manifest['documents'])} documents")
