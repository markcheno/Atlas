//! Building the map in the app (Slice 4.6.1, ADR 0024): the spike's
//! pipeline, from stored book vectors, kept stable across rebuilds.
//!
//! A rebuild starts from the old map: books keep their places and new
//! ones settle among their neighbours; areas that continue an old one (by
//! the books they share) keep its number, its colour and the owner's name.
//! Everything here is derived data; reading goes through the read-only
//! connections and writing is one transaction at the end.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use rusqlite::{Connection, OptionalExtension, params};

use super::books::{self, Books};
use super::cluster::{self, Area};
use super::hues;
use super::knn;
use super::layout::{self, LayoutOptions};
use super::names;
use super::survey::{self, Options};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::embedding::db::EmbeddingModel;
use crate::services::embedding::{db as edb, models};

/// Fewer books than this make no map.
pub const MIN_BOOKS: i64 = 30;
/// A map is redrawn once this share of its books are new, changed or gone…
pub const STALE_SHARE: f64 = 0.05;
/// …and at least this many.
pub const STALE_MIN: i64 = 10;
/// Book vectors filled in per writer call, for books embedded before 4.6.1.
const BACKFILL_BATCH: usize = 200;
/// The share of the larger of a new area and an old one that they must
/// have in common for the new area to continue the old.
const CONTINUES: f64 = 0.3;
/// Central books stored for each area, shown under its name.
const CENTRAL: usize = 3;

/// A document that is a book on the map's terms: text, a file, not a copy
/// kept out of a book in two formats (`?` is its id), done by model `?1`.
const IS_BOOK: &str = "EXISTS (SELECT 1 FROM documents d WHERE d.id = {doc} AND d.state = 'done')
     AND EXISTS (SELECT 1 FROM file_instances f WHERE f.document_id = {doc})
     AND NOT EXISTS (SELECT 1 FROM book_pairs bp WHERE bp.hidden_document_id = {doc})
     AND EXISTS (SELECT 1 FROM document_embeddings de
                 WHERE de.document_id = {doc} AND de.model_id = ?1 AND de.status = 'done')";

fn is_book(doc: &str) -> String {
    IS_BOOK.replace("{doc}", doc)
}

/// A drawn map's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRow {
    pub model_id: String,
    pub generation: i64,
    pub books: i64,
    pub through_seq: i64,
    pub computed_at: String,
}

pub fn map_row(conn: &Connection, model_id: &str) -> rusqlite::Result<Option<MapRow>> {
    conn.query_row(
        "SELECT model_id, generation, books, through_seq, computed_at
         FROM explore_maps WHERE model_id = ?1",
        [model_id],
        |r| {
            Ok(MapRow {
                model_id: r.get(0)?,
                generation: r.get(1)?,
                books: r.get(2)?,
                through_seq: r.get(3)?,
                computed_at: r.get(4)?,
            })
        },
    )
    .optional()
}

/// Where the map stands against the library now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staleness {
    /// The model search uses, if any.
    pub model: Option<String>,
    pub map: Option<MapRow>,
    /// Books the model has done now.
    pub books: i64,
    /// Books new, changed or gone since the map was drawn (all of them
    /// when there is no map).
    pub changed: i64,
}

impl Staleness {
    /// Whether the map should be drawn again now.
    pub fn due(&self) -> bool {
        if self.model.is_none() || self.books < MIN_BOOKS {
            return false;
        }
        match &self.map {
            None => true,
            Some(map) => {
                let share = (map.books as f64 * STALE_SHARE).ceil() as i64;
                self.changed >= share.max(STALE_MIN)
            }
        }
    }
}

