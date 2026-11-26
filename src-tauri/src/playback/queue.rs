//! Stable queue identity and separate original and playback orders.
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntryId(pub(super) u64);

impl EntryId {
    pub fn parse(value: &str) -> Option<Self> {
        let id = value.parse::<u64>().ok()?;
        (id != 0 && id.to_string() == value).then_some(Self(id))
    }

    pub fn decimal(self) -> String {
        self.0.to_string()
    }

    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    One,
    All,
}

/// Library identity and display text for a queue row that came from the
/// index. The path stays the authority for playback; this only labels it and
/// lets a failed open be reported back to the library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackRef {
    pub track_id: u32,
    pub title: String,
    pub artist: String,
    pub artwork_id: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: EntryId,
    path: PathBuf,
    /// Absent in sessions saved before the library existed.
    #[serde(default)]
    track: Option<TrackRef>,
}

impl Entry {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn track(&self) -> Option<&TrackRef> {
        self.track.as_ref()
    }
}

/// A playable item before it has a queue identity.
pub type Item = (PathBuf, Option<TrackRef>);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Queue {
    entries: Vec<Entry>,
    order: Vec<EntryId>,
    current: Option<EntryId>,
    last_id: u64,
    shuffled: bool,
    pub repeat: Repeat,
}

impl Queue {
    pub fn append(&mut self, path: PathBuf) -> Option<EntryId> {
        self.append_item((path, None))
    }

    pub fn append_item(&mut self, (path, track): Item) -> Option<EntryId> {
        if self.entries.len() >= MAX_ENTRIES {
            return None;
        }
        self.last_id = self.last_id.checked_add(1)?;
        let id = EntryId(self.last_id);
        self.entries.push(Entry { id, path, track });
        self.order.push(id);
        Some(id)
    }

    /// Call after stopping a source. The previous queue survives a rejected replacement.
    pub fn replace(&mut self, paths: Vec<PathBuf>) -> bool {
        self.replace_items(paths.into_iter().map(|path| (path, None)).collect())
    }

    pub fn replace_items(&mut self, items: Vec<Item>) -> bool {
        if items.len() > MAX_ENTRIES || self.last_id.checked_add(items.len() as u64).is_none() {
            return false;
        }
        self.current = None;
        self.entries.clear();
        self.order.clear();
        for item in items {
            self.append_item(item);
        }
        true
    }

    pub fn play_next(&mut self, path: PathBuf) -> Option<EntryId> {
        self.play_next_item((path, None))
    }

    pub fn play_next_item(&mut self, item: Item) -> Option<EntryId> {
        let id = self.append_item(item)?;
        let before = self
            .current
            .and_then(|id| self.order.iter().position(|item| *item == id))
            .map_or(0, |index| index + 1);
        let target = self.order.get(before).copied();
        self.move_before(id, target);
        Some(id)
    }

    #[cfg(any(windows, test))]
    pub(super) fn valid(&self) -> bool {
        if self.entries.len() > MAX_ENTRIES || self.order.len() != self.entries.len() {
            return false;
        }
        let ids: HashSet<_> = self.entries.iter().map(|entry| entry.id).collect();
        let order: HashSet<_> = self.order.iter().copied().collect();
        ids.len() == self.entries.len()
            && order == ids
            && ids.iter().all(|id| id.0 != 0 && id.0 <= self.last_id)
            && self.current.is_none_or(|id| ids.contains(&id))
    }

