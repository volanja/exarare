use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// One line of `events.jsonl`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Event {
    #[serde(with = "time::serde::rfc3339")]
    pub ts: OffsetDateTime,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    SessionStart,
    SessionEnd,
    /// A command line is about to run. It is paired with the next `CmdEnd`.
    CmdStart {
        cmd: String,
        cwd: String,
    },
    CmdEnd {
        exit_code: i32,
    },
    /// A heading or remark inserted by `exarare note`.
    Note {
        text: String,
    },
}

impl Event {
    pub fn now(kind: EventKind) -> Self {
        Self {
            ts: OffsetDateTime::now_utc(),
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_json() {
        let ev = Event::now(EventKind::CmdStart {
            cmd: "dnf install -y nginx".into(),
            cwd: "/root".into(),
        });
        let line = serde_json::to_string(&ev).unwrap();
        assert!(line.contains(r#""kind":"cmd_start""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(back.kind, ev.kind);
    }
}
