//! Notifications for friend requests and connection requests, with answer buttons.
//! Windows: a toast whose buttons open a signed `secpar-alert:` link, which starts this program
//! in answer mode. Linux: `notify-send` with actions. macOS has no app yet, so it shows nothing.
use crate::account::Answer;
use ring::{
    hmac,
    rand::{SecureRandom, SystemRandom},
};

const KEY: &str = "alert-key";
pub const SCHEME: &str = "secpar-alert";

pub struct Alert {
    pub title: String,
    pub body: String,
    /// The account id of the friend, or the connection request id.
    pub id: String,
    pub yes: Answer,
    pub no: Answer,
}
impl Answer {
    pub fn code(self) -> &'static str {
        match self {
            Answer::AllowConnection => "allow",
            Answer::DenyConnection => "deny",
            Answer::AcceptFriend => "accept",
            Answer::DeclineFriend => "decline",
        }
    }
    pub fn parse(code: &str) -> Option<Self> {
        [Answer::AllowConnection, Answer::DenyConnection, Answer::AcceptFriend, Answer::DeclineFriend]
            .into_iter()
            .find(|a| a.code() == code)
    }
    pub fn label(self) -> &'static str {
        match self {
            Answer::AllowConnection => "Allow",
            Answer::DenyConnection => "Deny",
            Answer::AcceptFriend => "Accept",
            Answer::DeclineFriend => "Decline",
        }
    }
}
/// The secret that signs answer links. Only this user's credential store holds it, so a web page
/// cannot forge a link that approves a request.
pub fn key() -> Option<hmac::Key> {
    let bytes = match crate::vault::load(KEY).ok()? {
        Some(hex) if hex.len() == 64 => unhex(&hex)?,
        _ => {
            let mut fresh = [0u8; 32];
            SystemRandom::new().fill(&mut fresh).ok()?;
            crate::vault::save(KEY, &hex(&fresh)).ok()?;
            fresh.to_vec()
        }
    };
    Some(hmac::Key::new(hmac::HMAC_SHA256, &bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok()).collect()
}
fn mac(key: &hmac::Key, answer: Answer, id: &str) -> String {
    hex(&hmac::sign(key, format!("{}/{id}", answer.code()).as_bytes()).as_ref()[..16])
}
pub fn link(key: &hmac::Key, answer: Answer, id: &str) -> String {
    format!("{SCHEME}:{}/{id}/{}", answer.code(), mac(key, answer, id))
}
/// The answer in a link, if the signature is ours. The id is checked again before any request.
pub fn verify(key: &hmac::Key, uri: &str) -> Option<(Answer, String)> {
    let rest = uri.strip_prefix(SCHEME)?.strip_prefix(':')?.trim_end_matches('/');
    let mut parts = rest.split('/');
    let (code, id, tag) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let answer = Answer::parse(code)?;
    let expected = unhex(tag).filter(|t| t.len() == 16)?;
    ring::constant_time::verify_slices_are_equal(&unhex(&mac(key, answer, id))?, &expected).ok()?;
    Some((answer, id.to_string()))
}
pub fn xml_escape(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c if c.is_control() => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}
pub fn toast_xml(alert: &Alert, key: &hmac::Key) -> String {
    let button = |a: Answer| {
        format!(
            "<action content=\"{}\" arguments=\"{}\" activationType=\"protocol\"/>",
            a.label(),
            xml_escape(&link(key, a, &alert.id))
        )
    };
    format!(
        "<toast scenario=\"reminder\"><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual><actions>{}{}</actions></toast>",
        xml_escape(&alert.title),
        xml_escape(&alert.body),
        button(alert.yes),
        button(alert.no)
    )
}

#[cfg(windows)]
pub fn show(alert: Alert, _answers: tokio::sync::mpsc::Sender<crate::account::Command>) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::sync::OnceLock;
    static REGISTERED: OnceLock<()> = OnceLock::new();
    let Some(key) = key() else { return };
    let reg = |args: &[&str]| {
        let _ = Command::new("reg").args(["add"]).args(args).arg("/f").creation_flags(0x0800_0000).output();
    };
    REGISTERED.get_or_init(|| {
        let Ok(exe) = std::env::current_exe() else { return };
        let exe = exe.display().to_string();
        let root = format!("HKCU\\Software\\Classes\\{SCHEME}");
        reg(&[&root, "/ve", "/d", "URL:Secpar answer"]);
        reg(&[&root, "/v", "URL Protocol", "/d", ""]);
        reg(&[&format!("{root}\\shell\\open\\command"), "/ve", "/d", &format!("\"{exe}\" \"%1\"")]);
        reg(&["HKCU\\Software\\Classes\\AppUserModelId\\com.secpar.host", "/v", "DisplayName", "/d", "Secpar"]);
    });
    let script = TOAST_SCRIPT;
    let xml = toast_xml(&alert, &key);
    std::thread::spawn(move || {
        let _ = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
            .env("SECPAR_TOAST", xml)
            .creation_flags(0x0800_0000)
            .output();
    });
}