    pub fn select(&mut self, id: EntryId) -> Option<()> {
        self.order.contains(&id).then(|| self.current = Some(id))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn order(&self) -> &[EntryId] {
        &self.order
    }

    pub fn current(&self) -> Option<EntryId> {
        self.current
    }

    pub fn shuffled(&self) -> bool {
        self.shuffled
    }

    /// A playing row must be stopped or replaced by its owner first.
    pub fn remove(&mut self, id: EntryId) -> bool {
        if self.current == Some(id) {
            return false;
        }
        self.order.retain(|candidate| *candidate != id);
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        before != self.entries.len()
    }

    /// Reorders upcoming tracks without rewriting playback history.
    pub fn move_before(&mut self, id: EntryId, before: Option<EntryId>) -> bool {
        let first = self
            .current
            .and_then(|current| self.order.iter().position(|item| *item == current))
            .map_or(0, |i| i + 1);
        let Some(from) = self
            .order
            .iter()
            .position(|item| *item == id)
            .filter(|index| *index >= first)
        else {
            return false;
        };
        let to = match before {
            Some(target) => match self
                .order
                .iter()
                .position(|item| *item == target)
                .filter(|index| *index >= first)
            {
                Some(index) => index,
                None => return false,
            },
            None => self.order.len(),
        };
        self.order.remove(from);
        self.order.insert(to - usize::from(to > from), id);
        if !self.shuffled {
            self.entries
                .sort_by_key(|entry| self.order.iter().position(|id| *id == entry.id));
        }
        true
    }

    pub fn clear_upcoming(&mut self) {
        let keep = self
            .current
            .and_then(|id| self.order.iter().position(|item| *item == id))
            .map_or(0, |i| i + 1);
        self.order.truncate(keep);
        self.entries.retain(|entry| self.order.contains(&entry.id));
    }

    pub fn neighbor(&self, previous: bool, natural_end: bool) -> Option<EntryId> {
        let Some(id) = self.current else {
            return (!previous).then(|| self.order.first().copied()).flatten();
        };
        let index = self.order.iter().position(|entry| *entry == id)?;
        if natural_end && self.repeat == Repeat::One {
            return Some(id);
        }
        let next = if previous {
            index.checked_sub(1).and_then(|i| self.order.get(i))
        } else {
            self.order.get(index + 1)
        };
        next.or_else(|| {
            if self.repeat == Repeat::All {
                if previous {
                    self.order.last()
                } else {
                    self.order.first()
                }
            } else {
                None
            }
        })
        .copied()
    }

    /// Begins shuffled playback of the whole queue: `first` (or a seeded pick)
    /// leads and everything else follows in shuffled order.
    pub fn start_shuffled(&mut self, first: Option<usize>, seed: u64) -> Option<EntryId> {
        if self.entries.is_empty() {
            return None;
        }
        let index = first
            .filter(|index| *index < self.entries.len())
            .unwrap_or((seed % self.entries.len() as u64) as usize);
        let lead = self.entries[index].id;
        self.order = self.entries.iter().map(|entry| entry.id).collect();
        self.order.remove(index);
        self.order.insert(0, lead);
        self.current = Some(lead);
        self.shuffled = false;
        self.shuffle(Some(seed));
        Some(lead)
    }

    /// Toggle shuffle. The visited prefix survives while shuffle is enabled.
    /// Repeated calls with the same mode leave the playback order unchanged.
    pub fn shuffle(&mut self, seed: Option<u64>) {
        if self.shuffled == seed.is_some() {
            return;
        }
        self.shuffled = seed.is_some();
        let Some(mut seed) = seed else {
            self.order = self.entries.iter().map(|entry| entry.id).collect();
            return;
        };
        let first = self
            .current
            .and_then(|id| self.order.iter().position(|item| *item == id))
            .map_or(0, |i| i + 1);
        for index in (first + 1..self.order.len()).rev() {
            // SplitMix64 is deterministic and adequate for non-cryptographic shuffling.
            seed = seed.wrapping_add(0x9e3779b97f4a7c15);
            let mut value = seed;
            value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
            value ^= value >> 31;
            self.order
                .swap(index, first + (value % (index - first + 1) as u64) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_queue_edits_preserve_unique_ids_and_reachable_order() {
        let mut queue = Queue::default();
        let mut seed = 19_u64;
        for _ in 0..500 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let ids = queue.order().to_vec();
            let id = ids.get((seed >> 32) as usize % ids.len().max(1)).copied();
            match (seed >> 12) % 7 {
                0 => {
                    queue.append("duplicate.flac".into());
                }
                1 => {
                    queue.play_next("duplicate.flac".into());
                }
                2 => {
                    if let Some(id) = id {
                        queue.remove(id);
                    }
                }
                3 => {
                    if let Some(id) = id {
                        queue.select(id);
                    }
                }
                4 => {
                    if let Some(id) = id {
                        queue.move_before(id, None);
                    }
                }
                5 => queue.shuffle(if queue.shuffled() { None } else { Some(seed) }),
                _ => queue.clear_upcoming(),
            }
            assert!(queue.valid());
        }
    }
}
