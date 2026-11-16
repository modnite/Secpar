//! The real engine: the supervised hosting engine with its pairing journal.
use crate::{Engine, Pairing, Pending, Refusal};
use secpar_gamestream_host::{ClientIdentity, Host};

impl Engine for Host {
    fn approve<'a>(&'a self, owner: &'a str, device: &'a str, pairing: &'a Pairing, name: &'a str) -> Pending<'a, Result<(), Refusal>> {
        Box::pin(async move {
            let identity = ClientIdentity { client_id: pairing.client_id.clone(), certificate_sha256: pairing.certificate_sha256.clone() };
            self.approve_for(owner, device, &identity, &pairing.pin, name).await.map_err(|error| match error.0 {
                "The expected device has no unique pending pairing" => Refusal::NotPending,
                "This client certificate was revoked" => Refusal::Revoked,
                _ => Refusal::Failed,
            })
        })
    }
    fn revoke<'a>(&'a self, owner: &'a str, device: &'a str) -> Pending<'a, bool> {
        Box::pin(async move { Host::revoke(self, owner, device).await.is_ok() })
    }
    fn granted(&self) -> Pending<'_, Vec<(String, String)>> {
        Box::pin(async move {
            let mut pairs: Vec<(String, String)> = self.grants().await.into_iter().filter(|g| !g.revoked).map(|g| (g.owner, g.device)).collect();
            pairs.sort();
            pairs.dedup();
            pairs
        })
    }
    fn resolution(&self, width: u32, height: u32) -> Pending<'_, Result<bool, Refusal>> {
        Box::pin(async move { Host::resolution(self, width, height).await.map_err(|_| Refusal::Failed) })
    }
}
