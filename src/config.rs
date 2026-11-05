//! What a caller must say to reach an account.
//!
//! One shape for every service: which service, which server, which
//! account, and where the local state lives. The password is never
//! part of it.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::event::Service;

fn default_store_path() -> PathBuf {
    PathBuf::from("./store")
}

fn default_device_name() -> String {
    "chatwire".to_owned()
}

/// How to reach one account on one service.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    /// Which service to talk to.
    pub service: Service,
    /// Base URL of the server, e.g. `https://matrix.example.org`.
    pub server: String,
    /// The account, in whatever form the service uses. For Matrix a
    /// full user id such as `@someone:example.org`.
    pub account: String,
    /// Directory holding the saved session, and for Matrix the
    /// encryption state. Each account gets its own subdirectory.
    #[serde(default = "default_store_path")]
    pub store_path: PathBuf,
    /// Name this client shows as in the account's device list.
    #[serde(default = "default_device_name")]
    pub device_name: String,
}

impl Config {
    /// Parse a configuration from YAML text.
    pub fn from_yaml(text: &str) -> Result<Self> {
        let config: Config = serde_norway::from_str(text)
            .map_err(|source| Error::Config(format!("cannot parse the configuration: {source}")))?;
        config.validate()?;
        Ok(config)
    }

    /// Read a configuration from a file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|source| {
            Error::Config(format!(
                "cannot read the configuration at {}: {source}",
                path.display()
            ))
        })?;
        Self::from_yaml(&text)
    }

    /// Where this account's own state lives.
    ///
    /// Two accounts never share a directory, so changing account is a
    /// fresh login rather than a collision with the session already
    /// saved.
    pub fn account_store_path(&self) -> PathBuf {
        self.store_path.join(directory_name_for(&self.account))
    }

    fn validate(&self) -> Result<()> {
        if !self.server.starts_with("http://") && !self.server.starts_with("https://") {
            return Err(Error::Config(format!(
                "the server must be an http or https URL, got {:?}",
                self.server
            )));
        }
        if self.account.trim().is_empty() {
            return Err(Error::Config("the account cannot be empty".to_owned()));
        }
        if self.service == Service::Matrix
            && (!self.account.starts_with('@') || !self.account.contains(':'))
        {
            return Err(Error::Config(format!(
                "a matrix account must look like @someone:server, got {:?}",
                self.account
            )));
        }
        if self.device_name.trim().is_empty() {
            return Err(Error::Config("the device name cannot be empty".to_owned()));
        }
        Ok(())
    }
}

/// Turn an account name into a single, predictable path component.
///
/// `@someone:example.org` becomes `someone_example.org`; anything that is not a
/// letter, a digit, a dash or a dot becomes an underscore, so no
/// account name can escape the store directory.
pub(crate) fn directory_name_for(account: &str) -> String {
    let trimmed = account.strip_prefix('@').unwrap_or(account);
    trimmed
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '.' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    const MINIMAL: &str = indoc! {"
        service: matrix
        server: https://matrix.example.org
        account: \"@someone:example.org\"
    "};

    #[test]
    fn a_minimal_configuration_is_accepted() {
        let config = Config::from_yaml(MINIMAL).expect("the minimal configuration is valid");
        assert_eq!(config.service, Service::Matrix);
        assert_eq!(config.server, "https://matrix.example.org");
        assert_eq!(config.account, "@someone:example.org");
    }

    #[test]
    fn the_store_path_has_a_default() {
        let config = Config::from_yaml(MINIMAL).expect("the minimal configuration is valid");
        assert_eq!(config.store_path, PathBuf::from("./store"));
    }

    #[test]
    fn the_device_name_has_a_default() {
        let config = Config::from_yaml(MINIMAL).expect("the minimal configuration is valid");
        assert_eq!(config.device_name, "chatwire");
    }

    #[test]
    fn every_field_can_be_given_explicitly() {
        let config = Config::from_yaml(indoc! {"
            service: matrix
            server: https://matrix.example.org
            account: \"@bot:example.org\"
            store-path: /var/lib/chatwire
            device-name: watcher
        "})
        .expect("the configuration is valid");
        assert_eq!(config.store_path, PathBuf::from("/var/lib/chatwire"));
        assert_eq!(config.device_name, "watcher");
    }

    #[test]
    fn rocket_chat_is_a_known_service() {
        let config = Config::from_yaml(indoc! {"
            service: rocket-chat
            server: https://chat.example.org
            account: someone
        "})
        .expect("the configuration is valid");
        assert_eq!(config.service, Service::RocketChat);
    }

    #[test]
    fn a_rocket_chat_account_needs_no_at_sign() {
        let config = Config::from_yaml(indoc! {"
            service: rocket-chat
            server: https://chat.example.org
            account: someone
        "})
        .expect("only matrix requires the @server form");
        assert_eq!(config.account, "someone");
    }

    #[test]
    fn a_server_without_a_scheme_is_refused() {
        let error = Config::from_yaml(indoc! {"
            service: matrix
            server: matrix.example.org
            account: \"@someone:example.org\"
        "})
        .expect_err("a bare hostname is not a URL");
        assert!(error.to_string().contains("http"), "unexpected: {error}");
    }

    #[test]
    fn a_matrix_account_without_an_at_sign_is_refused() {
        let error = Config::from_yaml(indoc! {"
            service: matrix
            server: https://matrix.example.org
            account: someone:example.org
        "})
        .expect_err("a matrix account must start with @");
        assert!(
            error.to_string().contains("matrix account"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn an_unknown_service_is_refused() {
        let error = Config::from_yaml(indoc! {"
            service: telepathy
            server: https://example.org
            account: someone
        "})
        .expect_err("only known services are accepted");
        assert!(
            error.to_string().contains("telepathy"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let error = Config::from_yaml(indoc! {"
            service: matrix
            server: https://matrix.example.org
            account: \"@someone:example.org\"
            password: hunter2
        "})
        .expect_err("an unknown key is a mistake worth reporting");
        assert!(
            error.to_string().contains("password"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn underscored_keys_are_refused_so_the_spelling_stays_consistent() {
        let error = Config::from_yaml(indoc! {"
            service: matrix
            server: https://matrix.example.org
            account: \"@someone:example.org\"
            store_path: ./store
        "})
        .expect_err("keys are dash-cased");
        assert!(
            error.to_string().contains("store_path"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn an_account_gets_its_own_directory_under_the_store() {
        let config = Config::from_yaml(indoc! {"
            service: matrix
            server: https://matrix.example.org
            account: \"@alice:example.org\"
            store-path: /var/lib/chatwire
        "})
        .expect("the configuration is valid");
        assert_eq!(
            config.account_store_path(),
            PathBuf::from("/var/lib/chatwire/alice_example.org")
        );
    }

    #[test]
    fn two_accounts_never_share_a_directory() {
        let one = Config::from_yaml(MINIMAL).expect("valid");
        let other = Config::from_yaml(indoc! {"
            service: matrix
            server: https://matrix.example.org
            account: \"@other:example.org\"
        "})
        .expect("valid");
        assert_ne!(one.account_store_path(), other.account_store_path());
    }

    #[test]
    fn an_account_name_cannot_escape_the_store_directory() {
        let name = directory_name_for("@../../etc:example.org");
        assert!(!name.contains('/'), "unexpected: {name}");
        assert_eq!(PathBuf::from(&name).components().count(), 1);
    }
}
