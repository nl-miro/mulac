pub mod prelude {
    #[cfg(feature = "diesel")]
    pub(crate) use super::payload;
    pub(crate) use super::validate_batch;
    pub use super::{RecordingError, RecordingOutcome};
}

type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What one recording attempt established about the outbox rows it was asked
/// to add. See the Failures table in the bulk recording plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingOutcome {
    /// Nothing was written, or the attempt's writes were rolled back.
    NoRowsAdded,
    /// A commit, rollback or release failed, so rows may or may not exist.
    Uncertain,
}

const DISPLAY_LIMIT: usize = 200;
const DISPLAY_IDS: usize = 3;

fn bounded(source: &Source) -> String {
    let text = source.to_string();
    match text.char_indices().nth(DISPLAY_LIMIT) {
        Some((end, _)) => format!("{}...", &text[..end]),
        None => text,
    }
}

fn bounded_ids(ids: &[uuid::Uuid]) -> String {
    let shown: Vec<_> = ids
        .iter()
        .take(DISPLAY_IDS)
        .map(ToString::to_string)
        .collect();
    match ids.len().saturating_sub(DISPLAY_IDS) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecordingError {
    #[error("duplicate event ID {0}; no rows added")]
    DuplicateEventId(uuid::Uuid),
    #[error("entry ID {entry_id} differs from metadata event ID {event_id}; no rows added")]
    MismatchedEventId {
        entry_id: uuid::Uuid,
        event_id: uuid::Uuid,
    },
    #[error("invalid payload for event {0}; no rows added")]
    InvalidPayload(uuid::Uuid),
    #[error("invalid metadata for event {event_id}: {field}; no rows added")]
    InvalidMetadata {
        event_id: uuid::Uuid,
        field: &'static str,
    },
    #[error("replay conflict for event {event_id}: {field}; no rows added")]
    ReplayConflict {
        event_id: uuid::Uuid,
        field: &'static str,
    },
    #[error("no open transaction; no rows added")]
    NoOpenTransaction,
    #[error(
        "events neither inserted nor found afterwards: {}; no rows added",
        bounded_ids(.0)
    )]
    MissingEvents(Vec<uuid::Uuid>),
    #[error("no recording connection: {}; no rows added", bounded(.0))]
    NoConnection(#[source] Source),
    #[error("database failure: {}; no rows added", bounded(.0))]
    DatabaseRolledBack(#[source] Source),
    #[error("database failure: {}; outcome uncertain", bounded(.0))]
    DatabaseUncertain(#[source] Source),
}

impl RecordingError {
    /// The outcome of the attempt that produced this error.
    pub fn outcome(&self) -> RecordingOutcome {
        match self {
            Self::DatabaseUncertain(_) => RecordingOutcome::Uncertain,
            _ => RecordingOutcome::NoRowsAdded,
        }
    }
}

// Token spelling is retained independently of serde_json's numeric representation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Json {
    Atom(String),
    String(String),
    Array(Vec<Json>),
    Object(std::collections::BTreeMap<String, Json>),
}
pub(crate) fn payload(input: &str) -> Result<Json, ()> {
    struct Parser<'a> {
        s: &'a str,
        i: usize,
    }
    impl Parser<'_> {
        fn ws(&mut self) {
            while self
                .s
                .as_bytes()
                .get(self.i)
                .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r' | b'\n'))
            {
                self.i += 1;
            }
        }
        fn string(&mut self) -> Result<String, ()> {
            let start = self.i;
            self.i += 1;
            loop {
                match self.s.as_bytes().get(self.i).ok_or(())? {
                    b'"' => {
                        self.i += 1;
                        return serde_json::from_str(&self.s[start..self.i]).map_err(|_| ());
                    }
                    b'\\' => self.i += 2,
                    _ => self.i += 1,
                }
            }
        }
        fn value(&mut self, depth: usize) -> Result<Json, ()> {
            self.ws();
            let c = *self.s.as_bytes().get(self.i).ok_or(())?;
            if c == b'"' {
                return Ok(Json::String(self.string()?));
            }
            if c == b'{' || c == b'[' {
                if depth >= 127 {
                    return Err(());
                }
                self.i += 1;
                self.ws();
                let mut object = std::collections::BTreeMap::new();
                let mut array = Vec::new();
                let end = if c == b'{' { b'}' } else { b']' };
                if self.s.as_bytes().get(self.i) != Some(&end) {
                    loop {
                        if c == b'{' {
                            self.ws();
                            if self.s.as_bytes().get(self.i) != Some(&b'"') {
                                return Err(());
                            }
                            let key = self.string()?;
                            self.ws();
                            if self.s.as_bytes().get(self.i) != Some(&b':') {
                                return Err(());
                            }
                            self.i += 1;
                            if object.insert(key, self.value(depth + 1)?).is_some() {
                                return Err(());
                            }
                        } else {
                            array.push(self.value(depth + 1)?);
                        }
                        self.ws();
                        if self.s.as_bytes().get(self.i) != Some(&b',') {
                            break;
                        }
                        self.i += 1;
                    }
                }
                if self.s.as_bytes().get(self.i) != Some(&end) {
                    return Err(());
                }
                self.i += 1;
                return Ok(if c == b'{' {
                    Json::Object(object)
                } else {
                    Json::Array(array)
                });
            }
            let start = self.i;
            while self
                .s
                .as_bytes()
                .get(self.i)
                .is_some_and(|c| !matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}'))
            {
                self.i += 1;
            }
            let token = &self.s[start..self.i];
            if !matches!(token, "true" | "false" | "null") {
                let b = token.as_bytes();
                let mut n = 0;
                if b.get(n) == Some(&b'-') {
                    n += 1;
                }
                match b.get(n) {
                    Some(b'0') => n += 1,
                    Some(b'1'..=b'9') => {
                        n += 1;
                        while b.get(n).is_some_and(u8::is_ascii_digit) {
                            n += 1;
                        }
                    }
                    _ => return Err(()),
                }
                if b.get(n) == Some(&b'.') {
                    n += 1;
                    let p = n;
                    while b.get(n).is_some_and(u8::is_ascii_digit) {
                        n += 1;
                    }
                    if n == p {
                        return Err(());
                    }
                }
                if matches!(b.get(n), Some(b'e' | b'E')) {
                    n += 1;
                    if matches!(b.get(n), Some(b'+' | b'-')) {
                        n += 1;
                    }
                    let p = n;
                    while b.get(n).is_some_and(u8::is_ascii_digit) {
                        n += 1;
                    }
                    if n == p {
                        return Err(());
                    }
                }
                if n != b.len() {
                    return Err(());
                }
            }
            Ok(Json::Atom(token.into()))
        }
    }
    let mut p = Parser { s: input, i: 0 };
    let value = p.value(0)?;
    p.ws();
    if p.i != input.len() || !matches!(value, Json::Object(_)) {
        return Err(());
    }
    Ok(value)
}