pub fn staleness(conn: &Connection) -> rusqlite::Result<Staleness> {
    let Some(model) = models::serving(conn)? else {
        return Ok(Staleness {
            model: None,
            map: None,
            books: 0,
            changed: 0,
        });
    };
    let books: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM document_embeddings e
             WHERE e.model_id = ?1 AND e.status = 'done' AND {}",
            is_book("e.document_id")
        ),
        [&model.id],
        |r| r.get(0),
    )?;
    let map = map_row(conn, &model.id)?;
    let changed = match &map {
        None => books,
        Some(m) => {
            let new_or_changed: i64 = conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM document_embeddings e
                     WHERE e.model_id = ?1 AND e.status = 'done' AND e.seq > ?2 AND {}",
                    is_book("e.document_id")
                ),
                params![model.id, m.through_seq],
                |r| r.get(0),
            )?;
            let gone: i64 = conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM explore_points p
                     WHERE p.model_id = ?1 AND NOT ({})",
                    is_book("p.document_id")
                ),
                [&model.id],
                |r| r.get(0),
            )?;
            new_or_changed + gone
        }
    };
    Ok(Staleness {
        model: Some(model.id),
        map,
        books,
        changed,
    })
}

/// What a build did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Built(Built),
    NoModel,
    TooFew(usize),
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub generation: i64,
    pub books: usize,
    pub areas: usize,
    /// Areas that continue one of the old map's.
    pub kept: usize,
    pub ms: u64,
}

/// An old area or sub-area, as a rebuild continues it.
#[derive(Debug, Clone)]
struct OldArea {
    id: i64,
    number: i64,
    hue: u8,
    name: Option<String>,
}

#[derive(Debug, Default)]
struct OldMap {
    xy: HashMap<String, [f32; 2]>,
    /// Each book's area and sub-area.
    of: HashMap<String, (i64, Option<i64>)>,
    areas: Vec<OldArea>,
}

