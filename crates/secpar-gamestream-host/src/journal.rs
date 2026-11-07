//! Durable enrollment journal. A revocation is written here BEFORE the host is
//! asked to forget the client, so a crash in between is repaired at next start.
use crate::{replace_private, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const LIMIT: usize = 512;
const FILE_LIMIT: usize = 512 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    /// Secpar account that owns the computer.
    pub owner: String,
    /// Authenticated Secpar device the grant was made for.
    pub device: String,
    pub client_id: String,
    pub certificate_sha256: String,
    pub revoked: bool,
}

pub struct Journal {
    path: PathBuf,
    records: Vec<Record>,
}

fn field_ok(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && !v.chars().any(char::is_control)
}

impl Journal {
    pub fn open(state: &Path) -> Result<Self> {
        let path = state.join("enrollment-journal.json");
        crate::private::ordinary_or_missing(&path)
            .map_err(|_| Error("Enrollment journal is a redirected file"))?;
        let records = if path.exists() {
            let bytes = crate::private::read(&path, FILE_LIMIT)
                .map_err(|_| Error("Could not read enrollment journal"))?;
            let records: Vec<Record> = serde_json::from_slice(&bytes)
                .map_err(|_| Error("Enrollment journal is corrupt"))?;
            if records.len() > LIMIT {
                return Err(Error("Enrollment journal is too large"));
            }
            records
        } else {
            Vec::new()
        };
        Ok(Self { path, records })
    }
    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.records)
            .map_err(|_| Error("Could not encode enrollment journal"))?;
        replace_private(&self.path, &bytes).map_err(|_| Error("Could not save enrollment journal"))
    }
    pub fn records(&self) -> &[Record] {
        &self.records
    }
    pub fn revoked(&self, fingerprint: &str) -> bool {
        self.records
            .iter()
            .any(|r| r.revoked && r.certificate_sha256 == fingerprint)
    }
    /// The same certificate may never be granted to a second owner/device, and a
    /// revoked certificate stays revoked.
    pub fn grant(&mut self, owner: &str, device: &str, client_id: &str, fp: &str) -> Result<()> {
        if ![owner, device, client_id].iter().all(|v| field_ok(v)) || fp.len() != 64 {
            return Err(Error("Enrollment record is invalid"));
        }
        if let Some(existing) = self.records.iter().find(|r| {
            r.certificate_sha256 == fp
                || (r.client_id == client_id && r.owner == owner && r.device == device)
        }) {
            if existing.revoked {
                return Err(Error("This client certificate was revoked"));
            }
            if existing.owner != owner
                || existing.device != device
                || existing.certificate_sha256 != fp
                || existing.client_id != client_id
            {
                return Err(Error("This client is already enrolled for another device"));
            }
            return Ok(());
        }
        if self.records.len() >= LIMIT {
            return Err(Error("Enrollment journal is full"));
        }
        self.records.push(Record {
            owner: owner.into(),
            device: device.into(),
            client_id: client_id.into(),
            certificate_sha256: fp.into(),
            revoked: false,
        });
        self.save()
    }
    /// Marks every grant of this owner's device as revoked and returns their
    /// fingerprints. Durable before returning.
    pub fn revoke(&mut self, owner: &str, device: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for r in &mut self.records {
            if r.owner == owner && r.device == device {
                r.revoked = true;
                out.push(r.certificate_sha256.clone());
            }
        }
        if !out.is_empty() {
            self.save()?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn grants_are_scoped_durable_and_not_transferable() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::open(dir.path()).unwrap();
        j.grant("acct1", "phone", "cid1", A).unwrap();
        j.grant("acct1", "phone", "cid1", A).unwrap();
        assert!(j.grant("acct2", "tablet", "cid1", A).is_err());
        assert!(j.grant("acct1", "tablet", "cid1", A).is_err());
        assert!(j.grant("acct1", "phone", "cid1", "short").is_err());
        j.grant("acct1", "tablet", "cid2", B).unwrap();
        let reopened = Journal::open(dir.path()).unwrap();
        assert_eq!(reopened.records().len(), 2);
    }

    #[test]
    fn revocation_is_durable_per_device_and_final() {
        let dir = tempfile::tempdir().unwrap();
        let mut j = Journal::open(dir.path()).unwrap();
        j.grant("acct1", "phone", "cid1", A).unwrap();
        j.grant("acct1", "tablet", "cid2", B).unwrap();
        assert_eq!(j.revoke("acct1", "phone").unwrap(), vec![A.to_string()]);
        assert!(j.revoked(A) && !j.revoked(B));
        assert!(j.grant("acct1", "phone", "cid1", A).is_err());
        assert!(Journal::open(dir.path()).unwrap().revoked(A));
        assert!(j.revoke("acct1", "nobody").unwrap().is_empty());
    }

    #[test]
    fn corrupt_journal_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("enrollment-journal.json"), b"not json").unwrap();
        assert!(Journal::open(dir.path()).is_err());
    }
}
