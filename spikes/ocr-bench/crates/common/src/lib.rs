//! The OCR spike's shared code (Slice 4.2, ADR 0020): the measures every
//! engine is scored by, the reading order the box engines share, the
//! engines' contract and the page render they share.

pub mod installed;
pub mod order;
pub mod run;
pub mod score;
pub mod sys;
