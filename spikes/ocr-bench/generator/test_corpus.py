"""Tests for the corpus generator: `uv run python -m unittest -v`."""

import io
import unittest

import numpy as np
from PIL import Image

import corpus
import degrade
import pdf
import wikisource


class HtmlToText(unittest.TestCase):
    def test_a_rendered_page_reads_as_printed(self):
        html = (
            '<div class="mw-parser-output"><div class="prp-page-qualityheader quality4">'
            'This page has been <a>validated</a>.</div><div class="pagetext">'
            "<style>.x{color:red}</style><hr/>"
            '<div class="wst-center"><p>DOWN THE RABBIT-HOLE\n</p></div>'
            "<p>once considering how in the world<sup class=\"reference\"><a>[1]</a></sup> she was\n</p>"
            "<p>The rabbit-hole went <i>straight</i>&nbsp;on.\n</p>"
            '<table><tr><td>4</td><td>ALICE</td></tr></table>'
            '<div class="wst-center"><p><span style="font-size:83%;">3</span></p></div></div></div>'
        )
        self.assertEqual(
            wikisource.html_to_text(html),
            "DOWN THE RABBIT-HOLE\n\nonce considering how in the world she was\n\n"
            "The rabbit-hole went straight on.\n\n4 ALICE\n\n3\n",
        )

    def test_nothing_after_a_skipped_block_is_lost(self):
        html = "<p>a<style>b</style>c</p><p>d</p>"
        self.assertEqual(wikisource.html_to_text(html), "ac\n\nd\n")


class ImagePdf(unittest.TestCase):
    def jpeg(self, w, h, mode="L"):
        buf = io.BytesIO()
        Image.new(mode, (w, h), 255).save(buf, "JPEG")
        return buf.getvalue()

    def test_the_jpeg_goes_in_unchanged_at_its_resolution(self):
        data = self.jpeg(800, 1200)
        self.assertEqual(pdf.jpeg_size(data), (800, 1200, 1))
        out = pdf.image_pdf(data, 400)
        self.assertIn(data, out)
        # 800 px at 400 ppi is 2 in, 144 pt.
        self.assertIn(b"/MediaBox [0 0 144.0000 216.0000]", out)
        self.assertIn(b"/ColorSpace /DeviceGray", out)
        self.assertTrue(out.rstrip().endswith(b"%%EOF"))

    def test_a_colour_scan_stays_colour(self):
        self.assertIn(b"/DeviceRGB", pdf.image_pdf(self.jpeg(10, 10, "RGB"), 300))

    def test_anything_else_is_refused(self):
        with self.assertRaises(ValueError):
            pdf.jpeg_size(b"\x89PNG....")


class Degrade(unittest.TestCase):
    def page(self):
        rng = np.random.default_rng(1)
        a = np.full((400, 300), 255, np.uint8)
        a[50:350:20, 30:270] = rng.integers(0, 80, (15, 240))
        return Image.fromarray(a, "L")

    def test_every_recipe_is_repeatable_for_its_seed(self):
        for recipe in degrade.RECIPES:
            a, ppi_a = degrade.degrade(self.page(), 400, recipe, 7)
            b, ppi_b = degrade.degrade(self.page(), 400, recipe, 7)
            self.assertEqual(a.tobytes(), b.tobytes(), recipe)
            self.assertEqual(ppi_a, ppi_b)

    def test_low_resolution_recipes_lower_the_resolution(self):
        im, ppi = degrade.degrade(self.page(), 400, "lowres150", 1)
        self.assertEqual(ppi, 150)
        self.assertEqual(im.size, (112, 150))
        _, ppi = degrade.degrade(self.page(), 100, "lowres150", 1)
        self.assertEqual(ppi, 100, "never raised")

    def test_an_unknown_recipe_is_refused(self):
        with self.assertRaises(ValueError):
            degrade.degrade(self.page(), 300, "melt", 1)


class Selection(unittest.TestCase):
    def test_word_error_rate(self):
        self.assertEqual(corpus.wer("the cat sat", "the cat sat"), 0.0)
        self.assertAlmostEqual(corpus.wer("the bat sat", "the cat sat"), 1 / 3)
        self.assertEqual(corpus.wer("", "two words"), 1.0)
        self.assertEqual(corpus.wer("", ""), 0.0)

    def test_spread_picks_evenly_including_the_ends(self):
        self.assertEqual(corpus.spread(list(range(10)), 3), [0, 4, 9])
        self.assertEqual(corpus.spread([1, 2], 5), [1, 2])

    def test_a_paper_fits_one_page_and_its_text_is_in_reading_order(self):
        src, truth = corpus.paper(0, 3)
        lines = truth.splitlines()
        self.assertTrue(lines[0].startswith("Journal of Public-Domain Studies"))
        self.assertTrue(lines[2].startswith("Abstract. "))
        self.assertEqual(lines[-1], "101")
        self.assertIn("#columns(2", src)
        self.assertIn("hyphenate: false", src)


if __name__ == "__main__":
    unittest.main()
