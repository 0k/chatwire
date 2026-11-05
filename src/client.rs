//! The interface every service is reached through.
//!
//! A caller writes against [`Client`] and never against a particular
//! service. Swapping Matrix for Rocket.Chat changes which constructor
//! is called, and nothing else.
//!
//! Reading and writing are equal halves here. A client that could only
//! listen would be useless to a bot, and a client that could only post
//! would be useless to a watcher.

use async_trait::async_trait;

use crate::error::Result;
use crate::event::{Conversation, Event, Service, Subscription};

/// The receiving end of a client's event stream.
///
/// Dropping it does not disturb the client: other subscribers keep
/// receiving, and the connection stays up.
pub struct Events {
    inner: tokio::sync::broadcast::Receiver<Event>,
}

impl Events {
    /// Wrap a broadcast receiver. Backends use this; callers get an
    /// `Events` from [`Client::subscribe`].
    pub fn new(inner: tokio::sync::broadcast::Receiver<Event>) -> Self {
        Self { inner }
    }

    /// Wait for the next event.
    ///
    /// Returns `None` once the client has stopped. A subscriber too
    /// slow to keep up loses the oldest events; this is reported on
    /// standard error and reception continues, because a late event is
    /// worth more than a stalled stream.
    pub async fn recv(&mut self) -> Option<Event> {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match self.inner.recv().await {
                Ok(event) => return Some(event),
                Err(RecvError::Closed) => return None,
                Err(RecvError::Lagged(missed)) => {
                    eprintln!("warning: subscriber too slow, {missed} event(s) dropped");
                }
            }
        }
    }
}

/// Text to post, and where it goes.
///
/// Built with [`Outgoing::text`], then refined: `.reply_to(id)` makes
/// it a reply, `.as_notice()` marks it as coming from a program rather
/// than a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    text: String,
    reply_to: Option<String>,
    notice: bool,
}

impl Outgoing {
    /// A plain text message.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reply_to: None,
            notice: false,
        }
    }

    /// Make it a reply to an existing message.
    pub fn reply_to(mut self, message_id: impl Into<String>) -> Self {
        self.reply_to = Some(message_id.into());
        self
    }

    /// Mark it as automated.
    ///
    /// Services display such messages differently, and well behaved
    /// bots ignore them, which is what stops two programs answering
    /// each other forever.
    pub fn as_notice(mut self) -> Self {
        self.notice = true;
        self
    }

    /// The text to post.
    pub fn body(&self) -> &str {
        &self.text
    }

    /// The message replied to, when this is a reply.
    pub fn in_reply_to(&self) -> Option<&str> {
        self.reply_to.as_deref()
    }

    /// Whether this is marked as automated.
    pub fn is_notice(&self) -> bool {
        self.notice
    }
}

/// A connected account on a chat service.
///
/// The connection is live for as long as the value exists: it is
/// receiving in the background, and every subscriber sees what
/// arrives.
#[async_trait]
pub trait Client: Send + Sync {
    /// Which service this client speaks to.
    fn service(&self) -> Service;

    /// The account this client is logged in as.
    fn account(&self) -> String;

    /// The device or session this client appears as, when the service
    /// has such a notion.
    fn device(&self) -> Option<String>;

    /// Start receiving everything.
    fn subscribe(&self) -> Events;

    /// Start receiving only what the subscription asks for.
    fn subscribe_to(&self, subscription: Subscription) -> Events;

    /// The conversations the account currently takes part in.
    async fn conversations(&self) -> Result<Vec<Conversation>>;

    /// Post a message, and return its identifier.
    async fn send(&self, conversation: &str, message: Outgoing) -> Result<String>;

    /// Replace the text of a message the account sent.
    async fn edit(&self, conversation: &str, message_id: &str, text: &str) -> Result<()>;

    /// Delete a message.
    async fn remove(&self, conversation: &str, message_id: &str) -> Result<()>;

    /// Add a reaction to a message.
    async fn react(&self, conversation: &str, message_id: &str, reaction: &str) -> Result<()>;

    /// Say the account is typing, or has stopped.
    async fn typing(&self, conversation: &str, typing: bool) -> Result<()>;

    /// Mark the conversation read up to a message.
    async fn mark_read(&self, conversation: &str, message_id: &str) -> Result<()>;

    /// Join a conversation.
    async fn join(&self, conversation: &str) -> Result<()>;

    /// Leave a conversation.
    async fn leave(&self, conversation: &str) -> Result<()>;

    /// Stop receiving and release the connection.
    async fn shutdown(self: Box<Self>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_message_carries_its_text() {
        let outgoing = Outgoing::text("hello");
        assert_eq!(outgoing.body(), "hello");
    }

    #[test]
    fn a_plain_message_is_not_a_reply() {
        assert_eq!(Outgoing::text("hello").in_reply_to(), None);
    }

    #[test]
    fn a_plain_message_is_not_automated() {
        assert!(!Outgoing::text("hello").is_notice());
    }

    #[test]
    fn a_reply_remembers_what_it_answers() {
        let outgoing = Outgoing::text("on it").reply_to("$question");
        assert_eq!(outgoing.in_reply_to(), Some("$question"));
    }

    #[test]
    fn a_notice_is_marked_as_automated() {
        assert!(Outgoing::text("built ok").as_notice().is_notice());
    }

    #[test]
    fn refinements_can_be_combined() {
        let outgoing = Outgoing::text("done").reply_to("$ask").as_notice();
        assert_eq!(outgoing.body(), "done");
        assert_eq!(outgoing.in_reply_to(), Some("$ask"));
        assert!(outgoing.is_notice());
    }

    #[tokio::test]
    async fn a_subscriber_receives_what_is_published() {
        let (sender, receiver) = tokio::sync::broadcast::channel(4);
        let mut events = Events::new(receiver);
        sender
            .send(Event::Connection(crate::event::Connection::Restored))
            .expect("the subscriber is listening");
        assert_eq!(
            events.recv().await,
            Some(Event::Connection(crate::event::Connection::Restored))
        );
    }

    #[tokio::test]
    async fn a_subscriber_stops_when_the_client_goes_away() {
        let (sender, receiver) = tokio::sync::broadcast::channel::<Event>(4);
        let mut events = Events::new(receiver);
        drop(sender);
        assert_eq!(events.recv().await, None);
    }
}
