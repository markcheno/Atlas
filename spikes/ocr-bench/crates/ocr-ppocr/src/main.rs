//! PP-OCR through ONNX Runtime (`oar-ocr` 0.10.2 on `ort` 2.0.0-rc.13):
//! DB text detection, then CTC recognition of each line, put in reading
//! order by the spike's column ordering (`ocrbench::order`), shared with
//! Vision.
//!
//! `--variant ppocr-v5-mobile|ppocr-v6-tiny|ppocr-v6-small` picks the
//! models under `ATLAS_PPOCR_MODELS` (the `ppocr-models` folder of
//! `spikes/.tools`). ONNX Runtime is the library named by `ORT_DYLIB_PATH`.
//! One thread: ONNX Runtime's intra- and inter-op pools at 1, spinning
//! off, and rayon (used by the pipeline) at 1.
//!
//! Each model's own detection defaults are used, as its `inference.yml`
//! gives them: v5 `thresh` 0.3, `box_thresh` 0.6, `unclip_ratio` 1.5; v6
//! 0.2, 0.45 (0.4 tiny), 1.4. The page goes in at its rendered size (the
//! detector's input is bounded at 4,000 px on its longer side).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oar_ocr::core::config::OrtSessionConfig;
use oar_ocr::domain::tasks::TextDetectionConfig;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};
use oar_ocr::processors::LimitType;
use ocrbench::order::{Line, ordered_text};
use ocrbench::run::{Engine, Grey, file_sha256, main_with};
use serde_json::{Value, json};

struct PpOcr {
    ocr: OAROCR,
    det: PathBuf,
    rec: PathBuf,
    variant: String,
}

/// The `character_dict` list of a PaddleX `inference.yml`: one YAML scalar
/// per `- ` item, plain, single-quoted ('' for a quote) or double-quoted.
fn character_dict(yml: &str) -> Result<Vec<String>, String> {
    let mut lines = yml.lines().skip_while(|l| l.trim() != "character_dict:");
    lines
        .next()
        .ok_or("no character_dict in the model's inference.yml")?;
    let mut out = Vec::new();
    for line in lines {
        let Some(item) = line.trim_start().strip_prefix("- ") else {
            if line.trim_start().starts_with('-') {
                // `-` alone: an empty scalar.
                out.push(String::new());
                continue;
            }
            break;
        };
        let value = if let Some(inner) = item.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
        {
            inner.replace("''", "'")
        } else if let Some(inner) = item.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            unescape_double(inner)?
        } else {
            item.to_owned()
        };
        out.push(value);
    }
    if out.is_empty() {
        return Err("an empty character_dict".into());
    }
    Ok(out)
}

fn unescape_double(s: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                let code = u32::from_str_radix(&hex, 16).map_err(|_| format!("\\u{hex}"))?;
                out.push(char::from_u32(code).ok_or(format!("\\u{hex}"))?);
            }
            Some('U') => {
                let hex: String = chars.by_ref().take(8).collect();
                let code = u32::from_str_radix(&hex, 16).map_err(|_| format!("\\U{hex}"))?;
                out.push(char::from_u32(code).ok_or(format!("\\U{hex}"))?);
            }
            other => return Err(format!("an escape the spike doesn't read: \\{other:?}")),
        }
    }
    Ok(out)
}

/// The detection defaults the model's `inference.yml` gives.
fn detection_config(yml: &str) -> TextDetectionConfig {
    let mut values: HashMap<&str, f32> = HashMap::new();
    for line in yml.lines() {
        let line = line.trim();
        for key in ["thresh", "box_thresh", "unclip_ratio"] {
            if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':'))
                && let Ok(v) = v.trim().parse()
            {
                values.insert(key, v);
            }
        }
    }
    let base = TextDetectionConfig::default();
    TextDetectionConfig {
        score_threshold: values
            .get("thresh")
            .copied()
            .unwrap_or(base.score_threshold),
        box_threshold: values
            .get("box_thresh")
            .copied()
            .unwrap_or(base.box_threshold),
        unclip_ratio: values
            .get("unclip_ratio")
            .copied()
            .unwrap_or(base.unclip_ratio),
        max_candidates: 3000,
        limit_side_len: Some(64),
        limit_type: Some(LimitType::Min),
        max_side_len: Some(4000),
    }
}

