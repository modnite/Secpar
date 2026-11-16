//! Shared Secpar logic for the desktop app, the host service and Android.
//! So far: the permanent device identity, the account service, and who may
//! connect to a hosting device.
pub mod account;
pub mod hosting;
pub mod identity;
pub mod store;