/// Arguments for `notify-send`. Actions need libnotify 0.8 or later.
#[cfg(any(target_os = "linux", test))]
pub fn notify_send_args(alert: &Alert, actions: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-a", "Secpar", "-t", "300000"].map(String::from).into();
    if actions {
        for a in [alert.yes, alert.no] {
            args.push("-A".into());
            args.push(format!("{}={}", a.code(), a.label()));
        }
        args.push("--wait".into());
    }
    args.push("--".into());
    args.push(alert.title.clone());
    args.push(alert.body.clone());
    args
}
#[cfg(target_os = "linux")]
pub fn show(alert: Alert, answers: tokio::sync::mpsc::Sender<crate::account::Command>) {
    use std::process::{Command, Stdio};
    std::thread::spawn(move || {
        let help = Command::new("notify-send").arg("--help").output();
        let Ok(help) = help else { return };
        let actions = String::from_utf8_lossy(&help.stdout).contains("--action");
        let out = Command::new("notify-send")
            .args(notify_send_args(&alert, actions))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        if let Ok(out) = out {
            let chosen = String::from_utf8_lossy(&out.stdout);
            if let Some(answer) = Answer::parse(chosen.trim()).filter(|a| *a == alert.yes || *a == alert.no) {
                let _ = answers.blocking_send(crate::account::Command::Answer(answer, alert.id));
            }
        }
    });
}
#[cfg(not(any(windows, target_os = "linux")))]
pub fn show(_alert: Alert, _answers: tokio::sync::mpsc::Sender<crate::account::Command>) {}

/// A plain notice with no buttons. Windows hosts toast from the tray, so only Linux needs this.
#[cfg(target_os = "linux")]
pub fn info(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    std::thread::spawn(move || {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "Secpar", "--", &title, &body])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}
#[cfg(windows)]
const TOAST_SCRIPT:&str = "$ErrorActionPreference='Stop';\
        [void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
        [void][Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom.XmlDocument,ContentType=WindowsRuntime];\
        $d=New-Object Windows.Data.Xml.Dom.XmlDocument;$d.LoadXml($env:SECPAR_TOAST);\
        $t=New-Object Windows.UI.Notifications.ToastNotification $d;\
        $t.ExpirationTime=[DateTimeOffset]::Now.AddMinutes(5);\
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('com.secpar.host').Show($t)";

