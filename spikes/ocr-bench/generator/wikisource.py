"""Wikisource and Commons: a pinned page's proofread text, and its scan.

The text of a `Page:` is taken from the page as MediaWiki renders it
(`action=parse&oldid=…`), not from its wikitext, so templates are
expanded by Wikisource itself and nothing has to imitate them. The
rendered page holds what is printed on the page: the running head, the
body and the page number, which every engine also sees.
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
import time
import urllib.parse
import urllib.request
from html.parser import HTMLParser
from pathlib import Path

WIKISOURCE = "https://en.wikisource.org/w/api.php"
COMMONS = "https://commons.wikimedia.org/w/api.php"
USER_AGENT = "atlas-ocr-corpus/0.1 (OCR spike; one-time pinned fetch)"
PAUSE = 5.0


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class Fetcher:
    """GETs with a cache keyed by URL, a pause between requests to every
    host, and backoff on 429 and 5xx."""

    def __init__(self, cache: Path):
        self.cache = cache
        self.cache.mkdir(parents=True, exist_ok=True)

    def get(self, url: str) -> bytes:
        cached = self.cache / hashlib.sha256(url.encode()).hexdigest()
        if cached.exists():
            return cached.read_bytes()
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        for attempt in range(6):
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    data = r.read()
                cached.write_bytes(data)
                time.sleep(PAUSE)
                return data
            except Exception as e:  # noqa: BLE001
                if attempt == 5:
                    raise
                wait = 30 * (attempt + 1)
                print(f"  retry in {wait}s: {url[:100]}: {e}", file=sys.stderr)
                time.sleep(wait)
        raise RuntimeError("unreachable")

    def api(self, base: str, **params) -> dict:
        query = urllib.parse.urlencode({"format": "json", "formatversion": "2", **params})
        return json.loads(self.get(f"{base}?{query}"))


# ------------------------------------------------------------------ text

BLOCKS = {"p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "table", "dd", "dt"}
CELLS = {"td", "th"}


class _Text(HTMLParser):
    """Rendered page HTML to plain text: blocks on lines of their own,
    table cells side by side, styles, reference marks and the quality
    banner left out."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.out: list[str] = []
        self.skip = 0

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        cls = a.get("class") or ""
        if self.skip:
            if tag not in ("br", "img", "hr", "link", "meta", "wbr"):
                self.skip += 1
            return
        if tag in ("style", "script") or "prp-page-qualityheader" in cls or "reference" in cls.split() or "mw-cite-backlink" in cls:
            self.skip = 1
            return
        if tag in BLOCKS:
            self.out.append("\n")
        elif tag in CELLS:
            self.out.append(" ")

    def handle_endtag(self, tag):
        if self.skip:
            self.skip -= 1
            return
        if tag in BLOCKS:
            self.out.append("\n")

    def handle_startendtag(self, tag, attrs):
        if not self.skip and tag in ("br", "hr"):
            self.out.append("\n")

    def handle_data(self, data):
        if not self.skip:
            self.out.append(data)


def html_to_text(html: str) -> str:
    """Plain text, one line per printed line or block, no blank runs."""
    parser = _Text()
    parser.feed(html)
    text = "".join(parser.out).replace(" ", " ")
    lines = [re.sub(r"[ \t]+", " ", line).strip() for line in text.split("\n")]
    out: list[str] = []
    for line in lines:
        if line or (out and out[-1]):
            out.append(line)
    return "\n".join(out).strip() + "\n"


def page_title(file: str, page: int) -> str:
    return f"Page:{file}/{page}"


def latest_revision(f: Fetcher, file: str, page: int) -> dict:
    """The page's current revision, with its quality (4 = validated)."""
    d = f.api(
        WIKISOURCE,
        action="query",
        titles=page_title(file, page),
        prop="revisions|proofread",
        rvprop="ids|sha1",
    )
    p = d["query"]["pages"][0]
    rev = p["revisions"][0]
    return {"revid": rev["revid"], "revSha1": rev["sha1"], "quality": p.get("proofread", {}).get("quality")}


def revision_text(f: Fetcher, revid: int) -> str:
    """The rendered text of one pinned revision."""
    d = f.api(WIKISOURCE, action="parse", oldid=revid, prop="text", disablelimitreport=1, disableeditsection=1)
    return html_to_text(d["parse"]["text"])


def validated_pages(f: Fetcher, file: str) -> list[int]:
    out, cont = [], {}
    while True:
        d = f.api(
            WIKISOURCE,
            action="query",
            generator="allpages",
            gapnamespace=104,
            gapprefix=file + "/",
            gaplimit=500,
            prop="proofread",
            **cont,
        )
        for p in d.get("query", {}).get("pages", []):
            if p.get("proofread", {}).get("quality") == 4:
                out.append(int(p["title"].rsplit("/", 1)[1]))
        if "continue" not in d:
            return sorted(out)
        cont = d["continue"]


# ------------------------------------------------------------------ scans


def file_pages(f: Fetcher, file: str) -> dict:
    """Per page of a DjVu on Commons: size, resolution, and the file's own
    hidden OCR text. Also the file's sha1."""
    d = f.api(COMMONS, action="query", titles="File:" + file, prop="imageinfo", iiprop="sha1|size|metadata", iimetadataversion="latest")
    info = d["query"]["pages"][0]["imageinfo"][0]
    meta = {m["name"]: m["value"] for m in info["metadata"]}
    pages = {}
    for entry in meta["data"][0]["value"] if meta["data"] and meta["data"][0]["name"] == "pages" else []:
        values = {v["name"]: v["value"] for v in entry["value"]}
        pages[entry["name"] + 1] = {"width": values["width"], "height": values["height"], "dpi": values.get("dpi")}
    for entry in meta.get("text", []):
        pages.setdefault(entry["name"] + 1, {})["layer"] = entry["value"]
    return {"sha1": info["sha1"], "pages": pages}


def scan_url(f: Fetcher, file: str, page: int, width: int) -> str:
    """Commons' JPEG of one DjVu page at its full width."""
    d = f.api(COMMONS, action="query", titles="File:" + file, prop="imageinfo", iiprop="url", iiurlparam=f"page{page}-{width}px")
    url = d["query"]["pages"][0]["imageinfo"][0]["thumburl"]
    return url.split("?")[0]
