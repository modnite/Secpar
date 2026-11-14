//! Adds a throwaway hosting device to a test account, so pages have something
//! to list. Only works against a test account service on this computer.
//! Usage: enrol_test_device <accounts file> <account number> <device name>
use secpar_core::account::{Platform, Service};
use secpar_tunnel::Identity;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let file: serde_json::Value = serde_json::from_slice(&std::fs::read(args.next().ok_or("accounts file")?)?)?;
    let account = &file["accounts"][args.next().ok_or("account number")?.parse::<usize>()?];
    let name = args.next().ok_or("device name")?;
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_string();
    let service = Service::local_test(&text(&file["origin"]), &text(&file["address"]))?;
    let session = service.sign_in(&text(&account["email"]), &text(&account["password"])).await?;
    let device = service.enrol(&session, &Identity::generate()?, &name, Platform::Linux, true).await?;
    println!("enrolled {}", device.name);
    Ok(())
}
