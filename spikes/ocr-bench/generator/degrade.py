"""Seeded degradations of a clean scan, each a recipe in the manifest.

Every operation takes its randomness from one seeded generator, so a
recipe and a seed give the same pixels on the machine that made them.
Pillow's resampling isn't promised to be identical across versions or
platforms, so CI builds the corpus once and every OS reads that build.
"""

from __future__ import annotations

import io

import numpy as np
from PIL import Image, ImageFilter

RECIPES = ("lowres150", "lowres100", "skew", "speckle", "jpeg", "blur", "binarised", "combined")


def _speckle(im: Image.Image, rng: np.random.Generator, share: float) -> Image.Image:
    a = np.asarray(im, dtype=np.int16).copy()
    mask = rng.random(a.shape) < share
    a[mask] = np.where(rng.random(int(mask.sum())) < 0.5, 0, 255)
    return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8), "L")


def _jpeg(im: Image.Image, quality: int) -> Image.Image:
    buf = io.BytesIO()
    im.save(buf, "JPEG", quality=quality)
    return Image.open(io.BytesIO(buf.getvalue())).convert("L")


def _down(im: Image.Image, ppi: float, to: float) -> tuple[Image.Image, float]:
    if ppi <= to:
        return im, ppi
    scale = to / ppi
    size = (max(1, round(im.width * scale)), max(1, round(im.height * scale)))
    return im.resize(size, Image.Resampling.BILINEAR), to


def degrade(im: Image.Image, ppi: float, recipe: str, seed: int) -> tuple[Image.Image, float]:
    """The degraded page in greyscale, and its resolution."""
    rng = np.random.default_rng(seed)
    im = im.convert("L")
    if recipe == "lowres150":
        return _down(im, ppi, 150)
    if recipe == "lowres100":
        return _down(im, ppi, 100)
    if recipe == "skew":
        angle = float(rng.uniform(2.0, 3.0)) * (1 if rng.random() < 0.5 else -1)
        return im.rotate(angle, resample=Image.Resampling.BICUBIC, expand=True, fillcolor=255), ppi
    if recipe == "speckle":
        return _speckle(im, rng, 0.004), ppi
    if recipe == "jpeg":
        return _jpeg(im, int(rng.integers(20, 31))), ppi
    if recipe == "blur":
        return im.filter(ImageFilter.GaussianBlur(radius=1.6 * ppi / 300)), ppi
    if recipe == "binarised":
        a = np.asarray(im)
        return Image.fromarray(np.where(a < 160, 0, 255).astype(np.uint8), "L"), ppi
    if recipe == "combined":
        im, ppi = _down(im, ppi, 150)
        im = im.rotate(float(rng.uniform(-1.5, 1.5)), resample=Image.Resampling.BICUBIC, expand=True, fillcolor=255)
        im = _speckle(im, rng, 0.002)
        return _jpeg(im, 35), ppi
    raise ValueError(f"unknown recipe {recipe}")


def to_jpeg(im: Image.Image, quality: int = 92) -> bytes:
    buf = io.BytesIO()
    im.convert("L").save(buf, "JPEG", quality=quality)
    return buf.getvalue()