#[cfg(any(windows,test))]
fn notice_xml(title:&str,body:&str)->String {
    format!("<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",xml_escape(title),xml_escape(body))
}
#[cfg(windows)]
pub fn update_notice(title:&str,body:&str) {
    use std::os::windows::process::CommandExt;
    let xml=notice_xml(title,body);
    std::thread::spawn(move|| {
        let _=std::process::Command::new("reg")
            .args(["add","HKCU\\Software\\Classes\\AppUserModelId\\com.secpar.host","/v","DisplayName","/d","Secpar","/f"])
            .creation_flags(0x0800_0000).output();
        let _=std::process::Command::new("powershell.exe")
            .args(["-NoProfile","-NonInteractive","-WindowStyle","Hidden","-Command",TOAST_SCRIPT])
            .env("SECPAR_TOAST",xml).creation_flags(0x0800_0000).output();
    });
}
#[cfg(not(target_os="linux"))]
pub fn info(_title:&str,_body:&str) {}
#[cfg(target_os="linux")]
pub fn update_notice(title:&str,body:&str) {info(title,body);}
#[cfg(not(any(windows,target_os="linux")))]
pub fn update_notice(_title:&str,_body:&str) {}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> hmac::Key {
        hmac::Key::new(hmac::HMAC_SHA256, &[7u8; 32])
    }
    const ID: &str = "11111111-2222-4333-8444-555555555555";
    fn alert() -> Alert {
        Alert {
            title: "Connection request".into(),
            body: "Bob <b> wants to use \"Studio\" & more".into(),
            id: ID.into(),
            yes: Answer::AllowConnection,
            no: Answer::DenyConnection,
        }
    }
    #[test]
    fn a_plain_notice_escapes_text_without_actions() {
        let xml=notice_xml("Secpar <update>","1 & 2");
        assert!(xml.contains("Secpar &lt;update&gt;"));
        assert!(xml.contains("1 &amp; 2"));assert!(!xml.contains("<actions"));
    }
    #[test]
    fn a_signed_link_round_trips() {
        let l = link(&key(), Answer::AllowConnection, ID);
        assert!(l.starts_with("secpar-alert:allow/"));
        assert_eq!(verify(&key(), &l), Some((Answer::AllowConnection, ID.to_string())));
        assert_eq!(verify(&key(), &format!("{l}/")), Some((Answer::AllowConnection, ID.to_string())));
    }
    #[test]
    fn a_forged_or_altered_link_is_refused() {
        let good = link(&key(), Answer::DenyConnection, ID);
        let other = hmac::Key::new(hmac::HMAC_SHA256, &[8u8; 32]);
        assert_eq!(verify(&other, &good), None);
        assert_eq!(verify(&key(), &good.replace("deny", "allow")), None);
        assert_eq!(verify(&key(), &good.replace("5555", "5556")), None);
        let (head, _) = good.rsplit_once('/').unwrap();
        for bad in [
            format!("{head}/"),
            format!("{head}/zz"),
            format!("{head}/{}", "0".repeat(32)),
            format!("{good}/extra"),
            "secpar-alert:".into(),
            "secpar-alert:allow".into(),
            "https://allow/x/y".into(),
            format!("other:{}", good.trim_start_matches("secpar-alert:")),
        ] {
            assert_eq!(verify(&key(), &bad), None, "{bad}");
        }
    }
    #[test]
    fn the_toast_escapes_text_and_carries_both_buttons() {
        let xml = toast_xml(&alert(), &key());
        assert!(xml.contains("Bob &lt;b&gt; wants to use &quot;Studio&quot; &amp; more"));
        assert!(!xml.contains("<b>"));
        assert_eq!(xml.matches("<action ").count(), 2);
        assert!(xml.contains("content=\"Allow\"") && xml.contains("content=\"Deny\""));
        assert!(xml.contains("activationType=\"protocol\""));
        assert!(xml.contains(&link(&key(), Answer::DenyConnection, ID)));
    }
    #[test]
    fn notify_send_gets_actions_only_when_supported_and_never_reads_text_as_options() {
        let with = notify_send_args(&alert(), true);
        assert!(with.windows(2).any(|w| w == ["-A", "allow=Allow"]));
        assert!(with.windows(2).any(|w| w == ["-A", "deny=Deny"]));
        assert!(with.contains(&"--wait".to_string()));
        let without = notify_send_args(&alert(), false);
        assert!(!without.contains(&"-A".to_string()) && !without.contains(&"--wait".to_string()));
        let dash = Alert { title: "--hint".into(), ..alert() };
        let args = notify_send_args(&dash, false);
        let split = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[split + 1], "--hint");
    }
    #[test]
    #[ignore = "Shows a real toast and writes the alert key to the credential store"]
    fn shows_a_real_toast() {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        show(alert(), tx);
        std::thread::sleep(std::time::Duration::from_secs(25));
    }
    #[test]
    fn answers_have_stable_codes() {
        for a in [Answer::AllowConnection, Answer::DenyConnection, Answer::AcceptFriend, Answer::DeclineFriend] {
            assert_eq!(Answer::parse(a.code()), Some(a));
        }
        assert_eq!(Answer::parse("approve"), None);
    }
}
