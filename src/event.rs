//! The vocabulary every service is translated into.
//!
//! These types are the whole point of the crate: a Matrix room and a
//! Rocket.Chat channel arrive here as the same [`Conversation`], and a
//! message from a friend arrives as the same [`Event::Message`]
//! whichever service carried it.
//!
//! Nothing here is service specific. A backend that cannot express
//! something in this vocabulary reports [`Event::Unsupported`] rather
//! than inventing a shape of its own.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Which service an event came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Service {
    Matrix,
    RocketChat,
}

impl Service {
    /// Short, stable tag used in event names and configuration.
    pub fn tag(&self) -> &'static str {
        match self {
            Service::Matrix => "matrix",
            Service::RocketChat => "rocketchat",
        }
    }
}

impl fmt::Display for Service {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.tag())
    }
}

/// A room, channel, or direct conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    /// Identifier, as the service knows it. Stable, opaque, and what
    /// you pass back to write into this conversation.
    pub id: String,
    /// Human readable name, when the service offers one.
    pub name: Option<String>,
    /// Whether the conversation is end-to-end encrypted.
    pub encrypted: bool,
    /// Whether the conversation is a one-to-one discussion.
    pub direct: bool,
}

impl Conversation {
    /// The name if there is one, the identifier otherwise.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Somebody taking part in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    /// Identifier, as the service knows it.
    pub id: String,
    /// Display name, when the service offers one.
    pub name: Option<String>,
}

impl Participant {
    /// The name if there is one, the identifier otherwise.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Who wrote a message, relative to the connected account.
///
/// The account's own messages are reported, never hidden: a program
/// following a conversation needs to know what its own side said. A
/// reply of "thanks, fixed it" is as meaningful as the question that
/// preceded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Direction {
    /// Written by somebody else.
    Incoming,
    /// Written by the connected account, from this client or another.
    Outgoing,
}

/// What a message turned out to contain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Body {
    /// Text, as the sender typed it.
    Text { text: String },
    /// An attachment. `kind` is the service's own type name, such as
    /// `m.image`, and `name` the file name when there is one.
    Media {
        media_kind: String,
        name: Option<String>,
    },
    /// An encrypted message this device holds no key for.
    ///
    /// Reported rather than dropped: a program must be able to tell
    /// "nothing was said" from "something was said that I cannot
    /// read".
    Undecryptable { reason: String },
}

impl fmt::Display for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Body::Text { text } => write!(f, "{text}"),
            Body::Media { media_kind, name } => match name {
                Some(name) => write!(f, "<{media_kind}: {name}>"),
                None => write!(f, "<{media_kind}>"),
            },
            Body::Undecryptable { reason } => write!(f, "<undecryptable: {reason}>"),
        }
    }
}

/// A message, in either direction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Identifier of this message, usable for deduplication and for
    /// replying to it.
    pub id: String,
    pub conversation: Conversation,
    pub sender: Participant,
    pub direction: Direction,
    pub body: Body,
    /// The message this one replies to, when it is a reply.
    pub replies_to: Option<String>,
    /// Milliseconds since the Unix epoch, as the service dates it.
    pub timestamp_ms: i64,
}

/// How the connection to the service is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Connection {
    /// The client is connected and receiving.
    Connected { account: String, device: String },
    /// The connection dropped; the client will try again.
    Lost { reason: String },
    /// The connection came back after a loss.
    Restored,
}

/// What happened to somebody's membership of a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Membership {
    Invited,
    Joined,
    Left,
}

