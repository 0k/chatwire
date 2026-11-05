//! Connect an account and print everything that happens to it.
//!
//! Reads `config.yml` from the current directory, asks for the
//! password on the first run, then stays on the console until Ctrl-C.
//!
//! It never writes to the service. Pass `--envelopes` to print the
//! JSON a dispatcher such as vigil-watch would receive instead of the
//! human readable lines.

use std::path::PathBuf;

use chatwire::matrix::Credentials;
use chatwire::{Client, Config, Envelope, Event, Result};

/// Environment variable holding the password, for runs with no
/// terminal.
const PASSWORD_VARIABLE: &str = "CHATWIRE_PASSWORD";

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut envelopes = false;
    let mut config_path = PathBuf::from("config.yml");
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--envelopes" => envelopes = true,
            "--help" | "-h" => {
                println!("usage: watch [--envelopes] [CONFIG]");
                return Ok(());
            }
            other => config_path = PathBuf::from(other),
        }
    }

    let config = Config::load(&config_path)?;
    println!("service : {}", config.service);
    println!("server  : {}", config.server);
    println!("account : {}", config.account);
    println!("store   : {}", config.account_store_path().display());

    let credentials = ask_for_password_if_needed(&config)?;

    println!("connecting...");
    let client = chatwire::matrix::connect(&config, credentials).await?;
    println!(
        "connected as {} on device {}",
        client.account(),
        client.device().unwrap_or_else(|| "<unknown>".to_owned())
    );

    let conversations = client.conversations().await?;
    if conversations.is_empty() {
        let account = client.account();
        Box::new(client).shutdown().await;
        return Err(format!(
            "{account} takes part in no conversation, so there is nothing to watch.\n\
             Waiting would print nothing, whatever gets posted elsewhere.\n\
             Check that this is the account you chat with, then invite it to a \
             conversation or join one from another client."
        )
        .into());
    }

    println!("{} conversation(s):", conversations.len());
    for conversation in &conversations {
        let marks = match (conversation.encrypted, conversation.direct) {
            (true, true) => " [encrypted, direct]",
            (true, false) => " [encrypted]",
            (false, true) => " [direct]",
            (false, false) => "",
        };
        println!("  {}{}", conversation.label(), marks);
    }

    println!("\nwatching everything, including what this account sends. Ctrl-C to stop\n");

    let service = client.service();
    let mut events = client.subscribe();
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else {
                    println!("the connection stopped");
                    break;
                };
                if envelopes {
                    println!("{}", Envelope::new(service, &event)?.to_json()?);
                } else {
                    println!("{}", describe(&event));
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nstopping");
                break;
            }
        }
    }

    Box::new(client).shutdown().await;
    Ok(())
}

/// One readable line per event.
fn describe(event: &Event) -> String {
    let place = event
        .conversation()
        .map(|conversation| truncate(conversation.label(), 22))
        .unwrap_or_else(|| "-".to_owned());
    let kind = event.kind().to_string();

    let detail = match event {
        Event::Message(message) => {
            let arrow = match message.direction {
                chatwire::Direction::Incoming => "<-",
                chatwire::Direction::Outgoing => "->",
            };
            format!("{arrow} {}: {}", message.sender.label(), message.body)
        }
        Event::MessageRemoved { removed_by, id, .. } => {
            format!("{} removed {id}", removed_by.label())
        }
        Event::MessageEdited { sender, body, .. } => {
            format!("{} edited: {body}", sender.label())
        }
        Event::Reaction {
            sender, reaction, ..
        } => format!("{} reacted {reaction}", sender.label()),
        Event::MembershipChanged {
            participant,
            membership,
            ..
        } => format!("{} is now {membership:?}", participant.label()),
        Event::Typing { participants, .. } => {
            if participants.is_empty() {
                "nobody is typing".to_owned()
            } else {
                let names: Vec<&str> = participants.iter().map(|who| who.label()).collect();
                format!("{} is typing", names.join(", "))
            }
        }
        Event::Receipt {
            participant,
            message_id,
            ..
        } => format!("{} read {message_id}", participant.label()),
        Event::Connection(connection) => format!("{connection:?}"),
        Event::Unsupported { service_event, .. } => format!("unsupported: {service_event}"),
        _ => String::new(),
    };

    format!("{:<18} {:<22} {}", kind, place, detail)
}

/// Ask for the password only when no session is saved yet.
fn ask_for_password_if_needed(config: &Config) -> Result<Option<Credentials>> {
    if chatwire::matrix::has_saved_session(config) {
        println!("using the session saved for {}", config.account);
        return Ok(None);
    }

    println!("no saved session, a password is needed for the first login");
    let password = match std::env::var(PASSWORD_VARIABLE) {
        Ok(password) if !password.is_empty() => {
            println!("taking the password from {PASSWORD_VARIABLE}");
            password
        }
        _ => rpassword::prompt_password(format!("password for {}: ", config.account)).map_err(
            |source| {
                chatwire::Error::Login(format!(
                    "cannot read the password: {source}; \
                     set {PASSWORD_VARIABLE} when there is no terminal"
                ))
            },
        )?,
    };

    Ok(Some(Credentials {
        account: config.account.clone(),
        password,
    }))
}

/// Keep a column narrow enough to stay readable.
fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}
