//! Logging in, and staying logged in.
//!
//! The session and the encryption keys live in a directory belonging
//! to one account. Getting this wrong is expensive: a client that
//! silently reuses another account's session connects successfully and
//! then sees none of the conversations the caller expected.

use std::path::{Path, PathBuf};

use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::Client;

use crate::config::Config;
use crate::error::{Error, Result};

/// Name of the session file inside an account's store directory.
pub(crate) const SESSION_FILE: &str = "session.json";
/// Name of the file holding the sync position.
pub(crate) const SYNC_TOKEN_FILE: &str = "sync-token";

/// A password login.
///
/// Needed the first time only, or after the saved session stops being
/// accepted.
#[derive(Clone)]
pub struct Credentials {
    pub account: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    /// Shows the account but never the password, so a debug log cannot
    /// leak it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("account", &self.account)
            .field("password", &"<hidden>")
            .finish()
    }
}

/// Whether a session is already saved for this configuration.
pub fn has_saved_session(config: &Config) -> bool {
    config.account_store_path().join(SESSION_FILE).is_file()
}

/// Create the account's store directory, readable by its owner only.
pub(crate) fn prepare_store(path: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(path).map_err(|source| {
        Error::Store(format!(
            "cannot create the store directory {}: {source}",
            path.display()
        ))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| {
                Error::Store(format!(
                    "cannot restrict the store directory {}: {source}",
                    path.display()
                ))
            },
        )?;
    }

    path.canonicalize().map_err(|source| {
        Error::Store(format!(
            "cannot resolve the store directory {}: {source}",
            path.display()
        ))
    })
}

/// Just enough of a saved session to tell whose it is.
#[derive(serde::Deserialize)]
struct StoredAccount {
    user_id: String,
}

/// Refuse a store belonging to another account, and say what to do.
///
/// Checked before the session is restored: discovering the mismatch
/// afterwards would mean the wrong account is already loaded, leaving
/// the caller with a complaint and no remedy.
pub(crate) fn check_stored_account(store_path: &Path, configured: &str) -> Result<()> {
    let session_file = store_path.join(SESSION_FILE);
    let Ok(text) = std::fs::read_to_string(&session_file) else {
        return Ok(());
    };
    let stored: StoredAccount = serde_json::from_str(&text).map_err(|source| {
        Error::Store(format!(
            "the saved session at {} is unreadable: {source}; \
             remove {} to start again",
            session_file.display(),
            store_path.display()
        ))
    })?;
    if stored.user_id != configured {
        return Err(Error::Login(format!(
            "the store at {} holds a session for {}, but the configuration asks for {}; \
             remove that directory to log in as {}",
            store_path.display(),
            stored.user_id,
            configured,
            configured
        )));
    }
    Ok(())
}

/// Restore the saved session, or log in and save a new one.
pub(crate) async fn establish(
    client: &Client,
    config: &Config,
    credentials: Option<Credentials>,
    store_path: &Path,
) -> Result<()> {
    let session_file = store_path.join(SESSION_FILE);

    if session_file.exists() {
        let text = std::fs::read_to_string(&session_file).map_err(|source| {
            Error::Store(format!(
                "cannot read the saved session at {}: {source}",
                session_file.display()
            ))
        })?;
        let session: MatrixSession = serde_json::from_str(&text).map_err(|source| {
            Error::Store(format!(
                "the saved session at {} is unreadable: {source}",
                session_file.display()
            ))
        })?;
        client
            .restore_session(session)
            .await
            .map_err(|source| Error::Login(format!("the saved session was refused: {source}")))?;
        return Ok(());
    }

    let credentials = credentials.ok_or_else(|| {
        Error::Login(format!(
            "no session saved in {} and no password given",
            store_path.display()
        ))
    })?;

    client
        .matrix_auth()
        .login_username(&credentials.account, &credentials.password)
        .initial_device_display_name(&config.device_name)
        .await
        .map_err(|source| Error::Login(format!("the server refused the login: {source}")))?;

    let session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| Error::Login("the login left no session behind".to_owned()))?;
    let text = serde_json::to_string_pretty(&session)
        .map_err(|source| Error::Store(format!("cannot serialize the session: {source}")))?;
    write_private_file(&session_file, &text)
}

