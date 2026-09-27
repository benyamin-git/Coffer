use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;

pub const CONTENT_VERSION: u32 = 1;
pub const TITLE_MAX: usize = 80;

#[derive(Debug, Error)]
pub enum NoteError {
    #[error("no note matches '{0}'")]
    NotFound(String),
    #[error("'{0}' matches multiple notes; use the numeric id")]
    Ambiguous(String),
    #[error("note {0} does not exist")]
    NoSuchId(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub created: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notebook {
    pub version: u32,
    pub next_id: u64,
    pub notes: Vec<Note>,
}

impl Default for Notebook {
    fn default() -> Self {
        Self {
            version: CONTENT_VERSION,
            next_id: 1,
            notes: Vec::new(),
        }
    }
}

impl Notebook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, title: &str, body: &str) -> u64 {
        let now = now_rfc3339();
        let id = self.next_id;
        self.next_id += 1;
        self.notes.push(Note {
            id,
            title: title.to_string(),
            body: body.to_string(),
            created: now.clone(),
            updated: now,
        });
        id
    }

    pub fn update(&mut self, id: u64, title: &str, body: &str) -> Result<(), NoteError> {
        let note = self.get_mut(id).ok_or(NoteError::NoSuchId(id))?;
        note.title = title.to_string();
        note.body = body.to_string();
        note.updated = now_rfc3339();
        Ok(())
    }

    pub fn remove(&mut self, id: u64) -> Result<Note, NoteError> {
        let pos = self
            .notes
            .iter()
            .position(|n| n.id == id)
            .ok_or(NoteError::NoSuchId(id))?;
        Ok(self.notes.remove(pos))
    }

    pub fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    /// Resolve a user-supplied query to a note id. Accepts a numeric id or an
    /// exact (case-insensitive) title.
    pub fn resolve(&self, query: &str) -> Result<u64, NoteError> {
        if let Ok(id) = query.trim().parse::<u64>() {
            if self.get(id).is_some() {
                return Ok(id);
            }
            return Err(NoteError::NoSuchId(id));
        }
        let matches: Vec<&Note> = self
            .notes
            .iter()
            .filter(|n| n.title.eq_ignore_ascii_case(query.trim()))
            .collect();
        match matches.as_slice() {
            [] => Err(NoteError::NotFound(query.to_string())),
            [only] => Ok(only.id),
            _ => Err(NoteError::Ambiguous(query.to_string())),
        }
    }

    pub fn search(&self, term: &str) -> Vec<&Note> {
        let needle = term.to_lowercase();
        self.notes
            .iter()
            .filter(|n| {
                n.title.to_lowercase().contains(&needle) || n.body.to_lowercase().contains(&needle)
            })
            .collect()
    }
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Derive a title from a note body: first non-empty line, truncated.
pub fn title_from_body(body: &str) -> String {
    for line in body.lines() {
        let line = line.trim();
        if !line.is_empty() {
            return truncate_chars(line, TITLE_MAX);
        }
    }
    "untitled".to_string()
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in s.chars().take(max) {
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notebook() -> Notebook {
        let mut nb = Notebook::new();
        nb.add("shopping", "milk\nbread");
        nb.add("ideas", "build a coffer");
        nb
    }

    #[test]
    fn add_assigns_increasing_ids() {
        let nb = notebook();
        assert_eq!(nb.notes[0].id, 1);
        assert_eq!(nb.notes[1].id, 2);
        assert_eq!(nb.next_id, 3);
    }

    #[test]
    fn resolve_by_id_and_title() {
        let nb = notebook();
        assert_eq!(nb.resolve("2").unwrap(), 2);
        assert_eq!(nb.resolve("SHOPPING").unwrap(), 1);
        assert!(matches!(nb.resolve("nope"), Err(NoteError::NotFound(_))));
        assert!(matches!(nb.resolve("99"), Err(NoteError::NoSuchId(99))));
    }

    #[test]
    fn resolve_reports_ambiguous_titles() {
        let mut nb = Notebook::new();
        nb.add("same", "a");
        nb.add("same", "b");
        assert!(matches!(nb.resolve("same"), Err(NoteError::Ambiguous(_))));
    }

    #[test]
    fn search_is_case_insensitive() {
        let nb = notebook();
        assert_eq!(nb.search("MILK").len(), 1);
        assert_eq!(nb.search("coffer").len(), 1);
        assert_eq!(nb.search("zzz").len(), 0);
    }

    #[test]
    fn title_derivation() {
        assert_eq!(title_from_body("\n\n  hello world  \nrest"), "hello world");
        assert_eq!(title_from_body("   \n\n"), "untitled");
        let long = "x".repeat(TITLE_MAX + 20);
        assert_eq!(title_from_body(&long).chars().count(), TITLE_MAX);
    }
}
