//! Local service calls contain a purpose-bound proof or machine credential only.
use serde::Deserialize;
#[cfg(any(windows, test))]
use serde_json::json;
use serde_json::Value;
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
        .map_err(|_| "Update Secpar Host to use account linking".into())
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
                Err(_) => return Err("Secpar Host is not available on this computer".into()),
            }
        };
        exchange(pipe, command, args).await
    })
    .await
    .map_err(|_| "Secpar Host did not respond in time".to_string())?
}
#[cfg(not(windows))]
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
            .take(16385)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| "Incomplete host response")?;
        total += n;
        if n == 0 || n > 16384 || total > 65536 || line.last() != Some(&b'\n') {
            return Err("Invalid host response".into());
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| "Invalid host response")?;
        if value["id"] != 1 {
            continue;
        }
        if value.get("err").is_some() {
            // Never surface arbitrary local-server strings containing credentials.
            return Err("Secpar Host could not complete this request. Check that it is updated and linked to your account.".into());
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
            server.write_all(&vec![b'a'; 20000]).await.unwrap();
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
