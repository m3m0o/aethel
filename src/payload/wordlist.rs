use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

pub struct WordlistReader {
    reader: BufReader<File>,
}

impl WordlistReader {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::new(File::open(path)?),
        })
    }

    pub fn next_value(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line.ends_with('\n') {
            line.pop();
        }
        if line.ends_with('\r') {
            line.pop();
        }
        Ok(Some(line))
    }
}

pub struct Clusterbomb {
    names: Vec<String>,
    paths: Vec<PathBuf>,
    readers: Vec<WordlistReader>,
    current: Vec<String>,
    started: bool,
    finished: bool,
}

impl Clusterbomb {
    pub fn open(wordlists: BTreeMap<String, PathBuf>) -> io::Result<Self> {
        let (names, paths): (Vec<_>, Vec<_>) = wordlists.into_iter().unzip();
        let readers = paths
            .iter()
            .map(WordlistReader::open)
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            names,
            paths,
            readers,
            current: Vec::new(),
            started: false,
            finished: false,
        })
    }

    pub fn next_payload(&mut self) -> io::Result<Option<BTreeMap<String, String>>> {
        if self.finished || self.names.is_empty() {
            return Ok(None);
        }
        if !self.started {
            self.started = true;
            let mut current = Vec::with_capacity(self.readers.len());
            for reader in &mut self.readers {
                let Some(value) = reader.next_value()? else {
                    self.finished = true;
                    return Ok(None);
                };
                current.push(value);
            }
            self.current = current;
        } else if !self.advance()? {
            self.finished = true;
            return Ok(None);
        }
        Ok(Some(self.current_map()))
    }

    fn advance(&mut self) -> io::Result<bool> {
        for index in (0..self.readers.len()).rev() {
            if let Some(value) = self.readers[index].next_value()? {
                self.current[index] = value;
                for reset in index + 1..self.readers.len() {
                    self.readers[reset] = WordlistReader::open(&self.paths[reset])?;
                    self.current[reset] = self.readers[reset].next_value()?.ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "empty wordlist")
                    })?;
                }
                return Ok(true);
            }
            self.readers[index] = WordlistReader::open(&self.paths[index])?;
            self.current[index] = self.readers[index]
                .next_value()?
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty wordlist"))?;
        }
        Ok(false)
    }

    fn current_map(&self) -> BTreeMap<String, String> {
        self.names
            .iter()
            .cloned()
            .zip(self.current.iter().cloned())
            .collect()
    }
}

pub struct Pitchfork {
    names: Vec<String>,
    readers: Vec<WordlistReader>,
}

impl Pitchfork {
    pub fn open(wordlists: BTreeMap<String, PathBuf>) -> io::Result<Self> {
        let (names, paths): (Vec<_>, Vec<_>) = wordlists.into_iter().unzip();
        let readers = paths
            .iter()
            .map(WordlistReader::open)
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { names, readers })
    }

    pub fn next_payload(&mut self) -> io::Result<Option<BTreeMap<String, String>>> {
        if self.names.is_empty() {
            return Ok(None);
        }
        let mut values = Vec::with_capacity(self.readers.len());
        for reader in &mut self.readers {
            let Some(value) = reader.next_value()? else {
                return Ok(None);
            };
            values.push(value);
        }
        Ok(Some(self.names.iter().cloned().zip(values).collect()))
    }
}

pub fn validate_wordlists(
    placeholders: &std::collections::BTreeSet<String>,
    wordlists: &BTreeMap<String, PathBuf>,
) -> io::Result<()> {
    for placeholder in placeholders {
        if !wordlists.contains_key(placeholder) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("placeholder has no wordlist: {placeholder}"),
            ));
        }
    }
    for name in wordlists.keys() {
        if !placeholders.contains(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("wordlist is not used by a placeholder: {name}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_wordlists, Clusterbomb, Pitchfork, WordlistReader};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("aethel-{name}-{}", std::process::id()));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn strips_only_line_endings_and_preserves_spaces() {
        let path = fixture("wordlist", " value \r\n");
        let mut reader = WordlistReader::open(&path).unwrap();
        assert_eq!(reader.next_value().unwrap().as_deref(), Some(" value "));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn clusterbomb_is_deterministic_without_materializing_product() {
        let first = fixture("cluster-a", "a\nb\n");
        let second = fixture("cluster-b", "1\n2\n");
        let mut lists = BTreeMap::new();
        lists.insert("a".to_owned(), first.clone());
        lists.insert("b".to_owned(), second.clone());
        let mut generator = Clusterbomb::open(lists).unwrap();
        let mut values = Vec::new();
        while let Some(payload) = generator.next_payload().unwrap() {
            values.push(payload);
        }
        assert_eq!(values.len(), 4);
        assert_eq!(values[0]["a"], "a");
        assert_eq!(values[0]["b"], "1");
        assert_eq!(values[3]["a"], "b");
        assert_eq!(values[3]["b"], "2");
        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }

    #[test]
    fn pitchfork_stops_at_shortest_list() {
        let first = fixture("pitch-a", "a\nb\n");
        let second = fixture("pitch-b", "1\n");
        let mut lists = BTreeMap::new();
        lists.insert("a".to_owned(), first.clone());
        lists.insert("b".to_owned(), second.clone());
        let mut generator = Pitchfork::open(lists).unwrap();
        assert!(generator.next_payload().unwrap().is_some());
        assert!(generator.next_payload().unwrap().is_none());
        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }
    #[test]
    fn empty_lines_and_unicode_are_valid_payloads() {
        let path = fixture("empty-unicode", "\nこんにちは\n");
        let mut reader = WordlistReader::open(&path).unwrap();
        assert_eq!(reader.next_value().unwrap().as_deref(), Some(""));
        assert_eq!(reader.next_value().unwrap().as_deref(), Some("こんにちは"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn empty_wordlist_produces_no_payloads() {
        let path = fixture("empty-list", "");
        let mut lists = BTreeMap::new();
        lists.insert("value".to_owned(), path.clone());
        let mut generator = Clusterbomb::open(lists).unwrap();
        assert!(generator.next_payload().unwrap().is_none());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn validates_placeholder_and_wordlist_names() {
        let path = fixture("validation", "value\n");
        let mut placeholders = BTreeSet::new();
        placeholders.insert("missing".to_owned());
        let mut wordlists = BTreeMap::new();
        wordlists.insert("value".to_owned(), path.clone());
        assert!(validate_wordlists(&placeholders, &wordlists).is_err());

        placeholders.clear();
        placeholders.insert("value".to_owned());
        wordlists.insert("unused".to_owned(), path.clone());
        assert!(validate_wordlists(&placeholders, &wordlists).is_err());
        fs::remove_file(path).unwrap();
    }
}
