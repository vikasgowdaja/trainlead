//! Local vendor and trainer onboarding: profiles, review status, and supporting documents.
mod profiles;

use std::{collections::HashMap, path::{Path, PathBuf}};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use trainlead_core::{Error, Result, models::Cluster};

pub use profiles::{Step, TrainerProfile, VendorProfile, normalize_phone, validate_url};
#[cfg(test)]
use profiles::SkillEntry;

const MAX_DOCUMENT_BYTES: usize = 2 * 1024 * 1024;
const VENDOR_DOCUMENTS: &[&str] = &["Registration certificate", "GST certificate", "PAN card", "Other"];
const TRAINER_DOCUMENTS: &[&str] = &["Resume", "ID proof", "Certificate", "Other"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind { Vendor, Trainer }

impl Kind {
    fn as_str(self) -> &'static str { match self { Self::Vendor => "vendor", Self::Trainer => "trainer" } }
    fn table(self) -> &'static str { match self { Self::Vendor => "vendors", Self::Trainer => "trainers" } }
    fn document_kinds(self) -> &'static [&'static str] { match self { Self::Vendor => VENDOR_DOCUMENTS, Self::Trainer => TRAINER_DOCUMENTS } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status { Draft, Pending, Approved, Rejected }

impl Status {
    fn as_str(self) -> &'static str {
        match self { Self::Draft => "draft", Self::Pending => "pending", Self::Approved => "approved", Self::Rejected => "rejected" }
    }

    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "draft" => Self::Draft, "pending" => Self::Pending, "approved" => Self::Approved, "rejected" => Self::Rejected,
            _ => return Err(Error::Validation("unknown onboarding status".into())),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub id: i64,
    pub kind: String,
    pub file_name: String,
    pub media_type: String,
    pub size: i64,
    pub credential_url: Option<String>,
    pub expires_on: Option<NaiveDate>,
    pub uploaded_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewDocument {
    pub kind: String,
    pub file_name: String,
    pub data_base64: String,
    pub credential_url: Option<String>,
    pub expires_on: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Record<P> {
    pub id: i64,
    pub status: Status,
    pub review_note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub profile: P,
    pub documents: Vec<Document>,
    pub checklist: Vec<Step>,
    /// Requirements in the lead list posted from this vendor's numbers (vendors only).
    pub postings: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Suggestion {
    pub name: Option<String>,
    pub company: Option<String>,
    pub phone: String,
    pub postings: usize,
}

pub struct Directory {
    connection: Connection,
    files: PathBuf,
}

struct Stored<P> {
    id: i64,
    status: Status,
    review_note: Option<String>,
    created_at: i64,
    updated_at: i64,
    profile: P,
    documents: Vec<Document>,
}

fn invalid(message: impl Into<String>) -> Error { Error::Validation(message.into()) }

/// Recognizes uploads by their content, not their file name.
fn media_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"%PDF-") { Some(("application/pdf", "pdf")) }
    else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some(("image/png", "png")) }
    else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) { Some(("image/jpeg", "jpg")) }
    else { None }
}