/// Everything the crate can tell you about.
///
/// New variants may be added as backends learn to report more, so
/// match with a catch-all arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Event {
    /// A message arrived, or the account sent one.
    Message(Message),
    /// A message was edited. `body` is the new content.
    MessageEdited {
        id: String,
        conversation: Conversation,
        sender: Participant,
        direction: Direction,
        body: Body,
        timestamp_ms: i64,
    },
    /// A message was deleted or redacted.
    MessageRemoved {
        id: String,
        conversation: Conversation,
        removed_by: Participant,
        timestamp_ms: i64,
    },
    /// A reaction was added to a message.
    Reaction {
        message_id: String,
        conversation: Conversation,
        sender: Participant,
        reaction: String,
        timestamp_ms: i64,
    },
    /// Somebody's membership of a conversation changed.
    MembershipChanged {
        conversation: Conversation,
        participant: Participant,
        membership: Membership,
        timestamp_ms: i64,
    },
    /// Somebody started or stopped typing.
    Typing {
        conversation: Conversation,
        participants: Vec<Participant>,
    },
    /// Somebody read up to a given message.
    Receipt {
        conversation: Conversation,
        participant: Participant,
        message_id: String,
        timestamp_ms: i64,
    },
    /// The state of the connection changed.
    Connection(Connection),
    /// The service reported something this vocabulary cannot express.
    ///
    /// Carries the service's own type name so nothing is silently
    /// lost, and so a gap in the vocabulary is visible rather than
    /// invisible.
    Unsupported {
        service_event: String,
        conversation: Option<Conversation>,
    },
}

/// The kind of an [`Event`], without its content.
///
/// Used to say what you want to receive, and to name an event when it
/// is handed to another system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Message,
    MessageEdited,
    MessageRemoved,
    Reaction,
    MembershipChanged,
    Typing,
    Receipt,
    Connection,
    Unsupported,
}

impl EventKind {
    /// Every kind, in a stable order.
    pub const ALL: [EventKind; 9] = [
        EventKind::Message,
        EventKind::MessageEdited,
        EventKind::MessageRemoved,
        EventKind::Reaction,
        EventKind::MembershipChanged,
        EventKind::Typing,
        EventKind::Receipt,
        EventKind::Connection,
        EventKind::Unsupported,
    ];

    /// Short, stable tag, used in configuration and in event names.
    pub fn tag(&self) -> &'static str {
        match self {
            EventKind::Message => "message",
            EventKind::MessageEdited => "message-edited",
            EventKind::MessageRemoved => "message-removed",
            EventKind::Reaction => "reaction",
            EventKind::MembershipChanged => "membership-changed",
            EventKind::Typing => "typing",
            EventKind::Receipt => "receipt",
            EventKind::Connection => "connection",
            EventKind::Unsupported => "unsupported",
        }
    }

    /// Parse a tag back into a kind.
    pub fn from_tag(tag: &str) -> Option<Self> {
        EventKind::ALL.into_iter().find(|kind| kind.tag() == tag)
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.tag())
    }
}

impl Event {
    /// The kind of this event.
    pub fn kind(&self) -> EventKind {
        match self {
            Event::Message(_) => EventKind::Message,
            Event::MessageEdited { .. } => EventKind::MessageEdited,
            Event::MessageRemoved { .. } => EventKind::MessageRemoved,
            Event::Reaction { .. } => EventKind::Reaction,
            Event::MembershipChanged { .. } => EventKind::MembershipChanged,
            Event::Typing { .. } => EventKind::Typing,
            Event::Receipt { .. } => EventKind::Receipt,
            Event::Connection(_) => EventKind::Connection,
            Event::Unsupported { .. } => EventKind::Unsupported,
        }
    }

    /// The conversation this event concerns, when it concerns one.
    pub fn conversation(&self) -> Option<&Conversation> {
        match self {
            Event::Message(message) => Some(&message.conversation),
            Event::MessageEdited { conversation, .. }
            | Event::MessageRemoved { conversation, .. }
            | Event::Reaction { conversation, .. }
            | Event::MembershipChanged { conversation, .. }
            | Event::Typing { conversation, .. }
            | Event::Receipt { conversation, .. } => Some(conversation),
            Event::Connection(_) => None,
            Event::Unsupported { conversation, .. } => conversation.as_ref(),
        }
    }