/// Refuse to run as an account other than the configured one.
pub(crate) fn verify_account(client: &Client, config: &Config) -> Result<()> {
    let actual = client
        .user_id()
        .ok_or_else(|| Error::Login("the client is not logged in".to_owned()))?;
    if actual.as_str() != config.account {
        return Err(Error::Login(format!(
            "logged in as {actual}, but the configuration asks for {}",
            config.account
        )));
    }
    Ok(())
}

/// Write a file only its owner can read.
pub(crate) fn write_private_file(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)
        .map_err(|source| Error::Store(format!("cannot write {}: {source}", path.display())))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
            |source| Error::Store(format!("cannot restrict {}: {source}", path.display())),
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    fn config_in(directory: &Path) -> Config {
        Config::from_yaml(&format!(
            indoc! {"
                service: matrix
                server: https://matrix.example.org
                account: \"@alice:example.org\"
                store-path: {}
            "},
            directory.display()
        ))
        .expect("the configuration is valid")
    }

    #[test]
    fn credentials_never_show_their_password() {
        let credentials = Credentials {
            account: "@alice:example.org".to_owned(),
            password: "hunter2".to_owned(),
        };
        let shown = format!("{credentials:?}");
        assert!(!shown.contains("hunter2"), "unexpected: {shown}");
        assert!(shown.contains("@alice:example.org"), "unexpected: {shown}");
    }

    #[test]
    fn a_store_directory_is_created_and_kept_private() {
        let parent = tempfile::tempdir().expect("a temporary directory");
        let store = parent.path().join("store");
        let resolved = prepare_store(&store).expect("the store can be created");
        assert!(resolved.is_dir());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&resolved)
                .expect("the store exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[test]
    fn preparing_an_existing_store_is_harmless() {
        let parent = tempfile::tempdir().expect("a temporary directory");
        let store = parent.path().join("store");
        let first = prepare_store(&store).expect("created");
        let second = prepare_store(&store).expect("prepared again");
        assert_eq!(first, second);
    }

    #[test]
    fn a_written_file_is_readable_by_its_owner_only() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let file = directory.path().join("secret");
        write_private_file(&file, "token").expect("written");
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "token");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file)
                .expect("exists")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn an_empty_store_belongs_to_nobody_yet() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        check_stored_account(directory.path(), "@alice:example.org")
            .expect("no session, no conflict");
    }

    #[test]
    fn a_store_holding_the_same_account_is_accepted() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join(SESSION_FILE),
            r#"{"user_id":"@alice:example.org","device_id":"AAA","access_token":"x"}"#,
        )
        .expect("written");
        check_stored_account(directory.path(), "@alice:example.org").expect("same account");
    }

    #[test]
    fn a_store_holding_another_account_is_refused() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join(SESSION_FILE),
            r#"{"user_id":"@other:example.org","device_id":"AAA","access_token":"x"}"#,
        )
        .expect("written");
        let error = check_stored_account(directory.path(), "@alice:example.org")
            .expect_err("the store belongs to another account");
        let message = error.to_string();
        assert!(
            message.contains("@other:example.org"),
            "unexpected: {message}"
        );
        assert!(
            message.contains("@alice:example.org"),
            "unexpected: {message}"
        );
    }

    #[test]
    fn refusing_a_foreign_store_says_how_to_recover() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join(SESSION_FILE),
            r#"{"user_id":"@other:example.org","device_id":"AAA","access_token":"x"}"#,
        )
        .expect("written");
        let error = check_stored_account(directory.path(), "@alice:example.org")
            .expect_err("another account");
        let message = error.to_string();
        assert!(message.contains("remove"), "unexpected: {message}");
        assert!(
            message.contains(&directory.path().display().to_string()),
            "the message must name the directory to remove: {message}"
        );
    }

    #[test]
    fn an_unreadable_session_is_reported_rather_than_ignored() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(directory.path().join(SESSION_FILE), "{ not json").expect("written");
        let error = check_stored_account(directory.path(), "@alice:example.org")
            .expect_err("corrupt session");
        assert!(
            error.to_string().contains("unreadable"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn a_saved_session_is_detected() {
        let parent = tempfile::tempdir().expect("a temporary directory");
        let config = config_in(parent.path());
        assert!(!has_saved_session(&config));

        let account_store = config.account_store_path();
        std::fs::create_dir_all(&account_store).expect("created");
        std::fs::write(account_store.join(SESSION_FILE), "{}").expect("written");
        assert!(has_saved_session(&config));
    }
}