pub(crate) fn validate_batch(
    entries: &[crate::application::prelude::NewOutboxEntry],
) -> Result<(), RecordingError> {
    let mut ids = std::collections::HashSet::new();
    for e in entries {
        if !ids.insert(e.id) {
            return Err(RecordingError::DuplicateEventId(e.id));
        }
        if e.id != e.meta.event_id {
            return Err(RecordingError::MismatchedEventId {
                entry_id: e.id,
                event_id: e.meta.event_id,
            });
        }
        payload(&e.payload).map_err(|_| RecordingError::InvalidPayload(e.id))?;
        let m = &e.meta;
        for (field, value) in [
            ("routing_key", Some(m.routing_key.as_str())),
            ("event_type", Some(m.event_type.as_str())),
            ("source", m.source.as_deref()),
            ("content_type", m.content_type.as_deref()),
        ] {
            if value.is_some_and(|s| s.contains('\0'))
                || (field == "routing_key" && m.routing_key.trim().is_empty())
            {
                return Err(RecordingError::InvalidMetadata {
                    event_id: e.id,
                    field,
                });
            }
        }
        if super::prelude::validate_sequence(m.sequence).is_err() {
            return Err(RecordingError::InvalidMetadata {
                event_id: e.id,
                field: "sequence",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::payload;
    #[test]
    fn equivalence() {
        assert_eq!(
            payload(r#"{"a":1,"b":[true,null]}"#),
            payload(r#" { "b":[true,null], "\u0061":1 } "#)
        );
        for (a, b) in [("1", "1.0"), ("1e2", "1E2"), ("1e2", "1e+2"), ("0", "-0")] {
            assert_ne!(
                payload(&format!("{{\"n\":{a}}}")),
                payload(&format!("{{\"n\":{b}}}"))
            );
        }
    }
    #[test]
    fn escaped_and_literal_spellings_of_keys_and_values_are_equivalent() {
        for (a, b) in [
            (r#"{"a":1}"#, r#"{"\u0061":1}"#),
            (r#"{"a":"x"}"#, r#"{"a":"\u0078"}"#),
            (r#"{"s":"é"}"#, r#"{"s":"\u00e9"}"#),
            (r#"{"s":"\u00e9"}"#, r#"{"s":"\u00E9"}"#),
            (r#"{"s":"😀"}"#, r#"{"s":"\ud83d\ude00"}"#),
            (r#"{"s":"\ud83d\ude00"}"#, r#"{"s":"\uD83D\uDE00"}"#),
            (r#"{"é":1}"#, r#"{"\u00e9":1}"#),
            (r#"{"😀":1}"#, r#"{"\ud83d\ude00":1}"#),
        ] {
            assert!(payload(a).is_ok(), "{a}");
            assert_eq!(payload(a), payload(b), "{a} ~ {b}");
        }
        for (a, b) in [
            (r#"{"s":"\u00e9"}"#, r#"{"s":"e\u0301"}"#),
            (r#"{"s":"é"}"#, r#"{"s":"\u00e8"}"#),
            (r#"{"s":"\ud83d\ude00"}"#, r#"{"s":"\ud83d\ude01"}"#),
            (r#"{"a":1}"#, r#"{"\u0062":1}"#),
        ] {
            assert_ne!(payload(a), payload(b), "{a} vs {b}");
        }
    }
    #[test]
    fn duplicate_keys_are_found_across_escape_spellings() {
        for s in [
            r#"{"a":1,"\u0061":2}"#,
            r#"{"\u0061":1,"a":2}"#,
            r#"{"\u0061":1,"\u0061":2}"#,
            r#"{"é":1,"\u00e9":2}"#,
            r#"{"\u00e9":1,"\u00E9":2}"#,
            r#"{"😀":1,"\ud83d\ude00":2}"#,
            r#"{"a/b":1,"a\/b":2}"#,
            r#"{"o":{"\u0061":1,"a":1}}"#,
        ] {
            assert!(payload(s).is_err(), "{s}");
        }
    }
    #[test]
    fn invalid() {
        for s in [
            r#"{"a":1,"\u0061":2}"#,
            r#"{"s":"\ud800"}"#,
            r#"{"s":"\udc00"}"#,
            r#"{"n":01}"#,
            r#"{"n":1.}"#,
            r#"{"n":+1}"#,
            r#"{"n":1e}"#,
            "[]",
            "null",
            "{} {}",
            "\u{feff}{}",
            "{}\u{a0}",
            "{\"x\":\"\0\"}",
            "{\"a\":true,}",
        ] {
            assert!(payload(s).is_err(), "{s:?}");
        }
    }
    #[test]
    fn accepted() {
        for s in [
            r#"{"n":1234567890123456789012345678901234567890}"#,
            r#"{"n":1e999999999999999999999}"#,
            r#"{"\u0000":"\u0000"}"#,
            r#"{"x":"\ud83d\ude00"}"#,
            r#"{"a":{"x":1},"b":{"x":2}}"#,
            r#"{"$serde_json::private::Number":"x"}"#,
        ] {
            assert!(payload(s).is_ok());
        }
    }
    #[test]
    fn container_boundary() {
        assert!(
            payload(&format!(
                "{{\"x\":{}0{}}}",
                "[".repeat(126),
                "]".repeat(126)
            ))
            .is_ok()
        );
        assert!(
            payload(&format!(
                "{{\"x\":{}0{}}}",
                "[".repeat(127),
                "]".repeat(127)
            ))
            .is_err()
        );
    }
}
