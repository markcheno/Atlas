//! The whole spike in one call: book vectors, areas, the map, their
//! measures and the names. [`chart`] works on vectors alone (the
//! measurement uses it on synthetic ones); [`run`] reads a catalog.

use std::collections::BTreeSet;
use std::time::Instant;

use rusqlite::Connection;
use serde::Serialize;

use super::books::{self, Books, Coverage, Sampling};
use super::cluster::{self, Area, AreaOptions};
use super::knn::{self, Neighbours};
use super::layout::{self, LayoutOptions};
use super::{Points, names, quality};
use crate::services::embedding::db::EmbeddingModel;

/// Rows measured for trustworthiness when there are more books than this.
const TRUST_SAMPLE: usize = 2_000;
/// Two books this alike are probably one text twice.
const NEAR_PAIR: f32 = 0.98;
/// What names are made from.
const CENTRAL_BOOKS: usize = 12;
const CENTRAL_CHUNKS: usize = 3;
const AREA_WORDS: usize = 6;
const SUB_AREA_BOOKS: usize = 8;
const SUB_AREA_WORDS: usize = 4;
const CENTRAL_TITLES: usize = 3;
/// A word names an area only if this many of its central books use it.
const MIN_BOOKS: usize = 2;

#[derive(Debug, Clone)]
pub struct Options {
    pub sampling: Sampling,
    pub areas: usize,
    pub sub_areas: usize,
    /// Neighbours per book in the graph the map is made from.
    pub neighbours: usize,
    pub seed: u64,
    /// Take out what every book shares before anything else.
    pub centred: bool,
    pub threads: usize,
}