    /// Whether the connected account caused this event.
    ///
    /// `None` when the notion does not apply, as for a connection
    /// state change.
    pub fn direction(&self) -> Option<Direction> {
        match self {
            Event::Message(message) => Some(message.direction),
            Event::MessageEdited { direction, .. } => Some(*direction),
            _ => None,
        }
    }
}

/// What a caller wants to receive.
///
/// The default is everything, including the account's own messages:
/// a caller that asked for a connection to a conversation presumably
/// wants to know what happens in it. Narrow it deliberately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    kinds: Vec<EventKind>,
}

impl Default for Subscription {
    fn default() -> Self {
        Self::all()
    }
}

impl Subscription {
    /// Every kind of event.
    pub fn all() -> Self {
        Self {
            kinds: EventKind::ALL.to_vec(),
        }
    }

    /// Only the listed kinds.
    pub fn only(kinds: &[EventKind]) -> Self {
        let mut kinds = kinds.to_vec();
        kinds.sort();
        kinds.dedup();
        Self { kinds }
    }

    /// Every kind except the listed ones.
    pub fn except(kinds: &[EventKind]) -> Self {
        Self {
            kinds: EventKind::ALL
                .into_iter()
                .filter(|kind| !kinds.contains(kind))
                .collect(),
        }
    }

    /// Whether this subscription wants that kind.
    pub fn wants(&self, kind: EventKind) -> bool {
        self.kinds.contains(&kind)
    }

