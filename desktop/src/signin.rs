//! Sign in, sign up and password recovery inside the app. The page keeps what the person typed and
//! says what to fix in plain words. The network calls live in `account.rs`.
use crate::design;
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    SignIn,
    SignUp,
    /// The six digit code from the email. `reset` is true when it sets a new password.
    Code { reset: bool },
    Forgot,
}
/// One call to the account service. Routes are `/v1/app/<name>`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Request {
    SignIn { email: String, password: String },
    Register { email: String, password: String, handle: String },
    Resend { email: String },
    Verify { email: String, code: String },
    Forgot { email: String },
    Reset { email: String, code: String, password: String },
}
impl Request {
    pub fn route(&self) -> &'static str {
        match self {
            Request::SignIn { .. } => "sign-in",
            Request::Register { .. } => "register",
            Request::Resend { .. } => "resend",
            Request::Verify { .. } => "verify",
            Request::Forgot { .. } => "forgot",
            Request::Reset { .. } => "reset",
        }
    }
    pub fn body(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Request::SignIn { email, password } => json!({"email": email, "password": password}),
            Request::Register { email, password, handle } => json!({"email": email, "password": password, "handle": handle}),
            Request::Resend { email } | Request::Forgot { email } => json!({"email": email}),
            Request::Verify { email, code } => json!({"email": email, "code": code}),
            Request::Reset { email, code, password } => json!({"email": email, "code": code, "password": password}),
        }
    }
}
pub struct Form {
    pub step: Step,
    email: String,
    password: String,
    handle: String,
    code: String,
    /// What to fix, in plain words.
    error: String,
    /// What just happened, such as a code that was sent.
    info: String,
}
impl Default for Form {
    fn default() -> Self {
        Self { step: Step::SignIn, email: String::new(), password: String::new(), handle: String::new(), code: String::new(), error: String::new(), info: String::new() }
    }
}
fn email_ok(email: &str) -> bool {
    let e = email.trim();
    e.len() <= 254 && e.split_once('@').is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.ends_with('.') && !e.contains(char::is_whitespace))
}
fn password_ok(password: &str) -> bool {
    (8..=256).contains(&password.chars().count()) && password.len() <= 256
}
fn handle_ok(handle: &str) -> bool {
    (3..=30).contains(&handle.len()) && handle.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
impl Form {
    /// Back to the first screen with nothing typed, after signing in or out.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn email(&self) -> &str {
        self.email.trim()
    }
    /// The reply to a request. Returns the next request when one follows without a click.
    pub fn done(&mut self, route: &str, result: Result<(), String>) -> Option<Request> {
        self.info.clear();
        self.error.clear();
        match (route, result) {
            ("sign-in", Err(note)) => {
                self.error = note;
                None
            }
            ("register" | "resend", Ok(())) => {
                self.step = Step::Code { reset: false };
                self.code.clear();
                self.info = format!("We sent a code to {}.", self.email.trim());
                None
            }
            ("forgot", Ok(())) => {
                self.step = Step::Code { reset: true };
                self.code.clear();
                self.password.clear();
                self.info = format!("We sent a code to {}.", self.email.trim());
                None
            }
            ("verify", Ok(())) => Some(Request::SignIn { email: self.email.trim().to_string(), password: self.password.clone() }),
            ("reset", Ok(())) => {
                self.step = Step::SignIn;
                self.code.clear();
                self.info = "Password changed. Sign in with the new one.".into();
                None
            }
            (_, Err(note)) => {
                self.error = note;
                None
            }
            _ => None,
        }
    }
    fn field(ui: &mut egui::Ui, value: &mut String, hint: &str, password: bool) -> egui::Response {
        let width = ui.available_width().min(360.);
        ui.add_sized([width, design::CONTROL], egui::TextEdit::singleline(value).hint_text(hint).password(password).margin(egui::vec2(10., 7.)))
    }
    fn link(ui: &mut egui::Ui, text: &str) -> bool {
        ui.add(egui::Label::new(RichText::new(text).color(design::ACCENT)).sense(egui::Sense::click())).clicked()
    }
    fn go(&mut self, step: Step) {
        self.step = step;
        self.error.clear();
        self.info.clear();
    }
    /// Draws the current step. Returns the request to send when a button was pressed and the input is fine.
    pub fn ui(&mut self, ui: &mut egui::Ui, busy: bool) -> Option<Request> {
        let mut send = None;
        ui.spacing_mut().item_spacing.y = 8.;
        let title = match self.step {
            Step::SignIn => "Sign in",
            Step::SignUp => "Create an account",
            Step::Code { reset: false } => "Enter the code",
            Step::Code { reset: true } => "Choose a new password",
            Step::Forgot => "Reset your password",
        };
        ui.label(RichText::new(title).strong());
        if !self.info.is_empty() {
            ui.label(RichText::new(&self.info).color(design::MUTED));
        }
        let mut enter = false;
        match self.step {
            Step::SignIn | Step::SignUp | Step::Forgot => {
                enter |= Self::field(ui, &mut self.email, "Email", false).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if self.step == Step::SignUp {
                    enter |= Self::field(ui, &mut self.handle, "Username", false).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
                if self.step != Step::Forgot {
                    enter |= Self::field(ui, &mut self.password, "Password", true).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
            }
            Step::Code { reset } => {
                enter |= Self::field(ui, &mut self.code, "Six digit code", false).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if reset {
                    enter |= Self::field(ui, &mut self.password, "New password", true).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
            }
        }
        if !self.error.is_empty() {
            ui.label(RichText::new(&self.error).color(design::BAD));
        }
        let label = match self.step {
            Step::SignIn => "Sign in",
            Step::SignUp => "Create account",
            Step::Code { reset: false } => "Confirm",
            Step::Code { reset: true } => "Save password",
            Step::Forgot => "Send code",
        };
        let pressed = ui.add_enabled(!busy, design::primary(label)).clicked() || (enter && !busy);
        if pressed {
            send = self.check();
        }
        ui.horizontal_wrapped(|ui| match self.step {
            Step::SignIn => {
                if Self::link(ui, "Create an account") {
                    self.go(Step::SignUp);
                }
                if Self::link(ui, "Forgot password") {
                    self.go(Step::Forgot);
                }
                if self.error.contains("not confirmed") && email_ok(&self.email) && Self::link(ui, "Send a new code") {
                    send = Some(Request::Resend { email: self.email.trim().to_string() });
                }
            }
            Step::SignUp | Step::Forgot => {
                if Self::link(ui, "I have an account") {
                    self.go(Step::SignIn);
                }
            }
            Step::Code { reset } => {
                if email_ok(&self.email) && Self::link(ui, "Send a new code") {
                    send = Some(if reset { Request::Forgot { email: self.email.trim().to_string() } } else { Request::Resend { email: self.email.trim().to_string() } });
                }
                if Self::link(ui, "Back") {
                    self.go(Step::SignIn);
                }
            }
        });
        send
    }
    /// Plain words for a problem the app can see before asking the service.
    fn check(&mut self) -> Option<Request> {
        self.error.clear();
        let email = self.email.trim().to_string();
        let code: String = self.code.chars().filter(|c| !c.is_whitespace()).collect();
        let bad = match self.step {
            Step::SignIn if !email_ok(&email) => Some("Enter your email address."),
            Step::SignIn if self.password.is_empty() => Some("Enter your password."),
            Step::SignUp if !email_ok(&email) => Some("Enter a valid email address."),
            Step::SignUp if !handle_ok(&self.handle) => Some("Use a username of 3 to 30 letters, numbers or underscores."),
            Step::SignUp if !password_ok(&self.password) => Some("Use a password of 8 to 256 characters."),
            Step::Forgot if !email_ok(&email) => Some("Enter a valid email address."),
            Step::Code { .. } if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) => Some("Enter the six digit code from the email."),
            Step::Code { reset: true } if !password_ok(&self.password) => Some("Use a password of 8 to 256 characters."),
            _ => None,
        };
        if let Some(note) = bad {
            self.error = note.into();
            return None;
        }
        Some(match self.step {
            Step::SignIn => Request::SignIn { email, password: self.password.clone() },
            Step::SignUp => Request::Register { email, password: self.password.clone(), handle: self.handle.trim().to_string() },
            Step::Forgot => Request::Forgot { email },
            Step::Code { reset: false } => Request::Verify { email, code },
            Step::Code { reset: true } => Request::Reset { email, code, password: self.password.clone() },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(step: Step) -> Form {
        Form { step, email: " a@b.co ".into(), password: "long enough".into(), handle: "Mia_1".into(), code: "123 456".into(), ..Form::default() }
    }
    #[test]
    fn checks_say_what_to_fix_and_send_nothing() {
        let mut f = Form::default();
        assert_eq!(f.check(), None);
        assert_eq!(f.error, "Enter your email address.");
        let mut f = Form { step: Step::SignUp, ..filled(Step::SignUp) };
        f.handle = "no spaces".into();
        assert_eq!(f.check(), None);
        assert!(f.error.starts_with("Use a username"));
        f.handle = "ok_name".into();
        f.password = "short".into();
        assert_eq!(f.check(), None);
        assert!(f.error.starts_with("Use a password"));
        let mut f = filled(Step::Code { reset: false });
        f.code = "12a456".into();
        assert_eq!(f.check(), None);
        assert!(f.error.contains("six digit"));
    }
    #[test]
    fn good_input_becomes_the_right_request() {
        assert_eq!(filled(Step::SignIn).check(), Some(Request::SignIn { email: "a@b.co".into(), password: "long enough".into() }));
        assert_eq!(filled(Step::Code { reset: false }).check(), Some(Request::Verify { email: "a@b.co".into(), code: "123456".into() }));
        assert_eq!(filled(Step::Code { reset: true }).check().map(|r| r.route()), Some("reset"));
        assert_eq!(Request::Register { email: "a@b.co".into(), password: "x".into(), handle: "h".into() }.body()["handle"], "h");
    }
    #[test]
    fn the_flow_moves_on_when_the_service_answers() {
        let mut f = filled(Step::SignUp);
        assert_eq!(f.done("register", Ok(())), None);
        assert_eq!(f.step, Step::Code { reset: false });
        assert!(f.info.contains("a@b.co"));
        // A confirmed email signs in with what was typed, with no further click.
        assert_eq!(f.done("verify", Ok(())), Some(Request::SignIn { email: "a@b.co".into(), password: "long enough".into() }));
        f.done("sign-in", Err("The email or password is wrong, or the email is not confirmed yet.".into()));
        assert!(f.error.contains("not confirmed"));
        let mut r = filled(Step::Code { reset: true });
        assert_eq!(r.done("reset", Ok(())), None);
        assert_eq!(r.step, Step::SignIn);
        assert!(r.info.starts_with("Password changed"));
    }
    #[test]
    fn email_username_and_password_rules_match_the_service() {
        assert!(email_ok("a@b.co") && !email_ok("a@b") && !email_ok("a b@c.de") && !email_ok("@b.co"));
        assert!(handle_ok("Mia_1") && !handle_ok("ab") && !handle_ok("a-b") && !handle_ok(&"a".repeat(31)));
        assert!(password_ok("12345678") && !password_ok("1234567") && !password_ok(&"a".repeat(257)));
    }
}
