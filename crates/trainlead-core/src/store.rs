use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    dedup::{DedupThresholds, exact_hash, is_similar},
    models::{Cluster, DuplicateKind, Message, Requirement, Sighting},
    source::Screenshot,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: i64,
    pub message: Message,
    pub status: String,
    pub review_reason: Option<String>,
}

pub struct Store {
    connection: Connection,
    thresholds: DedupThresholds,
}

impl Store {
    pub fn open(path: impl AsRef<Path>, thresholds: DedupThresholds) -> Result<Self> {
        Self::initialize(Connection::open(path)?, thresholds)
    }

    pub fn in_memory(thresholds: DedupThresholds) -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?, thresholds)
    }

    fn initialize(connection: Connection, thresholds: DedupThresholds) -> Result<Self> {
        thresholds.validate()?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > 1 {
            return Err(Error::Validation(
                "database schema is newer than this core supports".into(),
            ));
        }
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY, data TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('pending','processed','skipped','needs_review')),
                review_reason TEXT
            );
            CREATE TABLE IF NOT EXISTS screenshots (
                message_id INTEGER PRIMARY KEY REFERENCES messages(id),
                media_type TEXT NOT NULL, bytes BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS clusters (id INTEGER PRIMARY KEY);
            CREATE TABLE IF NOT EXISTS sightings (
                id INTEGER PRIMARY KEY,
                message_id INTEGER NOT NULL REFERENCES messages(id),
                cluster_id INTEGER NOT NULL REFERENCES clusters(id),
                exact_hash TEXT NOT NULL, requirement TEXT NOT NULL, source TEXT NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN ('new','exact','similar'))
            );
            CREATE INDEX IF NOT EXISTS sightings_hash ON sightings(exact_hash);
            CREATE INDEX IF NOT EXISTS sightings_cluster ON sightings(cluster_id);
            PRAGMA user_version=1;")?;
        Ok(Self {
            connection,
            thresholds,
        })
    }

    pub fn set_thresholds(&mut self, thresholds: DedupThresholds) -> Result<()> {
        thresholds.validate()?;
        self.thresholds = thresholds;
        Ok(())
    }

    pub fn record_messages(&mut self, messages: &[Message]) -> Result<Vec<i64>> {
        let transaction = self.connection.transaction()?;
        let mut ids = Vec::with_capacity(messages.len());
        for message in messages {
            transaction.execute(
                "INSERT INTO messages(data,status) VALUES (?1,'pending')",
                [serde_json::to_string(message)?],
            )?;
            ids.push(transaction.last_insert_rowid());
        }
        transaction.commit()?;
        Ok(ids)
    }

    pub fn record_screenshot(&mut self, screenshot: &Screenshot) -> Result<i64> {
        screenshot.validate()?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO messages(data,status) VALUES (?1,'pending')",
            [serde_json::to_string(&screenshot.message)?],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO screenshots(message_id,media_type,bytes) VALUES (?1,?2,?3)",
            params![id, screenshot.media_type, screenshot.bytes],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn screenshot(&self, message_id: i64) -> Result<Option<Screenshot>> {
        let row = self.connection.query_row(
            "SELECT m.data,s.media_type,s.bytes FROM screenshots s JOIN messages m ON m.id=s.message_id WHERE m.id=?1",
            [message_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?)),
        ).optional()?;
        row.map(|(message, media_type, bytes)| {
            Ok(Screenshot {
                message: serde_json::from_str(&message)?,
                media_type,
                bytes,
            })
        })
        .transpose()
    }

    pub fn mark_messages(&mut self, ids: &[i64], status: &str, reason: Option<&str>) -> Result<()> {
        let transaction = self.connection.transaction()?;
        for id in ids {
            let changed = transaction.execute(
                "UPDATE messages SET status=?1,review_reason=?2 WHERE id=?3 AND status='pending'",
                params![status, reason, id],
            )?;
            if changed != 1 {
                return Err(Error::Validation(
                    "message not found or already processed".into(),
                ));
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn messages_by_status(&self, status: &str) -> Result<Vec<StoredMessage>> {
        let mut statement = self.connection.prepare(
            "SELECT id,data,status,review_reason FROM messages WHERE status=?1 ORDER BY id",
        )?;
        let rows = statement.query_map([status], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, data, status, review_reason) = row?;
            Ok(StoredMessage {
                id,
                message: serde_json::from_str(&data)?,
                status,
                review_reason,
            })
        })
        .collect()
    }

    /// Moves text messages from the review inbox back to pending so they can be extracted again.
    /// Screenshots and messages not awaiting review are left untouched.
    pub fn requeue_review(&mut self, ids: &[i64]) -> Result<Vec<(i64, Message)>> {
        let transaction = self.connection.transaction()?;
        let mut requeued = Vec::with_capacity(ids.len());
        for id in ids {
            let changed = transaction.execute(
                "UPDATE messages SET status='pending',review_reason=NULL WHERE id=?1 AND status='needs_review'
                 AND NOT EXISTS (SELECT 1 FROM screenshots WHERE message_id=?1)",
                [id],
            )?;
            if changed == 1 {
                let data: String = transaction.query_row("SELECT data FROM messages WHERE id=?1", [id], |row| row.get(0))?;
                requeued.push((*id, serde_json::from_str(&data)?));
            }
        }
        transaction.commit()?;
        Ok(requeued)
    }

    pub fn save_batch(
        &mut self,
        ids: &[i64],
        records: &[(i64, Message, Requirement)],
    ) -> Result<Vec<DuplicateKind>> {
        let transaction = self.connection.transaction()?;
        let mut kinds = Vec::with_capacity(records.len());
        for (message_id, message, requirement) in records {
            if !ids.contains(message_id) {
                return Err(Error::Validation("record outside batch".into()));
            }
            let mut requirement = requirement.clone();
            requirement.normalize_and_validate()?;
            kinds.push(insert_sighting(
                &transaction,
                *message_id,
                message,
                &requirement,
                &self.thresholds,
            )?);
        }
        for id in ids {
            let status = if records.iter().any(|(message_id, _, _)| message_id == id) {
                "processed"
            } else {
                "skipped"
            };
            let changed = transaction.execute(
                "UPDATE messages SET status=?1,review_reason=NULL WHERE id=?2 AND status='pending'",
                params![status, id],
            )?;
            if changed != 1 {
                return Err(Error::Validation(
                    "message not found or already processed".into(),
                ));
            }
        }
        transaction.commit()?;
        Ok(kinds)
    }

    pub fn clusters(&self) -> Result<Vec<Cluster>> {
        let sightings = all_sightings(&self.connection)?;
        let mut groups: BTreeMap<i64, Vec<Sighting>> = BTreeMap::new();
        for sighting in sightings {
            groups
                .entry(sighting.cluster_id)
                .or_default()
                .push(sighting);
        }
        let mut clusters = Vec::with_capacity(groups.len());
        for (id, sources) in groups {
            let primary = sources
                .iter()
                .max_by_key(|sighting| (sighting.requirement.completeness(), sighting.id))
                .ok_or_else(|| Error::Validation("empty cluster".into()))?
                .clone();
            clusters.push(Cluster {
                id,
                primary,
                seen: sources.len(),
                groups: sources
                    .iter()
                    .map(|source| &source.message.source_group)
                    .collect::<BTreeSet<_>>()
                    .len(),
                exact: sources
                    .iter()
                    .filter(|source| source.kind == DuplicateKind::Exact)
                    .count(),
                similar: sources
                    .iter()
                    .filter(|source| source.kind == DuplicateKind::Similar)
                    .count(),
                sources,
            });
        }
        clusters.sort_by_key(|cluster| {
            std::cmp::Reverse(cluster.sources.iter().map(|source| source.id).max())
        });
        Ok(clusters)
    }

    pub fn merge_clusters(&mut self, target: i64, other: i64) -> Result<()> {
        if target == other {
            return Err(Error::Validation("select two distinct clusters".into()));
        }
        let transaction = self.connection.transaction()?;
        for id in [target, other] {
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM clusters WHERE id=?1",
                [id],
                |row| row.get(0),
            )?;
            if count != 1 {
                return Err(Error::Validation("cluster not found".into()));
            }
        }
        transaction.execute(
            "UPDATE sightings SET cluster_id=?1 WHERE cluster_id=?2",
            params![target, other],
        )?;
        transaction.execute("DELETE FROM clusters WHERE id=?1", [other])?;
        reclassify_cluster(&transaction, target)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn split_cluster(&mut self, cluster: i64, sighting_ids: &[i64]) -> Result<i64> {
        let selected: BTreeSet<i64> = sighting_ids.iter().copied().collect();
        let transaction = self.connection.transaction()?;
        let sources = all_sightings(&transaction)?;
        let available: BTreeSet<i64> = sources
            .iter()
            .filter(|source| source.cluster_id == cluster)
            .map(|source| source.id)
            .collect();
        if selected.is_empty()
            || selected.len() >= available.len()
            || !selected.is_subset(&available)
        {
            return Err(Error::Validation(
                "split must select a nonempty proper subset of a cluster".into(),
            ));
        }
        transaction.execute("INSERT INTO clusters DEFAULT VALUES", [])?;
        let new_cluster = transaction.last_insert_rowid();
        for id in selected {
            transaction.execute(
                "UPDATE sightings SET cluster_id=?1 WHERE id=?2",
                params![new_cluster, id],
            )?;
        }
        reclassify_cluster(&transaction, cluster)?;
        reclassify_cluster(&transaction, new_cluster)?;
        transaction.commit()?;
        Ok(new_cluster)
    }
}