impl Engine for PpOcr {
    fn recognise(&mut self, page: &Grey, _dpi: u32) -> Result<String, String> {
        // The models take three channels.
        let mut rgb = Vec::with_capacity(page.pixels.len() * 3);
        for &g in &page.pixels {
            rgb.extend_from_slice(&[g, g, g]);
        }
        let image = image::RgbImage::from_raw(page.width, page.height, rgb)
            .ok_or("an image of the wrong size")?;
        let results = self
            .ocr
            .predict(vec![image])
            .map_err(|e| format!("predict: {e}"))?;
        let mut lines = Vec::new();
        for result in results {
            for region in result.text_regions {
                let Some(text) = region.text.as_deref().filter(|t| !t.trim().is_empty()) else {
                    continue;
                };
                let points = &region.bounding_box.points;
                if points.is_empty() {
                    continue;
                }
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in points {
                    x0 = x0.min(p.x);
                    y0 = y0.min(p.y);
                    x1 = x1.max(p.x);
                    y1 = y1.max(p.y);
                }
                lines.push(Line {
                    x0,
                    y0,
                    x1,
                    y1,
                    text: text.to_owned(),
                });
            }
        }
        Ok(ordered_text(lines))
    }

    fn info(&self) -> Value {
        json!({
            "crate": "oar-ocr 0.10.2, ort 2.0.0-rc.13 (load-dynamic)",
            "onnxRuntime": std::env::var("ORT_DYLIB_PATH").ok()
                .and_then(|p| Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned())),
            "variant": self.variant,
            "detSha256": file_sha256(&self.det),
            "recSha256": file_sha256(&self.rec),
        })
    }
}

fn main() {
    main_with("ppocr", |args| {
        if std::env::var_os("ORT_DYLIB_PATH").is_none() {
            return Err("set ORT_DYLIB_PATH to the ONNX Runtime library".into());
        }
        // Before rayon starts any pool.
        // SAFETY: the process has one thread at this point.
        unsafe { std::env::set_var("RAYON_NUM_THREADS", "1") };
        let dir =
            PathBuf::from(std::env::var_os("ATLAS_PPOCR_MODELS").ok_or("set ATLAS_PPOCR_MODELS")?);
        let (det, rec) = match args.variant.as_str() {
            "ppocr-v5-mobile" => ("v5-mobile-det", "v5-mobile-en-rec"),
            "ppocr-v6-tiny" => ("v6-tiny-det", "v6-tiny-rec"),
            "ppocr-v6-small" => ("v6-small-det", "v6-small-rec"),
            other => return Err(format!("unknown variant {other}")),
        };
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(format!("{name}.yml")))
                .map_err(|e| format!("{name}.yml: {e}"))
        };
        let dict = character_dict(&read(rec)?)?;
        let detection = detection_config(&read(det)?);
        let session = OrtSessionConfig {
            intra_threads: Some(1),
            inter_threads: Some(1),
            parallel_execution: Some(false),
            session_config_entries: Some(HashMap::from([
                ("session.intra_op.allow_spinning".to_owned(), "0".to_owned()),
                ("session.inter_op.allow_spinning".to_owned(), "0".to_owned()),
            ])),
            ..OrtSessionConfig::default()
        };
        let (det, rec) = (
            dir.join(format!("{det}.onnx")),
            dir.join(format!("{rec}.onnx")),
        );
        let ocr = OAROCRBuilder::new(det.clone(), rec.clone(), PathBuf::new())
            .character_dict_content(dict.join("\n"))
            .text_detection_config(detection)
            .ort_session(session)
            .image_batch_size(1)
            .build()
            .map_err(|e| format!("build: {e}"))?;
        Ok(PpOcr {
            ocr,
            det,
            rec,
            variant: args.variant.clone(),
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionary_reads_every_kind_of_scalar() {
        let yml = "PostProcess:\n  name: CTCLabelDecode\n  character_dict:\n  - '0'\n  - A\n  - ''''\n  - '\"'\n  - \"\\\\\"\n  - \"\\u00E9\"\n  - ','\nNext: 1\n";
        assert_eq!(
            character_dict(yml).unwrap(),
            ["0", "A", "'", "\"", "\\", "é", ","]
        );
        assert!(character_dict("nothing: here").is_err());
    }

    #[test]
    fn detection_defaults_come_from_the_model() {
        let yml = "PostProcess:\n  name: DBPostProcess\n  thresh: 0.2\n  box_thresh: 0.45\n  max_candidates: 1000\n  unclip_ratio: 1.4\n";
        let c = detection_config(yml);
        assert_eq!(
            (c.score_threshold, c.box_threshold, c.unclip_ratio),
            (0.2, 0.45, 1.4)
        );
    }
}
