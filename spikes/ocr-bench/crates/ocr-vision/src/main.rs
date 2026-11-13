//! Apple's Vision framework: `VNRecognizeTextRequest` at the accurate
//! level, `en-US`, with language correction on (`vision-accurate-lc`) or
//! off (`vision-accurate-raw`). The lines it returns are put in reading
//! order by the spike's column ordering (`ocrbench::order`), shared with
//! PP-OCR. Vision chooses its own threads; the harness reports CPU time
//! beside wall time so that shows.
//!
//! Elsewhere than macOS the binary builds and every page is `unsupported`.

#[cfg(target_os = "macos")]
mod vision {
    use objc2::AnyThread;
    use objc2::rc::autoreleasepool;
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
        CGImageAlphaInfo,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };
    use ocrbench::order::{Line, ordered_text};
    use ocrbench::run::Grey;

    pub struct Vision {
        pub correction: bool,
        pub revision: usize,
    }

    impl Vision {
        pub fn new(correction: bool) -> Self {
            let request = VNRecognizeTextRequest::new();
            let revision = unsafe { request.revision() };
            Self {
                correction,
                revision,
            }
        }

        pub fn recognise(&self, page: &Grey) -> Result<String, String> {
            autoreleasepool(|_| {
                let data = CFData::from_bytes(&page.pixels);
                let provider =
                    CGDataProvider::with_cf_data(Some(&data)).ok_or("a data provider")?;
                let grey = CGColorSpace::new_device_gray().ok_or("a grey colour space")?;
                let (w, h) = (page.width as usize, page.height as usize);
                let image = unsafe {
                    CGImage::new(
                        w,
                        h,
                        8,
                        8,
                        w,
                        Some(&grey),
                        CGBitmapInfo(CGImageAlphaInfo::None.0),
                        Some(&provider),
                        std::ptr::null(),
                        false,
                        CGColorRenderingIntent::RenderingIntentDefault,
                    )
                }
                .ok_or("a CGImage")?;
                let request = VNRecognizeTextRequest::new();
                request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
                request.setUsesLanguageCorrection(self.correction);
                request.setRecognitionLanguages(&NSArray::from_retained_slice(&[
                    NSString::from_str("en-US"),
                ]));
                let handler = unsafe {
                    VNImageRequestHandler::initWithCGImage_options(
                        VNImageRequestHandler::alloc(),
                        &image,
                        &NSDictionary::new(),
                    )
                };
                let requests: objc2::rc::Retained<NSArray<VNRequest>> =
                    NSArray::from_slice(&[&**request]);
                handler
                    .performRequests_error(&requests)
                    .map_err(|e| format!("perform: {}", e.localizedDescription()))?;
                let mut lines = Vec::new();
                for observation in request.results().iter().flat_map(|r| r.iter()) {
                    let Some(best) = observation.topCandidates(1).iter().next() else {
                        continue;
                    };
                    let text = best.string().to_string();
                    if text.trim().is_empty() {
                        continue;
                    }
                    // Normalised, origin at the bottom left.
                    let b = unsafe { observation.boundingBox() };
                    let (fw, fh) = (page.width as f64, page.height as f64);
                    lines.push(Line {
                        x0: (b.origin.x * fw) as f32,
                        x1: ((b.origin.x + b.size.width) * fw) as f32,
                        y0: ((1.0 - b.origin.y - b.size.height) * fh) as f32,
                        y1: ((1.0 - b.origin.y) * fh) as f32,
                        text,
                    });
                }
                Ok(ordered_text(lines))
            })
        }
    }
}

use ocrbench::run::{Engine, Grey, main_with};
use serde_json::{Value, json};

struct Vision {
    #[cfg(target_os = "macos")]
    inner: vision::Vision,
    variant: String,
}

impl Engine for Vision {
    fn recognise(&mut self, page: &Grey, _dpi: u32) -> Result<String, String> {
        #[cfg(target_os = "macos")]
        {
            self.inner.recognise(page)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = page;
            Err("unsupported: Vision is macOS only".into())
        }
    }

    fn info(&self) -> Value {
        #[cfg(target_os = "macos")]
        let revision = Some(self.inner.revision);
        #[cfg(not(target_os = "macos"))]
        let revision: Option<usize> = None;
        json!({
            "engine": "Apple Vision VNRecognizeTextRequest",
            "variant": self.variant,
            "revision": revision,
            "level": "accurate",
            "languages": ["en-US"],
            "os": std::env::consts::OS,
        })
    }
}

fn main() {
    main_with("vision", |args| {
        let correction = match args.variant.as_str() {
            "vision-accurate-lc" => true,
            "vision-accurate-raw" => false,
            other => return Err(format!("unknown variant {other}")),
        };
        #[cfg(not(target_os = "macos"))]
        let _ = correction;
        Ok(Vision {
            #[cfg(target_os = "macos")]
            inner: vision::Vision::new(correction),
            variant: args.variant.clone(),
        })
    });
}
