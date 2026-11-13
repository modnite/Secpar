//! Test tool for the tunnel. Not part of the product.
//!   tunnel identity <file>                      create if missing, print fingerprint
//!   tunnel serve <identity> <bind addr:port> <engine ip> <base port> <allowed fingerprint>...
//!   tunnel connect <identity> <host addr:port> <host fingerprint> <listen ip> <base port>
use secpar_tunnel::{connect, serve, Allowed, Fingerprint, Identity, Ports};
use std::path::Path;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn pin(text: &str) -> Result<Fingerprint> {
    if text.len() != 64 {
        return Err("A fingerprint is 64 hex characters".into());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}
fn load(path: &Path) -> Result<Identity> {
    if !path.exists() {
        let identity = Identity::generate()?;
        let mut file = (identity.certificate_der().len() as u32).to_be_bytes().to_vec();
        file.extend_from_slice(identity.certificate_der());
        file.extend_from_slice(identity.key_der());
        std::fs::write(path, file)?;
    }
    let file = std::fs::read(path)?;
    let n = u32::from_be_bytes(file.get(..4).ok_or("Identity file is damaged")?.try_into()?) as usize;
    let certificate = file.get(4..4 + n).ok_or("Identity file is damaged")?.to_vec();
    Ok(Identity::from_der(certificate, file[4 + n..].to_vec()))
}

#[tokio::main]
async fn main() -> Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str).ok_or("Missing argument");
    let identity = load(Path::new(arg(1)?))?;
    match arg(0)? {
        "identity" => println!("{}", hex(&identity.fingerprint())),
        "serve" => {
            let allowed = args[5..].iter().map(|f| pin(f)).collect::<Result<Vec<_>>>()?;
            let _host = serve(
                arg(2)?.parse()?,
                &identity,
                Allowed::new(allowed),
                arg(3)?.parse()?,
                Ports::engine(arg(4)?.parse()?),
            )
            .await?;
            println!("READY");
            std::future::pending::<()>().await;
        }
        "connect" => {
            let client = connect(
                arg(2)?.parse()?,
                &identity,
                pin(arg(3)?)?,
                arg(4)?.parse()?,
                Ports::engine(arg(5)?.parse()?),
            )
            .await?;
            println!("READY");
            client.closed().await;
        }
        _ => return Err("Unknown command".into()),
    }
    Ok(())
}
