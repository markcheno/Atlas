//! Renders each page and returns no text: the size and build-time
//! reference for the engines, and the page images (`--dump`) for the
//! owner's side-by-side page.

use ocrbench::run::{Engine, Grey, main_with};
use serde_json::{Value, json};

struct Nothing;

impl Engine for Nothing {
    fn recognise(&mut self, _: &Grey, _: u32) -> Result<String, String> {
        Ok(String::new())
    }

    fn info(&self) -> Value {
        json!({ "engine": "none" })
    }
}

fn main() {
    main_with("baseline", |_| Ok(Nothing));
}
