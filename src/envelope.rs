//! Handing events to other programs.
//!
//! An [`Event`](crate::Event) is a Rust value; a program outside this
//! process needs JSON. This module fixes the shape of that JSON so
//! that consumers can rely on it.
//!
//! The shape is the one vigil-watch dispatches on, since that is the
//! first intended consumer: a type naming the event, and a free-form
//! object holding it.
//!
//! ```json
//! {
//!   "directory": "chatwire",
//!   "payload": {
//!     "type": "chat.matrix.message",
//!     "properties": { "event": "message", "...": "..." }
//!   }
//! }
//! ```
//!
//! The crate never talks to vigil-watch itself: it produces the
//! envelope, and the caller decides what to do with it. That keeps
//! this crate a chat client rather than a vigil-watch component.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::event::{Event, Service};

/// The `directory` field of every envelope this crate produces.
const ORIGIN: &str = "chatwire";

/// An event, named and wrapped for a consumer outside this process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Where the event came from. Always `chatwire`.
    pub directory: String,
    pub payload: Payload,
}

/// The named event itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    /// `chat.<service>.<kind>`, e.g. `chat.matrix.message`.
    #[serde(rename = "type")]
    pub event_type: String,
    /// The event, as JSON.
    pub properties: serde_json::Value,
}

impl Envelope {
    /// Wrap an event, naming it after its service and kind.
    pub fn new(service: Service, event: &Event) -> Result<Self> {
        let properties = serde_json::to_value(event)
            .map_err(|source| Error::Send(format!("cannot serialize the event: {source}")))?;
        Ok(Self {
            directory: ORIGIN.to_owned(),
            payload: Payload {
                event_type: event_type_for(service, event),
                properties,
            },
        })
    }

    /// The envelope as a single line of JSON.
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|source| Error::Send(format!("cannot serialize the envelope: {source}")))
    }
}

/// The name an event is dispatched under.
///
/// Built from the service and the kind, so that a consumer can react
/// to `chat.matrix.message` specifically, and so that adding a service
/// never renames an existing event.
pub fn event_type_for(service: Service, event: &Event) -> String {
    format!("chat.{}.{}", service.tag(), event.kind().tag())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{
        Body, Connection, Conversation, Direction, EventKind, Message, Participant,
    };

    fn message_event() -> Event {
        Event::Message(Message {
            id: "$one".to_owned(),
            conversation: Conversation {
                id: "!room:example.org".to_owned(),
                name: Some("project".to_owned()),
                encrypted: false,
                direct: false,
            },
            sender: Participant {
                id: "@alice:example.org".to_owned(),
                name: Some("Alice".to_owned()),
            },
            direction: Direction::Incoming,
            body: Body::Text {
                text: "hello".to_owned(),
            },
            replies_to: None,
            timestamp_ms: 1_700_000_000_000,
        })
    }

    #[test]
    fn an_event_is_named_after_its_service_and_kind() {
        assert_eq!(
            event_type_for(Service::Matrix, &message_event()),
            "chat.matrix.message"
        );
    }

    #[test]
    fn the_same_event_from_another_service_gets_another_name() {
        assert_eq!(
            event_type_for(Service::RocketChat, &message_event()),
            "chat.rocketchat.message"
        );
    }

    #[test]
    fn every_kind_produces_a_distinct_name() {
        let names: std::collections::HashSet<String> = EventKind::ALL
            .iter()
            .map(|kind| format!("chat.{}.{}", Service::Matrix.tag(), kind.tag()))
            .collect();
        assert_eq!(names.len(), EventKind::ALL.len());
    }

    #[test]
    fn an_envelope_says_where_it_came_from() {
        let envelope = Envelope::new(Service::Matrix, &message_event()).expect("wraps");
        assert_eq!(envelope.directory, "chatwire");
    }

    #[test]
    fn an_envelope_carries_the_whole_event_as_properties() {
        let envelope = Envelope::new(Service::Matrix, &message_event()).expect("wraps");
        let properties = &envelope.payload.properties;
        assert_eq!(properties["event"], "message");
        assert_eq!(properties["direction"], "incoming");
        assert_eq!(properties["body"]["text"], "hello");
        assert_eq!(properties["conversation"]["id"], "!room:example.org");
        assert_eq!(properties["sender"]["id"], "@alice:example.org");
    }

    #[test]
    fn an_outgoing_message_is_marked_as_such_in_the_envelope() {
        let Event::Message(mut message) = message_event() else {
            unreachable!("built as a message");
        };
        message.direction = Direction::Outgoing;
        let envelope = Envelope::new(Service::Matrix, &Event::Message(message)).expect("wraps");
        assert_eq!(envelope.payload.properties["direction"], "outgoing");
    }

    #[test]
    fn an_envelope_serializes_to_the_expected_json_shape() {
        let envelope = Envelope::new(Service::Matrix, &Event::Connection(Connection::Restored))
            .expect("wraps");
        let json: serde_json::Value =
            serde_json::from_str(&envelope.to_json().expect("serializes")).expect("valid json");
        assert_eq!(json["directory"], "chatwire");
        assert_eq!(json["payload"]["type"], "chat.matrix.connection");
        assert_eq!(json["payload"]["properties"]["state"], "restored");
    }

    #[test]
    fn an_envelope_survives_a_round_trip() {
        let envelope = Envelope::new(Service::Matrix, &message_event()).expect("wraps");
        let text = envelope.to_json().expect("serializes");
        let parsed: Envelope = serde_json::from_str(&text).expect("parses back");
        assert_eq!(parsed, envelope);
    }

    #[test]
    fn the_event_can_be_recovered_from_the_properties() {
        let original = message_event();
        let envelope = Envelope::new(Service::Matrix, &original).expect("wraps");
        let recovered: Event =
            serde_json::from_value(envelope.payload.properties).expect("parses back");
        assert_eq!(recovered, original);
    }
}
