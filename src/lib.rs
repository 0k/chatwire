//! One client interface over several instant messaging services.
//!
//! `chatwire` connects to a chat service as an ordinary account, the
//! way any desktop client does, and gives you two things:
//!
//! - every event that account sees, in a vocabulary that does not
//!   depend on the service;
//! - the means to write back: post, reply, edit, react, join, leave.
//!
//! It is a brick, not a program. It decides nothing, answers nobody by
//! itself, and holds no policy about what is worth reacting to. That
//! belongs to whatever uses it.
//!
//! # Connecting
//!
//! ```no_run
//! use chatwire::{Client, Config, Event, Outgoing};
//!
//! # async fn example() -> chatwire::Result<()> {
//! let config = Config::load(std::path::Path::new("config.yml"))?;
//! let client = chatwire::matrix::connect(&config, None).await?;
//! let mut events = client.subscribe();
//!
//! while let Some(event) = events.recv().await {
//!     if let Event::Message(message) = event {
//!         println!("{}: {}", message.sender.label(), message.body);
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Writing
//!
//! ```no_run
//! # use chatwire::{Client, Outgoing};
//! # async fn example(client: &dyn Client, room: &str, question: &str)
//! # -> chatwire::Result<()> {
//! client.send(room, Outgoing::text("on it")).await?;
//! client.send(room, Outgoing::text("done").reply_to(question)).await?;
//! client.send(room, Outgoing::text("built ok").as_notice()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Both directions are events
//!
//! Messages the connected account sends are delivered too, marked
//! [`Direction::Outgoing`]. They are not noise: knowing that you
//! already answered is what stops a program answering again.
//!
//! # Handing events elsewhere
//!
//! [`Envelope`] turns any event into JSON named `chat.<service>.<kind>`,
//! ready for a dispatcher such as vigil-watch. The crate produces the
//! envelope; delivering it is the caller's business.

#![recursion_limit = "512"]

mod client;
mod config;
mod dedup;
mod envelope;
mod error;
mod event;

#[cfg(feature = "matrix")]
pub mod matrix;

pub use client::{Client, Events, Outgoing};
pub use config::Config;
pub use dedup::SeenEvents;
pub use envelope::{event_type_for, Envelope, Payload};
pub use error::{Error, Result};
pub use event::{
    Body, Connection, Conversation, Direction, Event, EventKind, Membership, Message, Participant,
    Service, Subscription,
};