fn old_map(conn: &Connection, model_id: &str) -> rusqlite::Result<OldMap> {
    let mut old = OldMap::default();
    let mut stmt = conn.prepare(
        "SELECT document_id, x, y, area_id, sub_area_id FROM explore_points WHERE model_id = ?1",
    )?;
    let rows = stmt.query_map([model_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, f64>(1)?,
            r.get::<_, f64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    for row in rows {
        let (doc, x, y, area, sub) = row?;
        old.xy.insert(doc.clone(), [x as f32, y as f32]);
        old.of.insert(doc, (area, sub));
    }
    old.areas = conn
        .prepare("SELECT id, number, hue, name FROM explore_areas WHERE model_id = ?1")?
        .query_map([model_id], |r| {
            Ok(OldArea {
                id: r.get(0)?,
                number: r.get(1)?,
                hue: r.get::<_, i64>(2)? as u8,
                name: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(old)
}

/// For each new group (its members' document ids), the old group it
/// continues: largest overlap first, each old group at most once, and only
/// when the overlap is at least [`CONTINUES`] of the larger group.
pub fn match_groups(new: &[Vec<&str>], old_of: &HashMap<&str, i64>) -> Vec<Option<i64>> {
    let mut old_size: HashMap<i64, usize> = HashMap::new();
    for g in old_of.values() {
        *old_size.entry(*g).or_default() += 1;
    }
    let mut pairs: Vec<(usize, usize, i64)> = Vec::new();
    for (i, members) in new.iter().enumerate() {
        let mut counts: HashMap<i64, usize> = HashMap::new();
        for m in members {
            if let Some(g) = old_of.get(m) {
                *counts.entry(*g).or_default() += 1;
            }
        }
        for (g, c) in counts {
            pairs.push((c, i, g));
        }
    }
    // Largest overlap first; ties by the new group, then the old one.
    pairs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut out = vec![None; new.len()];
    let mut taken: BTreeSet<i64> = BTreeSet::new();
    for (count, i, g) in pairs {
        if out[i].is_some() || taken.contains(&g) {
            continue;
        }
        let larger = new[i].len().max(old_size[&g]).max(1);
        if count as f64 >= CONTINUES * larger as f64 {
            out[i] = Some(g);
            taken.insert(g);
        }
    }
    out
}

/// Numbers for `n` groups: a continued group keeps its old number, the
/// rest take the lowest free ones, in order.
fn numbers(continued: &[Option<i64>], old_number: &HashMap<i64, i64>) -> Vec<i64> {
    let kept: BTreeSet<i64> = continued
        .iter()
        .flatten()
        .filter_map(|g| old_number.get(g).copied())
        .collect();
    let mut free = (1..).filter(|n| !kept.contains(n));
    continued
        .iter()
        .map(|g| {
            g.and_then(|g| old_number.get(&g).copied())
                .unwrap_or_else(|| free.next().unwrap_or(0))
        })
        .collect()
}

/// How many areas a map of `n` books has: the square root of half of
/// them, from 2 to 16, so a small library isn't cut into areas of a few
/// books (16 from 512 books; the owner's 5,088 have 16, as in the spike).
pub fn area_count(n: usize) -> usize {
    ((n as f64 / 2.0).sqrt().round() as usize).clamp(2, 16)
}

/// Draws the serving model's map, from the old one unless `fresh`.
pub fn build(
    catalog: &CatalogHandle,
    reader: &CatalogReader,
    now: &str,
    fresh: bool,
    threads: usize,
    stop: &dyn Fn() -> bool,
) -> AtlasResult<Outcome> {
    let started = Instant::now();
    let db = |what: &'static str| move |e: rusqlite::Error| AtlasError::db(what)(e);
    let Some(model) = reader
        .read(models::serving)?
        .map_err(db("find the model"))?
    else {
        return Ok(Outcome::NoModel);
    };

    // Vectors for books embedded before 4.6.1, a batch at a time.
    loop {
        if stop() {
            return Ok(Outcome::Stopped);
        }
        let id = model.id.clone();
        let missing = reader
            .read(move |c| books::missing_vectors(c, &id, BACKFILL_BATCH))?
            .map_err(db("find books without a vector"))?;
        if missing.is_empty() {
            break;
        }
        let id = model.id.clone();
        catalog
            .call(move |c| {
                let tx = c.transaction()?;
                for doc in &missing {
                    books::store_vector(&tx, &id, doc)?;
                }
                tx.commit()
            })?
            .map_err(db("store book vectors"))?;
    }

    let (id, dims) = (model.id.clone(), model.dims);
    let (books, through_seq, stamp, old) = reader
        .read(move |c| -> rusqlite::Result<_> {
            Ok((
                books::stored_vectors(c, &id, dims)?,
                edb::last_seq(c, &id)?,
                crate::services::formats::db::stamp(c)?,
                old_map(c, &id)?,
            ))
        })?
        .map_err(db("read the book vectors"))?;
    let n = books.ids.len();
    if (n as i64) < MIN_BOOKS {
        return Ok(Outcome::TooFew(n));
    }
    if stop() {
        return Ok(Outcome::Stopped);
    }

    let opts = Options {
        areas: area_count(n),
        threads,
        ..Options::default()
    };
    let points = &books.vectors;
    let graph = knn::exact(points, opts.neighbours, opts.threads);
    if stop() {
        return Ok(Outcome::Stopped);
    }
    let area_opts = survey::area_options(n, &opts);
    let areas = if fresh || old.of.is_empty() {
        cluster::areas(points, &area_opts)
    } else {
        cluster::areas_seeded(points, &seeds(&books, &old), &area_opts)
    };
    if stop() {
        return Ok(Outcome::Stopped);
    }
    let from_old = !fresh && !old.xy.is_empty();
    let init: Vec<Option<[f32; 2]>> = books.ids.iter().map(|d| old.xy.get(d).copied()).collect();
    let layout_opts = if from_old {
        LayoutOptions::refining(n, opts.seed)
    } else {
        LayoutOptions::for_count(n, opts.seed)
    };
    let Some(xy) = layout::layout_from(
        points,
        &graph,
        from_old.then_some(init.as_slice()),
        &layout_opts,
        stop,
    ) else {
        return Ok(Outcome::Stopped);
    };

    let model_for_names = model.clone();
    let books_for_names = books.clone();
    let areas_for_names = areas.clone();
    let area_names = reader
        .read(move |c| -> rusqlite::Result<_> {
            let titles = books::titles(c, &books_for_names.ids)?;
            survey::name_areas(
                c,
                &model_for_names,
                &books_for_names,
                &titles,
                &books_for_names.vectors,
                &areas_for_names,
            )
        })?
        .map_err(db("name the areas"))?;

    let drawn = continue_old(&books, &areas, &old, &xy);
    let generation = write(
        catalog,
        &model,
        &books,
        &areas,
        &area_names,
        &drawn,
        &xy,
        (through_seq, stamp, now.to_owned()),
        started.elapsed().as_millis() as u64,
    )?;
    Ok(Outcome::Built(Built {
        generation,
        books: n,
        areas: areas.len(),
        kept: drawn.area_old.iter().flatten().count(),
        ms: started.elapsed().as_millis() as u64,
    }))
}

/// The old map's areas as rows of this build's books: each area's books
/// with its sub-areas' books, for the clustering to start from.
fn seeds(books: &Books, old: &OldMap) -> Vec<(Vec<u32>, Vec<Vec<u32>>)> {
    type Seed = (Vec<u32>, std::collections::BTreeMap<i64, Vec<u32>>);
    let mut by_area: std::collections::BTreeMap<i64, Seed> = std::collections::BTreeMap::new();
    for (row, doc) in books.ids.iter().enumerate() {
        if let Some((area, sub)) = old.of.get(doc) {
            let entry = by_area.entry(*area).or_default();
            entry.0.push(row as u32);
            if let Some(sub) = sub {
                entry.1.entry(*sub).or_default().push(row as u32);
            }
        }
    }
    by_area
        .into_values()
        .map(|(rows, subs)| (rows, subs.into_values().collect()))
        .collect()
}

/// How the new map continues the old one: numbers, hues and names.
struct Drawn {
    area_old: Vec<Option<i64>>,
    area_number: Vec<i64>,
    area_hue: Vec<u8>,
    area_name: Vec<Option<String>>,
    /// Per area, per child: number and name.
    sub_number: Vec<Vec<i64>>,
    sub_name: Vec<Vec<Option<String>>>,
}

fn members<'a>(books: &'a Books, area: &Area) -> Vec<&'a str> {
    area.members
        .iter()
        .map(|&m| books.ids[m as usize].as_str())
        .collect()
}

fn continue_old(books: &Books, areas: &[Area], old: &OldMap, xy: &[[f32; 2]]) -> Drawn {
    let by_id: HashMap<i64, &OldArea> = old.areas.iter().map(|a| (a.id, a)).collect();
    let old_area_of: HashMap<&str, i64> =
        old.of.iter().map(|(d, (a, _))| (d.as_str(), *a)).collect();
    let new_members: Vec<Vec<&str>> = areas.iter().map(|a| members(books, a)).collect();
    let area_old = match_groups(&new_members, &old_area_of);
    let old_number: HashMap<i64, i64> = old.areas.iter().map(|a| (a.id, a.number)).collect();
    let area_number = numbers(&area_old, &old_number);
    let area_name = area_old
        .iter()
        .map(|g| g.and_then(|g| by_id.get(&g).and_then(|a| a.name.clone())))
        .collect();
    let keep: Vec<Option<u8>> = area_old
        .iter()
        .map(|g| g.and_then(|g| by_id.get(&g).map(|a| a.hue)))
        .collect();
    let area_of = cluster::area_of_each(books.ids.len(), areas);
    let area_hue = hues::assign(&hues::touching(xy, &area_of, areas.len()), &keep);

    let mut sub_number = Vec::new();
    let mut sub_name = Vec::new();
    for (area, old_parent) in areas.iter().zip(&area_old) {
        // Sub-areas continue the old sub-areas of the area they continue.
        let old_sub_of: HashMap<&str, i64> = match old_parent {
            Some(parent) => old
                .of
                .iter()
                .filter(|(_, (a, _))| a == parent)
                .filter_map(|(d, (_, s))| s.map(|s| (d.as_str(), s)))
                .collect(),
            None => HashMap::new(),
        };
        let children: Vec<Vec<&str>> = area.children.iter().map(|c| members(books, c)).collect();
        let continued = match_groups(&children, &old_sub_of);
        sub_number.push(numbers(&continued, &old_number));
        sub_name.push(
            continued
                .iter()
                .map(|g| g.and_then(|g| by_id.get(&g).and_then(|a| a.name.clone())))
                .collect(),
        );
    }
    Drawn {
        area_old,
        area_number,
        area_hue,
        area_name,
        sub_number,
        sub_name,
    }
}

#[allow(clippy::too_many_arguments)]
fn write(
    catalog: &CatalogHandle,
    model: &EmbeddingModel,
    books: &Books,
    areas: &[Area],
    area_names: &[survey::AreaName],
    drawn: &Drawn,
    xy: &[[f32; 2]],
    (through_seq, stamp, now): (i64, String, String),
    ms: u64,
) -> AtlasResult<i64> {
    struct Row {
        number: i64,
        hue: u8,
        words: String,
        name: Option<String>,
        central: String,
        size: i64,
        cohesion: f64,
        subs: Vec<Row>,
    }
    let central = |area: &Area| {
        let ids: Vec<&str> = names::central(&books.vectors, area, CENTRAL)
            .into_iter()
            .map(|m| books.ids[m as usize].as_str())
            .collect();
        serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into())
    };
    let words = |w: &[String]| serde_json::to_string(w).unwrap_or_else(|_| "[]".into());
    let rows: Vec<Row> = areas
        .iter()
        .enumerate()
        .map(|(i, area)| Row {
            number: drawn.area_number[i],
            hue: drawn.area_hue[i],
            words: words(&area_names[i].words),
            name: drawn.area_name[i].clone(),
            central: central(area),
            size: area.members.len() as i64,
            cohesion: f64::from(area.cohesion),
            subs: area
                .children
                .iter()
                .enumerate()
                .map(|(j, child)| Row {
                    number: drawn.sub_number[i][j],
                    hue: drawn.area_hue[i],
                    words: words(area_names[i].sub_areas.get(j).map_or(&[][..], |w| w)),
                    name: drawn.sub_name[i][j].clone(),
                    central: central(child),
                    size: child.members.len() as i64,
                    cohesion: f64::from(child.cohesion),
                    subs: Vec::new(),
                })
                .collect(),
        })
        .collect();
    // Each book's area and sub-area, by position.
    let mut place: Vec<(usize, Option<usize>)> = vec![(0, None); books.ids.len()];
    for (i, area) in areas.iter().enumerate() {
        for &m in &area.members {
            place[m as usize].0 = i;
        }
        for (j, child) in area.children.iter().enumerate() {
            for &m in &child.members {
                place[m as usize] = (i, Some(j));
            }
        }
    }
    let points: Vec<(String, f64, f64, usize, Option<usize>)> = books
        .ids
        .iter()
        .zip(xy)
        .zip(place)
        .map(|((d, p), (a, s))| (d.clone(), f64::from(p[0]), f64::from(p[1]), a, s))
        .collect();
    let model_id = model.id.clone();
    let book_count = books.ids.len() as i64;
    catalog
        .call(move |c| -> rusqlite::Result<i64> {
            let tx = c.transaction()?;
            tx.execute(
                "DELETE FROM explore_points WHERE model_id = ?1",
                [&model_id],
            )?;
            tx.execute(
                "DELETE FROM explore_areas WHERE model_id = ?1 AND parent_id IS NOT NULL",
                [&model_id],
            )?;
            tx.execute("DELETE FROM explore_areas WHERE model_id = ?1", [&model_id])?;
            let mut ids: Vec<(i64, Vec<i64>)> = Vec::new();
            {
                let mut insert = tx.prepare(
                    "INSERT INTO explore_areas (model_id, parent_id, number, hue, words, name,
                         central, size, cohesion)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )?;
                for row in &rows {
                    insert.execute(params![
                        model_id,
                        Option::<i64>::None,
                        row.number,
                        i64::from(row.hue),
                        row.words,
                        row.name,
                        row.central,
                        row.size,
                        row.cohesion
                    ])?;
                    let id = tx.last_insert_rowid();
                    let mut subs = Vec::new();
                    for sub in &row.subs {
                        insert.execute(params![
                            model_id,
                            id,
                            sub.number,
                            i64::from(sub.hue),
                            sub.words,
                            sub.name,
                            sub.central,
                            sub.size,
                            sub.cohesion
                        ])?;
                        subs.push(tx.last_insert_rowid());
                    }
                    ids.push((id, subs));
                }
                let mut insert = tx.prepare(
                    "INSERT INTO explore_points (model_id, document_id, x, y, area_id, sub_area_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )?;
                for (doc, x, y, a, s) in &points {
                    let (area_id, subs) = &ids[*a];
                    insert.execute(params![model_id, doc, x, y, area_id, s.map(|s| subs[s])])?;
                }
            }
            tx.execute(
                "INSERT INTO explore_maps (model_id, generation, books, through_seq, pairs_stamp,
                     computed_at, ms)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (model_id) DO UPDATE SET
                     generation = generation + 1, books = excluded.books,
                     through_seq = excluded.through_seq, pairs_stamp = excluded.pairs_stamp,
                     computed_at = excluded.computed_at, ms = excluded.ms",
                params![model_id, book_count, through_seq, stamp, now, ms as i64],
            )?;
            let generation: i64 = tx.query_row(
                "SELECT generation FROM explore_maps WHERE model_id = ?1",
                [&model_id],
                |r| r.get(0),
            )?;
            tx.commit()?;
            Ok(generation)
        })?
        .map_err(AtlasError::db("save the map"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_continue_the_old_group_they_share_most_with() {
        let old: HashMap<&str, i64> = [
            ("a", 1),
            ("b", 1),
            ("c", 1),
            ("d", 2),
            ("e", 2),
            ("f", 2),
            ("g", 3),
        ]
        .into_iter()
        .collect();
        // The first new group is old 2 with a newcomer; the second is old 1
        // less a book; the third shares one book of three with old 3's one.
        let new = vec![
            vec!["d", "e", "f", "x"],
            vec!["a", "b"],
            vec!["g", "y", "z"],
        ];
        assert_eq!(match_groups(&new, &old), vec![Some(2), Some(1), Some(3)]);
        // A split: only the larger half continues the old group.
        let new = vec![vec!["a"], vec!["b", "c"]];
        assert_eq!(match_groups(&new, &old), vec![None, Some(1)]);
        // Too small an overlap continues nothing.
        let new = vec![vec!["a", "p", "q", "r", "s", "t", "u", "v", "w", "x"]];
        assert_eq!(match_groups(&new, &old), vec![None]);
    }

    #[test]
    fn small_libraries_have_fewer_areas() {
        assert_eq!(area_count(30), 4);
        assert_eq!(area_count(200), 10);
        assert_eq!(area_count(512), 16);
        assert_eq!(area_count(5_088), 16);
        assert_eq!(area_count(3), 2);
    }

    #[test]
    fn kept_numbers_stay_and_new_groups_take_the_free_ones() {
        let old_number: HashMap<i64, i64> = [(10, 1), (11, 2), (12, 3)].into_iter().collect();
        assert_eq!(
            numbers(&[None, Some(12), None, Some(10)], &old_number),
            vec![2, 3, 4, 1]
        );
    }

    #[test]
    fn a_map_is_due_once_enough_has_changed() {
        let map = MapRow {
            model_id: "m".into(),
            generation: 1,
            books: 1000,
            through_seq: 0,
            computed_at: "t".into(),
        };
        let s = |map: Option<MapRow>, books, changed| Staleness {
            model: Some("m".into()),
            map,
            books,
            changed,
        };
        assert!(s(None, 30, 30).due());
        assert!(!s(None, 29, 29).due(), "too few books");
        assert!(!s(Some(map.clone()), 1000, 49).due());
        assert!(s(Some(map.clone()), 1000, 50).due(), "5%");
        let small = MapRow { books: 40, ..map };
        assert!(!s(Some(small.clone()), 40, 9).due(), "at least ten");
        assert!(s(Some(small), 40, 10).due());
        assert!(
            !Staleness {
                model: None,
                map: None,
                books: 100,
                changed: 100
            }
            .due()
        );
    }
}
