use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, Serialize)]
pub struct ResultRecord<'a> {
    pub index: u64,
    pub worker: usize,
    pub payload: &'a std::collections::BTreeMap<String, String>,
    pub status: Option<u16>,
    pub duration_ms: u64,
    pub source_ipv6: Option<String>,
    pub matched_rules: &'a [String],
    pub body_file: Option<String>,
    pub error: Option<String>,
}

pub struct JsonlWriter {
    file: Mutex<std::fs::File>,
}
impl JsonlWriter {
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())
            .with_context(|| format!("failed to open JSONL output {}", path.as_ref().display()))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
    pub fn write<T: Serialize>(&self, record: &T) -> Result<()> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("JSONL writer mutex poisoned"))?;
        serde_json::to_writer(&mut *file, record).context("failed to serialize JSONL result")?;
        file.write_all(b"\n")
            .context("failed to write JSONL line")?;
        file.flush().context("failed to flush JSONL output")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::JsonlWriter;
    use serde::Serialize;
    use std::fs;
    #[derive(Serialize)]
    struct Record {
        index: u64,
        status: u16,
    }
    #[test]
    fn writes_one_json_object_per_line() {
        let path = std::env::temp_dir().join(format!("aethel-jsonl-{}", std::process::id()));
        let writer = JsonlWriter::create(&path).unwrap();
        writer
            .write(&Record {
                index: 42,
                status: 200,
            })
            .unwrap();
        drop(writer);
        let text = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(value["index"], 42);
        assert_eq!(value["status"], 200);
        fs::remove_file(path).unwrap();
    }
}
