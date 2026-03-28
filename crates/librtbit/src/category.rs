use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::bail;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentCategory {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_path: Option<PathBuf>,
}

#[derive(Debug)]
pub struct CategoryManager {
    categories: RwLock<HashMap<String, TorrentCategory>>,
}

impl Default for CategoryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CategoryManager {
    pub fn new() -> Self {
        Self {
            categories: RwLock::new(HashMap::new()),
        }
    }

    pub fn from_categories(cats: HashMap<String, TorrentCategory>) -> Self {
        Self {
            categories: RwLock::new(cats),
        }
    }

    pub fn list(&self) -> HashMap<String, TorrentCategory> {
        self.categories.read().clone()
    }

    pub fn create(&self, name: String, save_path: Option<PathBuf>) -> anyhow::Result<()> {
        let mut cats = self.categories.write();
        if cats.contains_key(&name) {
            bail!("category '{}' already exists", name);
        }
        cats.insert(name.clone(), TorrentCategory { name, save_path });
        Ok(())
    }

    pub fn edit(&self, name: &str, save_path: Option<PathBuf>) -> anyhow::Result<()> {
        let mut cats = self.categories.write();
        let cat = cats.get_mut(name);
        match cat {
            Some(cat) => {
                cat.save_path = save_path;
                Ok(())
            }
            None => bail!("category '{}' does not exist", name),
        }
    }

    pub fn remove(&self, name: &str) -> anyhow::Result<()> {
        let mut cats = self.categories.write();
        if cats.remove(name).is_none() {
            bail!("category '{}' does not exist", name);
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<TorrentCategory> {
        self.categories.read().get(name).cloned()
    }

    pub fn exists(&self, name: &str) -> bool {
        self.categories.read().contains_key(name)
    }

    pub fn snapshot(&self) -> HashMap<String, TorrentCategory> {
        self.categories.read().clone()
    }
}
