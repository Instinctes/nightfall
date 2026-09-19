//! Named receive addresses. Local only — never on chain.

use nightfall_crypto::Address;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddressBookEntry {
    pub name: String,
    pub address: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AddressBook {
    #[serde(default)]
    pub entries: Vec<AddressBookEntry>,
}

fn path(datadir: &Path) -> PathBuf {
    datadir.join("address_book.json")
}

impl AddressBook {
    pub fn load(datadir: &Path) -> Self {
        std::fs::read_to_string(path(datadir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, datadir: &Path) -> anyhow::Result<()> {
        let tmp = path(datadir).with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path(datadir))?;
        Ok(())
    }

    /// Longest contact name kept. A label that does not fit on its row is not
    /// a label, and the Settings row is sized from the window, not the name.
    pub const MAX_NAME: usize = 40;

    pub fn add(&mut self, name: String, address: String) -> Result<(), String> {
        let name = name.trim().to_string();
        let address = address.trim().to_string();
        if name.is_empty() {
            return Err("Give the contact a name".into());
        }
        if name.chars().count() > Self::MAX_NAME {
            return Err(format!(
                "Keep the name under {} characters — it has to fit on one row.",
                Self::MAX_NAME
            ));
        }
        Address::decode(&address).map_err(|e| e.to_string())?;
        if self.entries.iter().any(|e| e.address == address) {
            return Err("That address is already in the book".into());
        }
        self.entries.push(AddressBookEntry { name, address });
        Ok(())
    }

    pub fn remove(&mut self, address: &str) {
        self.entries.retain(|e| e.address != address);
    }
}

#[cfg(test)]
mod tests {
    use super::AddressBook;

    #[test]
    fn rejects_empty_name_and_keeps_order() {
        let mut book = AddressBook::default();
        assert!(book.add("".into(), "nf1dead".into()).is_err());
        assert!(book.entries.is_empty());
    }
}