    /// The kinds this subscription covers.
    pub fn kinds(&self) -> &[EventKind] {
        &self.kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation() -> Conversation {
        Conversation {
            id: "!room:example.org".to_owned(),
            name: Some("project".to_owned()),
            encrypted: false,
            direct: false,
        }
    }

    fn participant() -> Participant {
        Participant {
            id: "@alice:example.org".to_owned(),
            name: Some("Alice".to_owned()),
        }
    }

    fn message(direction: Direction) -> Message {
        Message {
            id: "$one".to_owned(),
            conversation: conversation(),
            sender: participant(),
            direction,
            body: Body::Text {
                text: "hello".to_owned(),
            },
            replies_to: None,
            timestamp_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn a_conversation_prefers_its_name_as_a_label() {
        assert_eq!(conversation().label(), "project");
    }

    #[test]
    fn a_conversation_without_a_name_falls_back_to_its_id() {
        let unnamed = Conversation {
            name: None,
            ..conversation()
        };
        assert_eq!(unnamed.label(), "!room:example.org");
    }

    #[test]
    fn a_participant_prefers_their_name_as_a_label() {
        assert_eq!(participant().label(), "Alice");
    }

    #[test]
    fn a_participant_without_a_name_falls_back_to_their_id() {
        let unnamed = Participant {
            id: "@bob:example.org".to_owned(),
            name: None,
        };
        assert_eq!(unnamed.label(), "@bob:example.org");
    }

    #[test]
    fn every_service_has_a_stable_tag() {
        assert_eq!(Service::Matrix.tag(), "matrix");
        assert_eq!(Service::RocketChat.tag(), "rocketchat");
    }

    #[test]
    fn an_undecryptable_body_says_so_when_displayed() {
        let body = Body::Undecryptable {
            reason: "no key".to_owned(),
        };
        assert_eq!(body.to_string(), "<undecryptable: no key>");
    }

    #[test]
    fn a_media_body_names_its_file_when_there_is_one() {
        let body = Body::Media {
            media_kind: "m.image".to_owned(),
            name: Some("photo.png".to_owned()),
        };
        assert_eq!(body.to_string(), "<m.image: photo.png>");
    }

    #[test]
    fn a_media_body_without_a_name_shows_its_kind() {
        let body = Body::Media {
            media_kind: "m.image".to_owned(),
            name: None,
        };
        assert_eq!(body.to_string(), "<m.image>");
    }

    #[test]
    fn an_event_reports_its_own_kind() {
        assert_eq!(
            Event::Message(message(Direction::Incoming)).kind(),
            EventKind::Message
        );
    }

    #[test]
    fn a_message_event_exposes_its_conversation() {
        let event = Event::Message(message(Direction::Incoming));
        assert_eq!(
            event.conversation().map(|c| c.id.as_str()),
            Some("!room:example.org")
        );
    }

    #[test]
    fn a_connection_event_concerns_no_conversation() {
        let event = Event::Connection(Connection::Restored);
        assert!(event.conversation().is_none());
    }

    #[test]
    fn a_message_event_reports_its_direction() {
        let event = Event::Message(message(Direction::Outgoing));
        assert_eq!(event.direction(), Some(Direction::Outgoing));
    }

    #[test]
    fn a_connection_event_has_no_direction() {
        let event = Event::Connection(Connection::Restored);
        assert_eq!(event.direction(), None);
    }

    #[test]
    fn every_kind_has_a_tag_that_parses_back() {
        for kind in EventKind::ALL {
            assert_eq!(EventKind::from_tag(kind.tag()), Some(kind), "{kind}");
        }
    }

    #[test]
    fn an_unknown_tag_does_not_parse() {
        assert_eq!(EventKind::from_tag("nonsense"), None);
    }

    #[test]
    fn kind_tags_are_all_distinct() {
        let mut tags: Vec<&str> = EventKind::ALL.iter().map(|kind| kind.tag()).collect();
        let count = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), count);
    }

    #[test]
    fn subscribing_by_default_takes_everything() {
        let subscription = Subscription::default();
        for kind in EventKind::ALL {
            assert!(subscription.wants(kind), "{kind} should be wanted");
        }
    }

    #[test]
    fn own_messages_are_delivered_rather_than_filtered() {
        // The account's own traffic is a labelled direction, never a
        // hidden event: this is what lets a caller notice its own
        // side of a conversation.
        let event = Event::Message(message(Direction::Outgoing));
        assert!(Subscription::default().wants(event.kind()));
        assert_eq!(event.direction(), Some(Direction::Outgoing));
    }

    #[test]
    fn subscribing_to_some_kinds_excludes_the_others() {
        let subscription = Subscription::only(&[EventKind::Message, EventKind::Reaction]);
        assert!(subscription.wants(EventKind::Message));
        assert!(subscription.wants(EventKind::Reaction));
        assert!(!subscription.wants(EventKind::Typing));
    }

    #[test]
    fn subscribing_to_a_kind_twice_lists_it_once() {
        let subscription = Subscription::only(&[EventKind::Message, EventKind::Message]);
        assert_eq!(subscription.kinds(), &[EventKind::Message]);
    }

    #[test]
    fn excluding_a_kind_keeps_all_the_others() {
        let subscription = Subscription::except(&[EventKind::Typing]);
        assert!(!subscription.wants(EventKind::Typing));
        assert!(subscription.wants(EventKind::Message));
        assert_eq!(subscription.kinds().len(), EventKind::ALL.len() - 1);
    }

    #[test]
    fn an_event_survives_a_round_trip_through_json() {
        let event = Event::Message(message(Direction::Incoming));
        let text = serde_json::to_string(&event).expect("an event serializes");
        let parsed: Event = serde_json::from_str(&text).expect("an event parses back");
        assert_eq!(parsed, event);
    }

    #[test]
    fn an_undecryptable_message_survives_a_round_trip() {
        let mut original = message(Direction::Incoming);
        original.body = Body::Undecryptable {
            reason: "no key".to_owned(),
        };
        let event = Event::Message(original);
        let text = serde_json::to_string(&event).expect("an event serializes");
        let parsed: Event = serde_json::from_str(&text).expect("an event parses back");
        assert_eq!(parsed, event);
    }
}
