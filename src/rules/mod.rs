use regex::Regex;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_BODY_ID: AtomicU64 = AtomicU64::new(1);
#[derive(Debug)]
pub struct Response<'a> {
    pub status: u16,
    pub headers: &'a reqwest::header::HeaderMap,
    pub body: &'a [u8],
}
#[derive(Debug, Clone, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
pub struct HeaderCondition {
    pub name: String,
    #[serde(default)]
    pub exists: Option<bool>,
    pub equals: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct JsonCondition {
    pub pointer: String,
    pub equals: serde_json::Value,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub name: String,
    pub when: Condition,
    #[serde(default)]
    pub action: Action,
}
#[derive(Debug, Clone, Default, Deserialize)]
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
    pub fn validate(&self) -> Result<(), regex::Error> {
        match self {
            Self::All { all } => {
                for c in all {
                    c.validate()?
                }
            }
            Self::Any { any } => {
                for c in any {
                    c.validate()?
                }
            }
            Self::Not { not } => not.validate()?,
            Self::BodyRegex { body_regex } => Regex::new(body_regex)?,
            _ => {}
        }
        Ok(())
    }
    pub fn matches(&self, r: &Response<'_>) -> Result<bool, regex::Error> {
        Ok(match self {
            Self::All { all } => {
                for c in all {
                    if !c.matches(r)? {
                        return Ok(false);
                    }
                }
                true
            }
            Self::Any { any } => {
                for c in any {
                    if c.matches(r)? {
                        return Ok(true);
                    }
                }
                false
            }
            Self::Not { not } => !not.matches(r)?,
            Self::Status { status } => r.status == *status,
            Self::Header { header } => {
                let v = r.headers.get(&header.name);
                match (v, header.exists, &header.equals) {
                    (_, Some(false), _) => v.is_none(),
                    (None, _, _) => false,
                    (Some(v), _, Some(e)) => v.to_str().ok() == Some(e),
                    (Some(_), _, None) => true,
                }
            }
            Self::BodyContains { body_contains } => {
                String::from_utf8_lossy(r.body).contains(body_contains)
            }
            Self::BodyEquals { body_equals } => r.body == body_equals.as_bytes(),
            Self::BodyRegex { body_regex } => {
                Regex::new(body_regex)?.is_match(&String::from_utf8_lossy(r.body))
            }
            Self::Json { json } => {
                serde_json::from_slice::<serde_json::Value>(r.body)
                    .ok()
                    .and_then(|v| v.pointer(&json.pointer).cloned())
                    == Some(json.equals.clone())
            }
        })
    }
}
pub fn evaluate(rules: &[Rule], r: &Response<'_>) -> Result<Decision, regex::Error> {
    let mut d = Decision::default();
    for rule in rules {
        if rule.when.matches(r)? {
            d.matched.insert(rule.name.clone());
            d.rotate |= rule.action.rotate;
            d.stop |= rule.action.stop;
            d.save_body |= rule.action.save_body;
            if let Some(s) = &rule.action.scope {
                d.scopes.insert(s.clone());
            }
        }
    }
    Ok(d)
}
pub struct BodyStore {
    temporary: PathBuf,
    file: File,
    max_bytes: u64,
    written: u64,
}
impl BodyStore {
    pub fn create(directory: impl AsRef<Path>, max_bytes: u64) -> io::Result<Self> {
        let directory = directory.as_ref();
        fs::create_dir_all(directory)?;
        let (temporary, file) = loop {
            let id = NEXT_BODY_ID.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!(".body-{}-{id}.tmp", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(f) => break (path, f),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        Ok(Self {
            temporary,
            file,
            max_bytes,
            written: 0,
        })
    }
    pub fn write_chunk(&mut self, chunk: &[u8]) -> io::Result<()> {
        let next = self.written.saturating_add(chunk.len() as u64);
        if next > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "response body exceeds configured limit",
            ));
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
        let final_path = self.temporary.with_extension("bin");
        fs::rename(&self.temporary, &final_path)?;
        Ok(Some(final_path))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};
    fn r<'a>(h: &'a HeaderMap, b: &'a [u8]) -> Response<'a> {
        Response {
            status: 200,
            headers: h,
            body: b,
        }
    }
    #[test]
    fn predicates_composition_and_actions() {
        let mut h = HeaderMap::new();
        h.insert("content-type", HeaderValue::from_static("application/json"));
        let rules = vec![
            Rule {
                name: "json".into(),
                when: Condition::Json {
                    json: JsonCondition {
                        pointer: "/ok".into(),
                        equals: serde_json::Value::Bool(true),
                    },
                },
                action: Action {
                    rotate: true,
                    scope: Some("api".into()),
                    ..Default::default()
                },
            },
            Rule {
                name: "composed".into(),
                when: Condition::All {
                    all: vec![
                        Condition::Status { status: 200 },
                        Condition::Any {
                            any: vec![
                                Condition::Header {
                                    header: HeaderCondition {
                                        name: "content-type".into(),
                                        exists: None,
                                        equals: Some("application/json".into()),
                                    },
                                },
                                Condition::BodyRegex {
                                    body_regex: "missing".into(),
                                },
                            ],
                        },
                        Condition::Not {
                            not: Box::new(Condition::BodyContains {
                                body_contains: "failure".into(),
                            }),
                        },
                    ],
                },
                action: Action {
                    stop: true,
                    save_body: true,
                    ..Default::default()
                },
            },
        ];
        let d = evaluate(&rules, &r(&h, br#"{"ok":true}"#)).unwrap();
        assert_eq!(
            d.matched.into_iter().collect::<Vec<_>>(),
            vec!["composed", "json"]
        );
        assert!(d.rotate && d.stop && d.save_body && d.scopes.contains("api"))
    }
    #[test]
    fn invalid_regex_and_limit_error() {
        let bad = Condition::BodyRegex {
            body_regex: "(".into(),
        };
        assert!(bad.validate().is_err());
        let h = HeaderMap::new();
        assert!(
            evaluate(
                &[Rule {
                    name: "bad".into(),
                    when: bad,
                    action: Default::default()
                }],
                &r(&h, b"x")
            )
            .is_err()
        );
        let d = std::env::temp_dir().join(format!(
            "aethel-rules-{}",
            NEXT_BODY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut s = BodyStore::create(&d, 2).unwrap();
        assert!(s.write_chunk(b"abc").is_err());
        drop(s);
        let _ = fs::remove_dir_all(d);
    }
    #[test]
    fn body_store_cleanup_and_unique_output() {
        let d = std::env::temp_dir().join(format!(
            "aethel-rules-{}",
            NEXT_BODY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut a = BodyStore::create(&d, 10).unwrap();
        a.write_chunk(b"one").unwrap();
        assert!(a.finish(false).unwrap().is_none());
        let mut b = BodyStore::create(&d, 10).unwrap();
        b.write_chunk(b"two").unwrap();
        let p = b.finish(true).unwrap().unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        fs::remove_dir_all(d).unwrap()
    }
}