impl Directory {
    pub fn open(database: &Path, files: &Path) -> Result<Self> {
        std::fs::create_dir_all(files)?;
        let connection = Connection::open(database)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS vendors (
                id INTEGER PRIMARY KEY, profile TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('draft','pending','approved','rejected')),
                review_note TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS trainers (
                id INTEGER PRIMARY KEY, profile TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('draft','pending','approved','rejected')),
                review_note TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS documents (
                id INTEGER PRIMARY KEY, owner_kind TEXT NOT NULL CHECK(owner_kind IN ('vendor','trainer')),
                owner_id INTEGER NOT NULL, kind TEXT NOT NULL, file_name TEXT NOT NULL, media_type TEXT NOT NULL,
                size INTEGER NOT NULL, stored_name TEXT NOT NULL UNIQUE, credential_url TEXT, expires_on TEXT,
                uploaded_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS documents_owner ON documents(owner_kind, owner_id);")?;
        Ok(Self { connection, files: files.to_path_buf() })
    }

    fn status(&self, kind: Kind, id: i64) -> Result<Status> {
        let status: Option<String> = self.connection
            .query_row(&format!("SELECT status FROM {} WHERE id=?1", kind.table()), [id], |row| row.get(0)).optional()?;
        Status::parse(&status.ok_or_else(|| invalid(format!("{} not found", kind.as_str())))?)
    }

    fn save(&mut self, kind: Kind, id: Option<i64>, profile: &impl Serialize, now: i64) -> Result<i64> {
        let json = serde_json::to_string(profile)?;
        match id {
            None => {
                self.connection.execute(&format!("INSERT INTO {}(profile,status,created_at,updated_at) VALUES(?1,'draft',?2,?2)", kind.table()),
                    params![json, now])?;
                Ok(self.connection.last_insert_rowid())
            }
            Some(id) => {
                // Editing a rejected profile reopens it as a draft so it can be resubmitted.
                let next = match self.status(kind, id)? { Status::Rejected => Status::Draft, status => status };
                self.connection.execute(&format!("UPDATE {} SET profile=?1,status=?2,updated_at=?3 WHERE id=?4", kind.table()),
                    params![json, next.as_str(), now, id])?;
                Ok(id)
            }
        }
    }

    pub fn save_vendor(&mut self, id: Option<i64>, mut profile: VendorProfile, now: i64) -> Result<i64> {
        profile.normalize()?;
        self.save(Kind::Vendor, id, &profile, now)
    }

    pub fn save_trainer(&mut self, id: Option<i64>, mut profile: TrainerProfile, now: i64) -> Result<i64> {
        profile.normalize()?;
        if let Some(id) = id {
            // Skill verification is an admin decision; editing the profile cannot grant it.
            let previous = self.trainer(id)?.profile;
            for skill in &mut profile.skills {
                skill.verified = previous.skills.iter().any(|old| old.name == skill.name && old.verified);
            }
        } else {
            profile.skills.iter_mut().for_each(|skill| skill.verified = false);
        }
        self.save(Kind::Trainer, id, &profile, now)
    }

    fn records<P: DeserializeOwned>(&self, kind: Kind, id: Option<i64>) -> Result<Vec<Stored<P>>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT id,status,review_note,created_at,updated_at,profile FROM {} WHERE ?1 IS NULL OR id=?1 ORDER BY updated_at DESC, id DESC",
            kind.table()))?;
        let rows = statement.query_map([id], |row| Ok((
            row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?, row.get::<_, i64>(4)?, row.get::<_, String>(5)?,
        )))?;
        rows.map(|row| {
            let (id, status, review_note, created_at, updated_at, profile) = row?;
            Ok(Stored { id, status: Status::parse(&status)?, review_note, created_at, updated_at,
                profile: serde_json::from_str(&profile)?, documents: self.documents(kind, id)? })
        }).collect()
    }

    pub fn vendors(&self, leads: &[Cluster]) -> Result<Vec<Record<VendorProfile>>> {
        Ok(self.records::<VendorProfile>(Kind::Vendor, None)?.into_iter().map(|Stored { id, status, review_note, created_at, updated_at, profile, documents }| {
            let phones: Vec<&str> = profile.phones().collect();
            let postings = leads.iter().flat_map(|cluster| &cluster.sources)
                .filter(|source| source.requirement.vendor_phone.as_deref().is_some_and(|phone| phones.contains(&phone))).count();
            Record { checklist: profile.checklist(documents.len(), status), id, status, review_note, created_at, updated_at, profile, documents, postings }
        }).collect())
    }

    pub fn trainers(&self) -> Result<Vec<Record<TrainerProfile>>> {
        Ok(self.records::<TrainerProfile>(Kind::Trainer, None)?.into_iter().map(|Stored { id, status, review_note, created_at, updated_at, profile, documents }| {
            let kinds: Vec<String> = documents.iter().map(|document| document.kind.clone()).collect();
            Record { checklist: profile.checklist(&kinds, status), id, status, review_note, created_at, updated_at, profile, documents, postings: 0 }
        }).collect())
    }

    fn trainer(&self, id: i64) -> Result<Record<TrainerProfile>> {
        self.trainers()?.into_iter().find(|record| record.id == id).ok_or_else(|| invalid("trainer not found"))
    }

    fn checklist(&self, kind: Kind, id: i64) -> Result<Vec<Step>> {
        match kind {
            Kind::Vendor => self.vendors(&[])?.into_iter().find(|record| record.id == id).map(|record| record.checklist),
            Kind::Trainer => Some(self.trainer(id)?.checklist),
        }.ok_or_else(|| invalid("vendor not found"))
    }

    pub fn submit(&mut self, kind: Kind, id: i64, now: i64) -> Result<()> {
        if !matches!(self.status(kind, id)?, Status::Draft | Status::Rejected) {
            return Err(invalid("only drafts or rejected profiles can be submitted"));
        }
        let missing: Vec<&str> = self.checklist(kind, id)?.into_iter().filter(|step| step.required && !step.done).map(|step| step.label).collect();
        if !missing.is_empty() { return Err(invalid(format!("Complete these steps first: {}", missing.join(", ")))); }
        self.set_status(kind, id, Status::Pending, None, now)
    }

    pub fn decide(&mut self, kind: Kind, id: i64, approve: bool, reason: Option<String>, now: i64) -> Result<()> {
        if self.status(kind, id)? != Status::Pending { return Err(invalid("only profiles awaiting review can be approved or rejected")); }
        if approve { return self.set_status(kind, id, Status::Approved, None, now); }
        let reason = reason.map(|reason| reason.trim().to_string()).filter(|reason| !reason.is_empty())
            .ok_or_else(|| invalid("Give a reason for the rejection"))?;
        if reason.chars().count() > 500 { return Err(invalid("Reason must be at most 500 characters")); }
        self.set_status(kind, id, Status::Rejected, Some(reason), now)
    }

    fn set_status(&mut self, kind: Kind, id: i64, status: Status, note: Option<String>, now: i64) -> Result<()> {
        self.connection.execute(&format!("UPDATE {} SET status=?1,review_note=?2,updated_at=?3 WHERE id=?4", kind.table()),
            params![status.as_str(), note, now, id])?;
        Ok(())
    }

    pub fn set_skill_verified(&mut self, trainer_id: i64, skill: &str, verified: bool, now: i64) -> Result<()> {
        let mut profile = self.trainer(trainer_id)?.profile;
        let entry = profile.skills.iter_mut().find(|entry| entry.name == skill).ok_or_else(|| invalid("skill not found"))?;
        entry.verified = verified;
        self.connection.execute("UPDATE trainers SET profile=?1,updated_at=?2 WHERE id=?3",
            params![serde_json::to_string(&profile)?, now, trainer_id])?;
        Ok(())
    }

    pub fn delete(&mut self, kind: Kind, id: i64) -> Result<()> {
        self.status(kind, id)?;
        for document in self.documents(kind, id)? { self.delete_document(document.id)?; }
        self.connection.execute(&format!("DELETE FROM {} WHERE id=?1", kind.table()), [id])?;
        Ok(())
    }

    fn documents(&self, kind: Kind, owner: i64) -> Result<Vec<Document>> {
        let mut statement = self.connection.prepare(
            "SELECT id,kind,file_name,media_type,size,credential_url,expires_on,uploaded_at FROM documents
             WHERE owner_kind=?1 AND owner_id=?2 ORDER BY id")?;
        let rows = statement.query_map(params![kind.as_str(), owner], |row| Ok(Document {
            id: row.get(0)?, kind: row.get(1)?, file_name: row.get(2)?, media_type: row.get(3)?, size: row.get(4)?,
            credential_url: row.get(5)?,
            expires_on: row.get::<_, Option<String>>(6)?.and_then(|date| date.parse().ok()),
            uploaded_at: row.get(7)?,
        }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_document(&mut self, kind: Kind, owner: i64, mut document: NewDocument, now: i64) -> Result<Document> {
        self.status(kind, owner)?;
        if !kind.document_kinds().contains(&document.kind.as_str()) { return Err(invalid("Choose a document type from the list")); }
        let file_name: String = document.file_name.rsplit(['/', '\\']).next().unwrap_or_default().trim().chars().take(120).collect();
        if file_name.is_empty() { return Err(invalid("Document needs a file name")); }
        let bytes = STANDARD.decode(document.data_base64.trim()).map_err(|_| invalid("Document upload was corrupted"))?;
        if bytes.is_empty() || bytes.len() > MAX_DOCUMENT_BYTES { return Err(invalid("Documents must be at most 2 MB")); }
        let (media_type, extension) = media_type(&bytes).ok_or_else(|| invalid("Only PDF, PNG or JPEG documents are accepted"))?;
        validate_url(&mut document.credential_url, "Credential link")?;
        let stored_name = format!("{}-{owner}-{}.{extension}", kind.as_str(), uuid::Uuid::new_v4());
        std::fs::write(self.files.join(&stored_name), &bytes)?;
        let inserted = self.connection.execute(
            "INSERT INTO documents(owner_kind,owner_id,kind,file_name,media_type,size,stored_name,credential_url,expires_on,uploaded_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![kind.as_str(), owner, document.kind, file_name, media_type, bytes.len() as i64, stored_name,
                document.credential_url, document.expires_on.map(|date| date.to_string()), now]);
        if let Err(error) = inserted {
            let _ = std::fs::remove_file(self.files.join(&stored_name));
            return Err(error.into());
        }
        let id = self.connection.last_insert_rowid();
        self.documents(kind, owner)?.into_iter().find(|document| document.id == id).ok_or_else(|| invalid("document not saved"))
    }

    pub fn delete_document(&mut self, id: i64) -> Result<()> {
        let stored: Option<String> = self.connection
            .query_row("SELECT stored_name FROM documents WHERE id=?1", [id], |row| row.get(0)).optional()?;
        let stored = stored.ok_or_else(|| invalid("document not found"))?;
        self.connection.execute("DELETE FROM documents WHERE id=?1", [id])?;
        match std::fs::remove_file(self.files.join(stored)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// Vendors seen in WhatsApp requirements whose numbers are not yet in the directory.
    pub fn suggestions(&self, leads: &[Cluster]) -> Result<Vec<Suggestion>> {
        let known: Vec<String> = self.vendors(&[])?.iter().flat_map(|record| record.profile.phones().map(str::to_string).collect::<Vec<_>>()).collect();
        let mut found: HashMap<String, Suggestion> = HashMap::new();
        for source in leads.iter().flat_map(|cluster| &cluster.sources) {
            let requirement = &source.requirement;
            let Some(phone) = requirement.vendor_phone.as_deref().and_then(|phone| normalize_phone(phone).ok()) else { continue };
            if known.contains(&phone) { continue; }
            let entry = found.entry(phone.clone()).or_insert_with(|| Suggestion { name: None, company: None, phone, postings: 0 });
            entry.postings += 1;
            entry.name = entry.name.take().or_else(|| requirement.vendor_name.clone());
            entry.company = entry.company.take().or_else(|| requirement.vendor_company.clone());
        }
        let mut suggestions: Vec<Suggestion> = found.into_values().collect();
        suggestions.sort_by(|left, right| right.postings.cmp(&left.postings).then_with(|| left.phone.cmp(&right.phone)));
        Ok(suggestions)
    }
}

#[cfg(test)]
mod tests;