impl Default for Options {
    /// The spike's fixed configuration (docs/spike-explore.md).
    fn default() -> Self {
        Self {
            sampling: Sampling::Sample,
            areas: 16,
            sub_areas: 4,
            neighbours: 15,
            seed: 1,
            centred: false,
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    pub load_ms: u64,
    pub neighbours_ms: u64,
    pub areas_ms: u64,
    pub layout_ms: u64,
    pub measures_ms: u64,
    pub names_ms: u64,
}

impl Timings {
    pub fn after_loading_ms(&self) -> u64 {
        self.neighbours_ms + self.areas_ms + self.layout_ms + self.measures_ms + self.names_ms
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaNumbers {
    pub books: usize,
    pub cohesion: f32,
    pub sub_areas: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapNumbers {
    /// Of the map, at the graph's neighbour count.
    pub trustworthiness: f32,
    /// Of the starting layout (PCA alone), for comparison.
    pub pca_trustworthiness: f32,
    pub neighbour_preservation: f32,
    /// At 10 neighbours on the map.
    pub area_agreement: f32,
}

/// What [`chart`] makes of a set of vectors.
#[derive(Debug, Clone)]
pub struct Chart {
    pub areas: Vec<Area>,
    pub graph: Neighbours,
    pub xy: Vec<[f32; 2]>,
    pub map: MapNumbers,
    /// The areas' silhouette, and that of the other variant (centred when
    /// this run isn't, plain when it is), clustered the same way.
    pub silhouette: f32,
    pub silhouette_other: f32,
    pub near_pairs: usize,
    pub timings: Timings,
}

fn ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

fn area_options(n: usize, opts: &Options) -> AreaOptions {
    AreaOptions {
        areas: opts.areas,
        min_area: (n / (opts.areas.max(1) * 8)).max(3),
        sub_areas: opts.sub_areas,
        min_sub_area: 3,
        seed: opts.seed,
    }
}

/// Areas, the map and their measures for `points` (already centred, if
/// they are to be).
pub fn chart(points: &Points, opts: &Options) -> Chart {
    let n = points.len();
    let mut timings = Timings::default();

    let t = Instant::now();
    let graph = knn::exact(points, opts.neighbours, opts.threads);
    timings.neighbours_ms = ms(t);

    let t = Instant::now();
    let area_opts = area_options(n, opts);
    let areas = cluster::areas(points, &area_opts);
    timings.areas_ms = ms(t);

    let t = Instant::now();
    let xy = layout::layout(points, &graph, &LayoutOptions::for_count(n, opts.seed));
    timings.layout_ms = ms(t);

    let t = Instant::now();
    let sample = (n > TRUST_SAMPLE).then_some((TRUST_SAMPLE, opts.seed));
    let area_of = cluster::area_of_each(n, &areas);
    let map = MapNumbers {
        trustworthiness: quality::trustworthiness(points, &xy, opts.neighbours, sample),
        pca_trustworthiness: quality::trustworthiness(
            points,
            &layout::pca2(points, opts.seed),
            opts.neighbours,
            sample,
        ),
        neighbour_preservation: quality::neighbour_preservation(&graph, &xy, opts.neighbours),
        area_agreement: quality::area_agreement(&xy, &area_of, 10),
    };
    let silhouette = quality::silhouette(points, &areas);
    // The other variant can't be told from here; `run` fills it in.
    let near_pairs = quality::near_pairs(&graph, NEAR_PAIR);
    timings.measures_ms = ms(t);

    Chart {
        areas,
        graph,
        xy,
        map,
        silhouette,
        silhouette_other: 0.0,
        near_pairs,
        timings,
    }
}

/// An area's name and its sub-areas' names.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaName {
    pub words: Vec<String>,
    /// The titles of its most central books.
    pub titles: Vec<String>,
    /// Words for each sub-area, in the order of the area's children.
    pub sub_areas: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkCounts {
    pub total: u64,
    pub fewest: u32,
    pub median: u32,
    pub most: u32,
}

/// Everything the owner may share: counts, measures and timings. No
/// field can hold a title, a word from a book or a model's name.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Numbers {
    pub spike_version: u32,
    pub coverage: Coverage,
    /// `used` over `with_text`: below 0.9 the run is preliminary.
    pub covered: f32,
    pub all_chunks: bool,
    pub centred: bool,
    pub dims: usize,
    pub books: usize,
    pub chunks: ChunkCounts,
    pub areas: Vec<AreaNumbers>,
    pub silhouette: f32,
    pub silhouette_other_variant: f32,
    pub map: MapNumbers,
    pub near_pairs: usize,
    pub timings: Timings,
}

/// A library surveyed: what the page shows and the numbers to share.
#[derive(Debug, Clone)]
pub struct Survey {
    pub books: Books,
    pub titles: Vec<String>,
    pub areas: Vec<Area>,
    pub names: Vec<AreaName>,
    pub xy: Vec<[f32; 2]>,
    pub numbers: Numbers,
}

/// Surveys the library behind `conn` with `model`'s vectors. `None` when
/// the model has too few books to make anything of (fewer than three).
pub fn run(
    conn: &Connection,
    model: &EmbeddingModel,
    opts: &Options,
) -> rusqlite::Result<Option<Survey>> {
    let t = Instant::now();
    let (books, coverage) = books::book_vectors(conn, model, opts.sampling)?;
    let load_ms = ms(t);
    if books.ids.len() < 3 {
        return Ok(None);
    }
    let centred = books.vectors.centred();
    let (points, other) = if opts.centred {
        (&centred, &books.vectors)
    } else {
        (&books.vectors, &centred)
    };
    let mut chart = chart(points, opts);
    chart.timings.load_ms = load_ms;
    chart.silhouette_other = quality::silhouette(
        other,
        &cluster::areas(other, &area_options(other.len(), opts)),
    );

    let t = Instant::now();
    let titles = books::titles(conn, &books.ids)?;
    let names = name_areas(conn, model, &books, &titles, points, &chart.areas)?;
    chart.timings.names_ms = ms(t);

    let mut sorted = books.chunks.clone();
    sorted.sort_unstable();
    let numbers = Numbers {
        spike_version: 1,
        coverage,
        covered: coverage.used as f32 / coverage.with_text.max(1) as f32,
        all_chunks: opts.sampling == Sampling::All,
        centred: opts.centred,
        dims: model.dims,
        books: books.ids.len(),
        chunks: ChunkCounts {
            total: sorted.iter().map(|&c| u64::from(c)).sum(),
            fewest: sorted[0],
            median: sorted[sorted.len() / 2],
            most: sorted[sorted.len() - 1],
        },
        areas: chart
            .areas
            .iter()
            .map(|a| AreaNumbers {
                books: a.members.len(),
                cohesion: a.cohesion,
                sub_areas: a.children.len(),
            })
            .collect(),
        silhouette: chart.silhouette,
        silhouette_other_variant: chart.silhouette_other,
        map: chart.map.clone(),
        near_pairs: chart.near_pairs,
        timings: chart.timings.clone(),
    };
    Ok(Some(Survey {
        books,
        titles,
        areas: chart.areas,
        names,
        xy: chart.xy,
        numbers,
    }))
}

/// One set of words per central book of `area`: its title and what it
/// says where it is most like the area.
fn book_words(
    conn: &Connection,
    model: &EmbeddingModel,
    books: &Books,
    titles: &[String],
    points: &Points,
    area: &Area,
    take_books: usize,
) -> rusqlite::Result<Vec<BTreeSet<String>>> {
    names::central(points, area, take_books)
        .into_iter()
        .map(|m| {
            let m = m as usize;
            let mut words: BTreeSet<String> = names::words(&titles[m]).into_iter().collect();
            for text in books::central_chunks(
                conn,
                model,
                &books.ids[m],
                books.chunks[m] as usize,
                &area.centre,
                CENTRAL_CHUNKS,
            )? {
                words.extend(names::words(&text));
            }
            Ok(words)
        })
        .collect()
}

fn name_areas(
    conn: &Connection,
    model: &EmbeddingModel,
    books: &Books,
    titles: &[String],
    points: &Points,
    areas: &[Area],
) -> rusqlite::Result<Vec<AreaName>> {
    let per_area: Vec<Vec<BTreeSet<String>>> = areas
        .iter()
        .map(|a| book_words(conn, model, books, titles, points, a, CENTRAL_BOOKS))
        .collect::<rusqlite::Result<_>>()?;
    let words = names::distinctive(&per_area, AREA_WORDS, MIN_BOOKS);
    areas
        .iter()
        .zip(words)
        .map(|(area, words)| {
            // A sub-area is named against its siblings.
            let per_child: Vec<Vec<BTreeSet<String>>> = area
                .children
                .iter()
                .map(|c| book_words(conn, model, books, titles, points, c, SUB_AREA_BOOKS))
                .collect::<rusqlite::Result<_>>()?;
            Ok(AreaName {
                words,
                titles: names::central(points, area, CENTRAL_TITLES)
                    .into_iter()
                    .map(|m| titles[m as usize].clone())
                    .collect(),
                sub_areas: names::distinctive(&per_child, SUB_AREA_WORDS, MIN_BOOKS),
            })
        })
        .collect()
}