fn all_sightings(connection: &Connection) -> Result<Vec<Sighting>> {
    let mut statement = connection
        .prepare("SELECT id,cluster_id,kind,source,requirement FROM sightings ORDER BY id")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    rows.map(|row| {
        let (id, cluster_id, kind, source, requirement) = row?;
        let kind = match kind.as_str() {
            "new" => DuplicateKind::New,
            "exact" => DuplicateKind::Exact,
            "similar" => DuplicateKind::Similar,
            _ => return Err(Error::Validation("invalid stored duplicate kind".into())),
        };
        Ok(Sighting {
            id,
            cluster_id,
            kind,
            message: serde_json::from_str(&source)?,
            requirement: serde_json::from_str(&requirement)?,
        })
    })
    .collect()
}

fn kind_name(kind: DuplicateKind) -> &'static str {
    match kind {
        DuplicateKind::New => "new",
        DuplicateKind::Exact => "exact",
        DuplicateKind::Similar => "similar",
    }
}

fn insert_sighting(
    transaction: &Transaction<'_>,
    message_id: i64,
    message: &Message,
    requirement: &Requirement,
    thresholds: &DedupThresholds,
) -> Result<DuplicateKind> {
    let hash = exact_hash(message, requirement)?;
    let mut exact = transaction
        .prepare("SELECT cluster_id FROM sightings WHERE exact_hash=?1 ORDER BY id LIMIT 1")?;
    let mut rows = exact.query([&hash])?;
    let exact_cluster = if let Some(row) = rows.next()? {
        Some(row.get::<_, i64>(0)?)
    } else {
        None
    };
    let matched = if let Some(cluster) = exact_cluster {
        Some((cluster, DuplicateKind::Exact))
    } else {
        all_sightings(transaction)?
            .into_iter()
            .find(|source| {
                is_similar(
                    &source.requirement,
                    requirement,
                    &source.message.raw_text,
                    &message.raw_text,
                    thresholds,
                )
            })
            .map(|source| (source.cluster_id, DuplicateKind::Similar))
    };
    let (cluster_id, kind) = match matched {
        Some(matched) => matched,
        None => {
            transaction.execute("INSERT INTO clusters DEFAULT VALUES", [])?;
            (transaction.last_insert_rowid(), DuplicateKind::New)
        }
    };
    transaction.execute("INSERT INTO sightings(message_id,cluster_id,exact_hash,requirement,source,kind) VALUES (?1,?2,?3,?4,?5,?6)",
        params![message_id, cluster_id, hash, serde_json::to_string(requirement)?, serde_json::to_string(message)?, kind_name(kind)])?;
    Ok(kind)
}

fn reclassify_cluster(transaction: &Transaction<'_>, cluster: i64) -> Result<()> {
    let sources: Vec<Sighting> = all_sightings(transaction)?
        .into_iter()
        .filter(|source| source.cluster_id == cluster)
        .collect();
    let mut hashes = BTreeSet::new();
    for (index, source) in sources.iter().enumerate() {
        let hash = exact_hash(&source.message, &source.requirement)?;
        let kind = if index == 0 {
            DuplicateKind::New
        } else if hashes.contains(&hash) {
            DuplicateKind::Exact
        } else {
            DuplicateKind::Similar
        };
        hashes.insert(hash);
        transaction.execute(
            "UPDATE sightings SET kind=?1 WHERE id=?2",
            params![kind_name(kind), source.id],
        )?;
    }
    Ok(())
}
