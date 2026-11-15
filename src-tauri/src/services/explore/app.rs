//! What the window reads of the map, and the owner's names for areas
//! (Slice 4.6.1).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use super::books;
use super::build::{self, MIN_BOOKS};
use super::watch::ExploreShared;
use crate::domain::dto::{
    ExploreArea, ExploreMap, ExplorePoint, ExploreState, ExploreStatus, ExploreSubArea,
};

/// The longest name an area may have, in characters.
pub const MAX_NAME: usize = 60;

pub fn status(conn: &Connection, shared: &ExploreShared) -> rusqlite::Result<ExploreStatus> {
    let s = build::staleness(conn)?;
    let state = match (&s.model, &s.map) {
        (None, _) => ExploreState::NoModel,
        (Some(_), Some(_)) => ExploreState::Ready,
        (Some(_), None) if s.books < MIN_BOOKS => ExploreState::TooFew,
        (Some(_), None) => ExploreState::NotDrawn,
    };
    Ok(ExploreStatus {
        state,
        building: shared.building(),
        books: s.map.as_ref().map_or(0, |m| m.books as u32),
        library_books: s.books as u32,
        new_since: if s.map.is_some() { s.changed as u32 } else { 0 },
        computed_at: s.map.as_ref().map(|m| m.computed_at.clone()),
        generation: s.map.as_ref().map(|m| m.generation),
        held: shared.held(),
        min_books: MIN_BOOKS as u32,
    })
}

fn json_list(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

pub fn map(conn: &Connection, shared: &ExploreShared) -> rusqlite::Result<ExploreMap> {
    let status = status(conn, shared)?;
    let Some(model) = build::staleness(conn)?
        .model
        .filter(|_| status.state == ExploreState::Ready)
    else {
        return Ok(ExploreMap {
            status,
            areas: Vec::new(),
            points: Vec::new(),
        });
    };
    // Areas and their sub-areas.
    type AreaRow = (
        i64,
        Option<i64>,
        i64,
        i64,
        String,
        Option<String>,
        String,
        i64,
    );
    let rows: Vec<AreaRow> = conn
        .prepare(
            "SELECT id, parent_id, number, hue, words, name, central, size
             FROM explore_areas WHERE model_id = ?1 ORDER BY parent_id IS NOT NULL, number",
        )?
        .query_map([&model], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut areas: Vec<ExploreArea> = Vec::new();
    let mut at: HashMap<i64, usize> = HashMap::new();
    for (id, parent, number, hue, words, name, central, size) in rows {
        match parent {
            None => {
                let ids = json_list(&central);
                at.insert(id, areas.len());
                areas.push(ExploreArea {
                    id,
                    number: number as u32,
                    name,
                    words: json_list(&words),
                    hue: hue.clamp(0, 7) as u8,
                    size: size as u32,
                    central: books::titles(conn, &ids)?,
                    subs: Vec::new(),
                });
            }
            Some(parent) => {
                if let Some(&i) = at.get(&parent) {
                    areas[i].subs.push(ExploreSubArea {
                        id,
                        number: number as u32,
                        name,
                        words: json_list(&words),
                        size: size as u32,
                    });
                }
            }
        }
    }
    // Every book, with the file its details panel opens.
    let mut stmt = conn.prepare(
        "SELECT p.document_id, p.x, p.y, p.area_id, p.sub_area_id,
                (SELECT f.id FROM file_instances f WHERE f.document_id = p.document_id
                  ORDER BY f.availability = 'available' DESC,
                           f.id IN (SELECT folded_file_id FROM format_pairs
                                    WHERE folded_file_id IS NOT NULL),
                           f.display_path LIMIT 1)
         FROM explore_points p WHERE p.model_id = ?1 ORDER BY p.document_id",
    )?;
    type PointRow = (String, f64, f64, i64, Option<i64>, Option<String>);
    let rows: Vec<PointRow> = stmt
        .query_map([&model], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let titles = books::titles(conn, &ids)?;
    let points = rows
        .into_iter()
        .zip(titles)
        .filter_map(|((doc, x, y, area, sub, file), title)| {
            Some(ExplorePoint {
                document_id: doc,
                file_id: file?,
                x: x as f32,
                y: y as f32,
                area,
                sub,
                title,
            })
        })
        .collect();
    Ok(ExploreMap {
        status,
        areas,
        points,
    })
}

/// Why a name can't be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameError {
    NotFound,
    TooLong,
}

/// The owner's name for an area or sub-area; `None`, or only spaces, goes
/// back to its words. Kept across redraws where the area survives.
pub fn rename(
    conn: &Connection,
    area_id: i64,
    name: Option<&str>,
) -> rusqlite::Result<Result<(), RenameError>> {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    if name.is_some_and(|n| n.chars().count() > MAX_NAME) {
        return Ok(Err(RenameError::TooLong));
    }
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM explore_areas WHERE id = ?1",
            [area_id],
            |r| r.get(0),
        )
        .optional()?;
    if found.is_none() {
        return Ok(Err(RenameError::NotFound));
    }
    conn.execute(
        "UPDATE explore_areas SET name = ?2 WHERE id = ?1",
        params![area_id, name],
    )?;
    Ok(Ok(()))
}
