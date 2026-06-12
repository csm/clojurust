//! nREPL message types: decoded requests and response builders.

use std::collections::BTreeMap;

use crate::bencode::Bencode;

/// A decoded client request. Fields not present in the message are `None`;
/// unknown keys are ignored.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub op: String,
    pub id: Option<String>,
    pub session: Option<String>,
    /// `eval`: the code to evaluate.
    pub code: Option<String>,
    /// `eval` / `completions` / `lookup`: namespace context.
    pub ns: Option<String>,
    /// `load-file`: full file contents.
    pub file: Option<String>,
    /// `load-file`: display name for the file.
    pub file_name: Option<String>,
    /// `completions`: the prefix to complete.
    pub prefix: Option<String>,
    /// `lookup`: the symbol to look up.
    pub sym: Option<String>,
    /// `interrupt`: id of the eval message to interrupt.
    pub interrupt_id: Option<String>,
}

impl Request {
    /// Decode a request from a bencode dictionary. Returns `None` when the
    /// message has no `op` (nothing we could ever dispatch).
    pub fn from_bencode(msg: &Bencode) -> Option<Request> {
        let dict = msg.as_dict()?;
        let get = |key: &str| -> Option<String> {
            dict.get(key.as_bytes())
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        Some(Request {
            op: get("op")?,
            id: get("id"),
            session: get("session"),
            code: get("code"),
            ns: get("ns"),
            file: get("file"),
            file_name: get("file-name"),
            prefix: get("prefix"),
            sym: get("sym"),
            interrupt_id: get("interrupt-id"),
        })
    }
}

/// Builder for response messages. Every response echoes the request `id` and
/// the session it belongs to, per the nREPL protocol.
pub struct Response {
    entries: BTreeMap<Vec<u8>, Bencode>,
}

impl Response {
    pub fn for_request(req: &Request, session: &str) -> Response {
        let mut entries = BTreeMap::new();
        if let Some(id) = &req.id {
            entries.insert(b"id".to_vec(), Bencode::str(id));
        }
        entries.insert(b"session".to_vec(), Bencode::str(session));
        Response { entries }
    }

    pub fn field(mut self, key: &str, value: Bencode) -> Response {
        self.entries.insert(key.as_bytes().to_vec(), value);
        self
    }

    pub fn str_field(self, key: &str, value: impl AsRef<str>) -> Response {
        self.field(key, Bencode::str(value))
    }

    pub fn status(self, statuses: &[&str]) -> Response {
        self.field(
            "status",
            Bencode::List(statuses.iter().map(Bencode::str).collect()),
        )
    }

    pub fn build(self) -> Bencode {
        Bencode::Dict(self.entries)
    }
}
