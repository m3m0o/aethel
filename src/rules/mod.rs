use regex::Regex;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Response<'a> {
    pub status: u16,
    pub headers: &'a reqwest::header::HeaderMap,
    pub body: &'a [u8],
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Condition {
    All { all: Vec<Condition> },
    Any { any: Vec<Condition> },
    Not { not: Box<Condition> },
    Status { status: u16 },
    Header { header: HeaderCondition },
    BodyContains { body_contains: String },
    BodyEquals { body_equals: String },
    BodyRegex { body_regex: String },
    Json { json: JsonCondition },
}

#[derive(Debug, Deserialize)]
pub struct HeaderCondition {
    pub name: String,
    #[serde(default)]
    pub exists: Option<bool>,
    pub equals: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JsonCondition {
    pub pointer: String,
    pub equals: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    pub name: String,
    pub when: Condition,
    #[serde(default)]
    pub action: Action,
}

#[derive(Debug, Default, Deserialize)]
pub struct Action {
    #[serde(default)]
    pub rotate: bool,
    #[serde(default)]
    pub stop: bool,
    #[serde(default)]
    pub save_body: bool,
    pub scope: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub matched: BTreeSet<String>,
    pub rotate: bool,
    pub stop: bool,
    pub save_body: bool,
    pub scopes: BTreeSet<String>,
}

impl Condition {
    pub fn matches(&self, response: &Response<'_>) -> bool {
        match self {
            Self::All { all } => all.iter().all(|condition| condition.matches(response)),
            Self::Any { any } => any.iter().any(|condition| condition.matches(response)),
            Self::Not { not } => !not.matches(response),
            Self::Status { status } => response.status == *status,
            Self::Header { header } => {
                let value = response.headers.get(&header.name);
                match (value, header.exists, &header.equals) {
                    (_, Some(exists), _) if !exists => value.is_none(),
                    (None, _, _) => false,
                    (Some(_), Some(false), _) => false,
                    (Some(value), _, Some(expected)) => value.to_str().ok() == Some(expected),
                    (Some(_), _, None) => true,
                }
            }
            Self::BodyContains { body_contains } => {
                String::from_utf8_lossy(response.body).contains(body_contains)
            }
            Self::BodyEquals { body_equals } => response.body == body_equals.as_bytes(),
            Self::BodyRegex { body_regex } => Regex::new(body_regex)
                .map(|regex| regex.is_match(&String::from_utf8_lossy(response.body)))
                .unwrap_or(false),
            Self::Json { json } => serde_json::from_slice::<serde_json::Value>(response.body)
                .ok()
                .and_then(|value| value.pointer(&json.pointer))
                == Some(&json.equals),
        }
    }
}

pub fn evaluate(rules: &[Rule], response: &Response<'_>) -> Decision {
    let mut decision = Decision::default();
    for rule in rules {
        if rule.when.matches(response) {
            decision.matched.insert(rule.name.clone());
            decision.rotate |= rule.action.rotate;
            decision.stop |= rule.action.stop;
            decision.save_body |= rule.action.save_body;
            if let Some(scope) = &rule.action.scope {
                decision.scopes.insert(scope.clone());
            }
        }
    }
    decision
}

pub struct BodyStore {
    temporary: PathBuf,
    output_dir: PathBuf,
    file: File,
    max_bytes: u64,
    written: u64,
}

impl BodyStore {
    pub fn create(directory: impl AsRef<Path>, max_bytes: u64) -> io::Result<Self> {
        let output_dir = directory.as_ref().to_path_buf();
        fs::create_dir_all(&output_dir)?;
        let temporary = output_dir.join(format!(".body-{}.tmp", std::process::id()));
        let file = File::create(&temporary)?;
        Ok(Self { temporary, output_dir, file, max_bytes, written: 0 })
    }

    pub fn write_chunk(&mut self, chunk: &[u8]) -> io::Result<()> {
        let next = self.written.saturating_add(chunk.len() as u64);
        if next > self.max_bytes {
            return Err(io::Error::new(io::ErrorKind::FileTooLarge, "response body exceeds configured limit"));
        }
        self.file.write_all(chunk)?;
        self.written = next;
        Ok(())
    }

    pub fn finish(mut self, matched: bool) -> io::Result<Option<PathBuf>> {
        self.file.flush()?;
        drop(self.file);
        if !matched {
            fs::remove_file(&self.temporary)?;
            return Ok(None);
        }
        let final_path = self.output_dir.join(format!("body-{}.bin", std::process::id()));
        fs::rename(&self.temporary, &final_path)?;
        Ok(Some(final_path))
    }
}
