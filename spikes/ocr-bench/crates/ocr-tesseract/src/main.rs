//! Tesseract 5.5 with Leptonica, built from source and linked statically
//! (`xberg-tesseract`): the LSTM engine, automatic page segmentation (psm
//! 3, which finds columns itself), English.
//!
//! `--variant fast|best` picks the model folder under `ATLAS_TESSDATA`
//! (`<dir>/fast/eng.traineddata`, `<dir>/best/eng.traineddata`).
//! One `TessBaseAPI`, loaded once, reused for every page; no OpenMP, so
//! it reads on one thread.

use std::path::PathBuf;

use ocrbench::run::{Engine, Grey, file_sha256, main_with};
use serde_json::{Value, json};
use xberg_tesseract::{TessPageSegMode, TesseractAPI};

struct Tesseract {
    api: TesseractAPI,
    model: PathBuf,
}

impl Engine for Tesseract {
    fn recognise(&mut self, page: &Grey, dpi: u32) -> Result<String, String> {
        let (w, h) = (page.width as i32, page.height as i32);
        self.api
            .set_image(&page.pixels, w, h, 1, w)
            .map_err(|e| format!("image: {e}"))?;
        self.api
            .set_source_resolution(dpi as i32)
            .map_err(|e| format!("resolution: {e}"))?;
        self.api
            .get_utf8_text()
            .map_err(|e| format!("recognise: {e}"))
    }

    fn info(&self) -> Value {
        json!({
            "tesseract": TesseractAPI::version(),
            "crate": "xberg-tesseract 1.3.10",
            "model": self.model.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
            "modelSha256": file_sha256(&self.model),
            "psm": 3,
        })
    }
}

fn main() {
    main_with("tesseract", |args| {
        let dir = PathBuf::from(
            std::env::var_os("ATLAS_TESSDATA")
                .ok_or("set ATLAS_TESSDATA to the tessdata folder")?,
        );
        let which = match args.variant.as_str() {
            "tesseract-fast" | "fast" => "fast",
            "tesseract-best" | "best" => "best",
            other => return Err(format!("unknown variant {other}")),
        };
        let datapath = dir.join(which);
        let api = TesseractAPI::new().map_err(|e| format!("new: {e}"))?;
        api.init(&datapath, "eng")
            .map_err(|e| format!("init: {e}"))?;
        api.set_page_seg_mode(TessPageSegMode::PSM_AUTO)
            .map_err(|e| format!("psm: {e}"))?;
        // Keep Tesseract's own messages ("Estimating resolution…") quiet.
        let _ = api.set_variable(
            "debug_file",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        );
        Ok(Tesseract {
            api,
            model: datapath.join("eng.traineddata"),
        })
    });
}
