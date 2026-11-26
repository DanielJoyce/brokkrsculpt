// SPDX-License-Identifier: AGPL-3.0-only

//! A TinkerAtlas account stored by an earlier build, so a bug report can be
//! replied to.
//!
//! Signing in was taken off the welcome screen on 2026-11-26, with the rest
//! of its links to that site, at the maintainer's word: *"Remove the links to
//! the external website."* The flow that did it (a loopback listener, a
//! nonce, the browser) went with the button. What stays is reading an account
//! an earlier build stored, because a bug report sent with one can be
//! answered; with none, a report goes out anonymous, as it always could.
//!
//! # The token
//!
//! It is a credential, and this repository is public. It is never logged,
//! never put in a status line, never included in a crash report, and never
//! rendered by [`std::fmt::Debug`] -- [`Account`] has a hand-written one for
//! exactly that reason, because a derived `Debug` on a struct that holds a
//! secret is one `dbg!` away from a leak.

use std::path::PathBuf;

/// The signed-in user, and the token that proves it.
#[derive(Clone)]
pub struct Account {
    pub username: String,
    pub display_name: String,
    /// Empty when the profile has no picture. Not fetched by this module.
    pub avatar_url: String,
    /// **Private, and it stays private.** Read it through
    /// [`Account::authorization`] so that every use is a deliberate one.
    token: String,
}

impl Account {
    /// The `Authorization` header value for an authenticated request.
    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// What to call this person on screen, preferring what they chose.
    pub fn label(&self) -> &str {
        if !self.display_name.is_empty() { &self.display_name } else { &self.username }
    }
}

/// Hand-written so that no accidental `{:?}` can ever print the token.
///
/// The derived form would put a live credential into whatever it was written
/// to -- a log line, a panic message, a crash report. This one names the field
/// without its value, so a `Debug` that was added for debugging stays that.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("avatar_url", &self.avatar_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Where the signed-in account is kept.
///
/// State and not config: it is something the application recovers, not
/// something the user set, and it must never be mistaken for a preference
/// worth syncing or committing.
fn account_file() -> Option<PathBuf> {
    crate::paths::state_file("tinkeratlas-account.json")
}

/// Read the stored account, if there is one.
///
/// Every failure is the same answer -- signed out. A corrupt or unreadable
/// file is not worth an error on the welcome screen: the remedy is to sign in
/// again, which is the button already on screen.
pub fn load() -> Option<Account> {
    let text = std::fs::read_to_string(account_file()?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let field = |name: &str| value.get(name).and_then(|v| v.as_str()).unwrap_or_default();

    let token = field("token").to_string();
    if token.is_empty() {
        return None;
    }
    Some(Account {
        username: field("username").to_string(),
        display_name: field("display_name").to_string(),
        avatar_url: field("avatar_url").to_string(),
        token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The token must not be printable by accident.** A derived `Debug` here
    /// would put a live credential into any log line or panic message that
    /// formatted an `Account`, and this repository is public.
    #[test]
    fn debug_never_prints_the_token() {
        let account = Account {
            username: "maker".to_string(),
            display_name: "Maker".to_string(),
            avatar_url: String::new(),
            token: "ta_bskt_supersecretvalue".to_string(),
        };
        let shown = format!("{account:?}");
        assert!(!shown.contains("supersecret"), "the token was printed: {shown}");
        assert!(shown.contains("redacted"), "the field vanished instead of being masked");
        // And the header is still built from the real thing.
        assert_eq!(account.authorization(), "Bearer ta_bskt_supersecretvalue");
    }

    #[test]
    fn label_prefers_the_name_the_user_chose() {
        let mut account = Account {
            username: "maker".to_string(),
            display_name: "Maker Viking".to_string(),
            avatar_url: String::new(),
            token: "t".to_string(),
        };
        assert_eq!(account.label(), "Maker Viking");
        account.display_name = String::new();
        assert_eq!(account.label(), "maker", "an empty display name hid the user");
    }
}
