//! Local service calls contain a purpose-bound proof or machine credential only.
use serde::Deserialize;
#[cfg(any(windows, test))]
use serde_json::json;
use serde_json::Value;
#[cfg(any(windows, test))]
const MAX_RESPONSE: usize = 512 * 1024;
#[cfg(any(windows, test))]
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Clone, Default, Deserialize)]
pub struct State {
    pub computer: Option<String>,
    pub owner: Option<String>,
    pub online: bool,
    #[serde(default)]
    pub note: String,
}
pub async fn state() -> Result<State, String> {
    serde_json::from_value(call("host_account_state", Value::Null).await?)
        .map_err(|_| "Update Secpar to use account linking".into())
}
#[cfg(windows)]
pub async fn call(command: &str, args: Value) -> Result<Value, String> {
    use tokio::net::windows::named_pipe::ClientOptions;
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let start = std::time::Instant::now();
        let pipe = loop {
            match ClientOptions::new().open(r"\\.\pipe\secpar") {
                Ok(pipe) => break pipe,
                Err(error)
                    if error.raw_os_error() == Some(231) && start.elapsed().as_secs() < 2 =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(_) => {
                    return Err(
                        "The Secpar hosting service is not available on this computer".into(),
                    )
                }
            }
        };
        exchange(pipe, command, args).await
    })
    .await
    .map_err(|_| "The Secpar hosting service did not respond in time".to_string())?
}
#[cfg(target_os = "linux")]
pub async fn call(command: &str, args: Value) -> Result<Value, String> {
    secpar_host_linux::backend::call(command, args).await
}
#[cfg(target_os = "macos")]
pub async fn call(command: &str, args: Value) -> Result<Value, String> {
    use secpar_host_macos::{backend::Backend, config_directory, native::MacPlatform};
    use std::sync::Arc;
    static HOST: tokio::sync::OnceCell<Arc<Backend>> = tokio::sync::OnceCell::const_new();
    // Capture, audio and input have not run on a physical Mac yet. Sign-in enrolls the host on
    // every platform, so hosting stays off unless a tester sets SECPAR_MACOS_HOST=1.
    if std::env::var_os("SECPAR_MACOS_HOST").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Err("Hosting on this platform is still in development".into());
    }
    let backend = HOST
        .get_or_try_init(|| async {
            let directory = config_directory(std::env::var_os("HOME"))?;
            let bind = "0.0.0.0:48999".parse().map_err(|_| "Invalid host address".to_string())?;
            Backend::start(&directory, Arc::new(MacPlatform), bind).await
        })
        .await?;
    backend.command(command, args).await
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub async fn call(_command: &str, _args: Value) -> Result<Value, String> {
    Err("Hosting on this platform is still in development".into())
}
#[cfg(any(windows, test))]
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    let request = format!("{}\n", json!({"id":1,"cmd":command,"args":args}));
    if request.len() > 16384 {
        return Err("Invalid host request".into());
    }
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| "Could not contact Secpar Host")?;
    let mut reader = BufReader::new(stream);
    // Both per-line and total limits apply, including pushed events before the reply.
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let n = (&mut reader)
            .take((MAX_RESPONSE + 1) as u64)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| "Incomplete host response")?;
        total += n;
        if n == 0 || n > MAX_RESPONSE || total > MAX_RESPONSE * 2 || line.last() != Some(&b'\n') {
            return Err("Invalid host response".into());
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| "Invalid host response")?;
        if value["id"] != 1 {
            continue;
        }
        if value.get("err").is_some() {
            // Never surface arbitrary local-server strings containing credentials.
            return Err("The Secpar hosting service could not complete this request. Check that it is updated and linked to your account.".into());
        }
        return value
            .get("ok")
            .cloned()
            .ok_or_else(|| "Invalid host response".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn accepts_the_hosts_large_qr_svg_without_truncating_the_link() {
        let (client, mut server) = tokio::io::duplex(65536);
        tokio::spawn(async move {
            let mut request = [0; 512];
            let n = server.read(&mut request).await.unwrap();
            let request: Value = serde_json::from_slice(&request[..n]).unwrap();
            assert_eq!(request["cmd"], "new_qr");
            let reply = format!(
                "{}\n",
                json!({"id":1,"ok":{"link":"secpar://pair?fixture=not-a-credential","svg":"x".repeat(100_000)}})
            );
            server.write_all(reply.as_bytes()).await.unwrap();
        });
        let response = exchange(client, "new_qr", Value::Null).await.unwrap();
        assert_eq!(response["link"], "secpar://pair?fixture=not-a-credential");
        assert_eq!(response["svg"].as_str().unwrap().len(), 100_000);
    }
    #[tokio::test]
    async fn waits_for_the_matching_reply_and_bounds_unterminated_input() {
        let (client, mut server) = tokio::io::duplex(65536);
        tokio::spawn(async move {
            let mut request = [0; 512];
            let n = server.read(&mut request).await.unwrap();
            let value: Value = serde_json::from_slice(&request[..n]).unwrap();
            assert_eq!(value["cmd"], "host_account_state");
            assert!(value["args"].is_null());
            server
                .write_all(b"{\"ev\":\"devices\"}\n{\"id\":1,\"ok\":{\"online\":false}}\n")
                .await
                .unwrap();
        });
        assert_eq!(
            exchange(client, "host_account_state", Value::Null)
                .await
                .unwrap()["online"],
            false
        );
        let (client, mut server) = tokio::io::duplex(65536);
        tokio::spawn(async move {
            let mut request = [0; 512];
            server.read(&mut request).await.unwrap();
            // A QR SVG can exceed 16KB; still bound malformed/unterminated data.
            let _ = server.write_all(&vec![b'a'; MAX_RESPONSE + 1]).await;
        });
        assert!(exchange(client, "host_account_state", Value::Null)
            .await
            .is_err());
    }
    #[tokio::test]
    async fn never_displays_raw_service_errors() {
        let (client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let mut request = [0; 512];
            server.read(&mut request).await.unwrap();
            server
                .write_all(b"{\"id\":1,\"err\":\"sensitive-value\"}\n")
                .await
                .unwrap();
        });
        let error = exchange(client, "host_account_state", Value::Null)
            .await
            .unwrap_err();
        assert!(!error.contains("sensitive-value"));
    }
}
