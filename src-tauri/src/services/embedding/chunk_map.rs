//! Chunk id → document, for filtering meaning search by folder or file
//! type (Slice 2.4). Chunk rows never change and their ids are never
//! reused (ADR 0013), so the map is topped up with the chunks added since
//! and never goes stale; which documents a filter allows is asked of the
//! catalog per search, so moved or missing files are always current.

use std::collections::HashMap;

use rusqlite::Connection;

#[derive(Default)]
pub struct ChunkMap {
    /// Document number per chunk id; 0 means unknown.
    docs: Vec<u32>,
    /// Document id per document number (index 0 unused).
    ids: Vec<String>,
    numbers: HashMap<String, u32>,
    /// The highest chunk id mapped.
    through: u64,
}

impl ChunkMap {
    /// Adds every chunk above the highest one mapped.
    pub fn top_up(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        if self.ids.is_empty() {
            self.ids.push(String::new());
        }
        let mut stmt =
            conn.prepare_cached("SELECT id, document_id FROM chunks WHERE id > ?1 ORDER BY id")?;
        let rows = stmt.query_map([self.through as i64], |r| {
            Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (chunk, doc) = row?;
            let number = match self.numbers.get(&doc) {
                Some(n) => *n,
                None => {
                    let n = self.ids.len() as u32;
                    self.ids.push(doc.clone());
                    self.numbers.insert(doc, n);
                    n
                }
            };
            let at = chunk as usize;
            if self.docs.len() <= at {
                self.docs.resize(at + 1, 0);
            }
            self.docs[at] = number;
            self.through = self.through.max(chunk);
        }
        Ok(())
    }

    /// The document number of a chunk, if mapped.
    pub fn document_of(&self, chunk: u64) -> Option<u32> {
        match self.docs.get(chunk as usize) {
            Some(0) | None => None,
            Some(n) => Some(*n),
        }
    }

    /// A document id's number, if any chunk of it is mapped.
    pub fn number_of(&self, document_id: &str) -> Option<u32> {
        self.numbers.get(document_id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_chunks_to_documents_and_tops_up() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE chunks (id INTEGER PRIMARY KEY, document_id TEXT);
             INSERT INTO chunks VALUES (1, 'a'), (2, 'a'), (5, 'b');",
        )
        .unwrap();
        let mut m = ChunkMap::default();
        m.top_up(&c).unwrap();
        let (a, b) = (m.number_of("a").unwrap(), m.number_of("b").unwrap());
        assert_ne!(a, b);
        assert_eq!(m.document_of(1), Some(a));
        assert_eq!(m.document_of(5), Some(b));
        assert_eq!(m.document_of(3), None);
        assert_eq!(m.document_of(99), None);
        c.execute_batch("INSERT INTO chunks VALUES (9, 'a'), (10, 'c');")
            .unwrap();
        m.top_up(&c).unwrap();
        assert_eq!(
            m.document_of(9),
            Some(a),
            "the same number for the same document"
        );
        assert!(m.number_of("c").is_some());
    }
}
