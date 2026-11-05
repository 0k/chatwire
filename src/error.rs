//! What can go wrong, said plainly.

use std::fmt;

/// What can go wrong.
#[derive(Debug)]
pub enum Error {
    /// The configuration is missing, unreadable or wrong.
    Config(String),
    /// The local store holding the session could not be used.
    Store(String),
    /// Logging in failed, or no usable session was found.
    Login(String),
    /// The conversation with the service failed.
    Sync(String),
    /// Writing to the service failed.
    Send(String),
    /// The conversation does not exist, or the account is not in it.
    NoSuchConversation(String),
    /// This service cannot do that.
    ///
    /// One interface over several services means some operations have
    /// no counterpart everywhere. Saying so plainly is better than
    /// pretending to succeed, and better than leaving the method out
    /// and forcing callers to know which backend they hold.
    Unsupported { service: String, operation: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config(detail) => write!(f, "configuration error: {detail}"),
            Error::Store(detail) => write!(f, "store error: {detail}"),
            Error::Login(detail) => write!(f, "login error: {detail}"),
            Error::Sync(detail) => write!(f, "sync error: {detail}"),
            Error::Send(detail) => write!(f, "send error: {detail}"),
            Error::NoSuchConversation(id) => {
                write!(f, "no such conversation, or not a member of it: {id}")
            }
            Error::Unsupported { service, operation } => {
                write!(f, "{service} cannot {operation}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_error_names_its_kind() {
        let error = Error::Config("missing homeserver".to_owned());
        assert_eq!(error.to_string(), "configuration error: missing homeserver");
    }

    #[test]
    fn a_login_error_names_its_kind() {
        let error = Error::Login("wrong password".to_owned());
        assert_eq!(error.to_string(), "login error: wrong password");
    }

    #[test]
    fn errors_are_usable_as_standard_errors() {
        fn accepts(_: &dyn std::error::Error) {}
        accepts(&Error::Sync("connection reset".to_owned()));
    }

    #[test]
    fn a_missing_conversation_names_the_one_asked_for() {
        let error = Error::NoSuchConversation("!room:example.org".to_owned());
        assert_eq!(
            error.to_string(),
            "no such conversation, or not a member of it: !room:example.org"
        );
    }

    #[test]
    fn an_unsupported_operation_names_the_service_and_the_operation() {
        let error = Error::Unsupported {
            service: "rocketchat".to_owned(),
            operation: "edit a message".to_owned(),
        };
        assert_eq!(error.to_string(), "rocketchat cannot edit a message");
    }
}
