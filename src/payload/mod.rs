mod wordlist;

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use crate::config::PayloadMode;

pub use wordlist::{Clusterbomb, Pitchfork, WordlistReader, validate_wordlists};

pub enum Generator {
    Clusterbomb(Clusterbomb),
    Pitchfork(Pitchfork),
    Single(bool),
}

impl Generator {
    pub fn open(mode: PayloadMode, wordlists: BTreeMap<String, PathBuf>) -> io::Result<Self> {
        if wordlists.is_empty() {
            return Ok(Self::Single(false));
        }
        match mode {
            PayloadMode::Clusterbomb => Ok(Self::Clusterbomb(Clusterbomb::open(wordlists)?)),
            PayloadMode::Pitchfork => Ok(Self::Pitchfork(Pitchfork::open(wordlists)?)),
        }
    }

    pub fn next_payload(&mut self) -> io::Result<Option<BTreeMap<String, String>>> {
        match self {
            Self::Clusterbomb(generator) => generator.next_payload(),
            Self::Pitchfork(generator) => generator.next_payload(),
            Self::Single(done) => {
                if *done {
                    Ok(None)
                } else {
                    *done = true;
                    Ok(Some(BTreeMap::new()))
                }
            }
        }
    }
}
