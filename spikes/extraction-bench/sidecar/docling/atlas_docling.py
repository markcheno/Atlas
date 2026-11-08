"""Docling as a standalone extractor for the Slice 0.9 spike.

Same contract as the Rust candidates (crates/common): files on the command
line, one JSON line per file to $XBENCH_OUT (else stdout), then a "done"
line. With `--serve`, file paths are read from stdin, one per line, and a
result line is written for each, so an app could keep one warm process.

The layout and table models load before the first file (the cold start).
CPU only, no OCR: the comparison is with text-layer extractors. PDFs and
EPUBs both go through the same converter.
Models come from the frozen bundle (`models/`), $DOCLING_ARTIFACTS, or
build/models next to this file.
"""

import json
import os
import sys
import time
from pathlib import Path

started = time.perf_counter()


def peak_rss_bytes() -> int:
    if sys.platform == "win32":
        import ctypes
        from ctypes import wintypes

        class Counters(ctypes.Structure):
            _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
                        ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
                        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                        ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t)]

        c = Counters()
        c.cb = ctypes.sizeof(Counters)
        ctypes.windll.psapi.GetProcessMemoryInfo(ctypes.windll.kernel32.GetCurrentProcess(), ctypes.byref(c), c.cb)
        return int(c.PeakWorkingSetSize)
    import resource

    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return int(rss if sys.platform == "darwin" else rss * 1024)


def artifacts() -> Path:
    if getattr(sys, "frozen", False):
        return Path(getattr(sys, "_MEIPASS")) / "models"
    if os.environ.get("DOCLING_ARTIFACTS"):
        return Path(os.environ["DOCLING_ARTIFACTS"])
    return Path(__file__).resolve().parent / "build" / "models"


def converter():
    from docling.datamodel.accelerator_options import AcceleratorDevice, AcceleratorOptions
    from docling.datamodel.base_models import InputFormat
    from docling.datamodel.pipeline_options import PdfPipelineOptions
    from docling.document_converter import DocumentConverter, PdfFormatOption

    opts = PdfPipelineOptions(
        artifacts_path=str(artifacts()),
        do_ocr=False,
        do_table_structure=True,
        accelerator_options=AcceleratorOptions(device=AcceleratorDevice.CPU),
    )
    conv = DocumentConverter(format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=opts)})
    conv.initialize_pipeline(InputFormat.PDF)
    return conv


def extract(conv, path: str) -> dict:
    from docling.datamodel.base_models import ConversionStatus

    t = time.perf_counter()
    try:
        result = conv.convert(path, raises_on_error=False)
    except Exception as e:  # noqa: BLE001 - classify everything
        msg = f"{type(e).__name__}: {e}"
        kind = "encrypted" if any(w in msg.lower() for w in ("password", "encrypt")) else "malformed"
        return {"file": path, "outcome": kind, "error": msg[:500], "ms": (time.perf_counter() - t) * 1e3}
    ms = (time.perf_counter() - t) * 1e3
    if result.status not in (ConversionStatus.SUCCESS, ConversionStatus.PARTIAL_SUCCESS):
        msg = "; ".join(str(e.error_message) for e in result.errors)[:500] or str(result.status)
        kind = "encrypted" if any(w in msg.lower() for w in ("password", "encrypt")) else "malformed"
        return {"file": path, "outcome": kind, "error": msg, "ms": ms}
    doc = result.document
    # Paged formats (PDF) export per page; EPUB and other flowing formats
    # have no pages, so the whole text is one "page".
    pages = [doc.export_to_text(page_no=p) for p in sorted(doc.pages)] if doc.pages else [doc.export_to_text()]
    return {"file": path, "outcome": "ok", "pages": pages, "ms": ms,
            "meta": {"pageCount": len(pages), "status": str(result.status)}}


def main() -> None:
    files = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = open(os.environ["XBENCH_OUT"], "w", encoding="utf-8") if os.environ.get("XBENCH_OUT") else sys.stdout
    conv = converter()
    ready_ms = (time.perf_counter() - started) * 1e3

    def emit(obj: dict) -> None:
        out.write(json.dumps(obj, ensure_ascii=False) + "\n")
        out.flush()

    if "--serve" in sys.argv:
        emit({"ready": True, "readyMs": ready_ms})
        for line in sys.stdin:
            if line.strip():
                emit(extract(conv, line.strip()))
    else:
        for f in files:
            emit(extract(conv, f))
    import docling

    emit({"done": True, "extractor": "docling", "version": f"docling {getattr(docling, '__version__', '2.149.0')} (CPU, no OCR)",
          "readyMs": ready_ms, "peakRssBytes": peak_rss_bytes()})


if __name__ == "__main__":
    # Frozen builds re-launch this executable for multiprocessing helpers
    # (`-c "from multiprocessing.resource_tracker import main…"`); this hands
    # those launches to multiprocessing instead of treating them as files.
    import multiprocessing

    multiprocessing.freeze_support()
    main()
