//! Turning what Matrix says into the shared vocabulary.
//!
//! Kept apart from the connection code so it can be tested on its own:
//! these are pure functions from Matrix types to [`crate::event`]
//! types, with no network and no state.

use matrix_sdk::ruma::events::room::message::MessageType;
use matrix_sdk::ruma::MilliSecondsSinceUnixEpoch;
use matrix_sdk::Room;

use crate::event::{Body, Conversation, Direction, Participant};

/// Describe the room an event happened in.
pub(crate) async fn conversation_of(room: &Room) -> Conversation {
    Conversation {
        id: room.room_id().to_string(),
        name: room
            .display_name()
            .await
            .ok()
            .map(|display_name| display_name.to_string()),
        encrypted: room.encryption_state().is_encrypted(),
        direct: room.is_direct().await.unwrap_or(false),
    }
}

/// Describe somebody taking part in a room.
pub(crate) async fn participant_of(room: &Room, user_id: &matrix_sdk::ruma::UserId) -> Participant {
    Participant {
        id: user_id.to_string(),
        name: room
            .get_member_no_sync(user_id)
            .await
            .ok()
            .flatten()
            .and_then(|member| member.display_name().map(str::to_owned)),
    }
}

/// Say whether the connected account is the author.
pub(crate) fn direction_of(
    own_account: Option<&matrix_sdk::ruma::UserId>,
    sender: &matrix_sdk::ruma::UserId,
) -> Direction {
    match own_account {
        Some(own) if own == sender => Direction::Outgoing,
        _ => Direction::Incoming,
    }
}

/// Turn a message content into the shared body vocabulary.
pub(crate) fn body_of(message_type: &MessageType) -> Body {
    match message_type {
        MessageType::Text(content) => Body::Text {
            text: content.body.clone(),
        },
        MessageType::Notice(content) => Body::Text {
            text: content.body.clone(),
        },
        MessageType::Emote(content) => Body::Text {
            text: content.body.clone(),
        },
        MessageType::Image(content) => Body::Media {
            media_kind: "m.image".to_owned(),
            name: Some(content.body.clone()),
        },
        MessageType::File(content) => Body::Media {
            media_kind: "m.file".to_owned(),
            name: Some(content.body.clone()),
        },
        MessageType::Audio(content) => Body::Media {
            media_kind: "m.audio".to_owned(),
            name: Some(content.body.clone()),
        },
        MessageType::Video(content) => Body::Media {
            media_kind: "m.video".to_owned(),
            name: Some(content.body.clone()),
        },
        other => Body::Media {
            media_kind: other.msgtype().to_owned(),
            name: None,
        },
    }
}

/// Convert a Matrix timestamp into milliseconds since the Unix epoch.
pub(crate) fn millis_of(timestamp: MilliSecondsSinceUnixEpoch) -> i64 {
    let millis: u64 = timestamp.0.into();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::events::room::message::{
        EmoteMessageEventContent, ImageMessageEventContent, NoticeMessageEventContent,
        TextMessageEventContent,
    };
    use matrix_sdk::ruma::events::room::MediaSource;
    use matrix_sdk::ruma::{owned_mxc_uri, user_id, UInt};

    #[test]
    fn a_text_message_keeps_its_text() {
        let message = MessageType::Text(TextMessageEventContent::plain("hello there"));
        assert_eq!(
            body_of(&message),
            Body::Text {
                text: "hello there".to_owned()
            }
        );
    }

    #[test]
    fn a_notice_is_read_as_text() {
        let message = MessageType::Notice(NoticeMessageEventContent::plain("build failed"));
        assert_eq!(
            body_of(&message),
            Body::Text {
                text: "build failed".to_owned()
            }
        );
    }

    #[test]
    fn an_emote_is_read_as_text() {
        let message = MessageType::Emote(EmoteMessageEventContent::plain("waves"));
        assert_eq!(
            body_of(&message),
            Body::Text {
                text: "waves".to_owned()
            }
        );
    }

    #[test]
    fn an_image_is_read_as_media_with_its_name() {
        let message = MessageType::Image(ImageMessageEventContent::new(
            "photo.png".to_owned(),
            MediaSource::Plain(owned_mxc_uri!("mxc://example.org/abc")),
        ));
        assert_eq!(
            body_of(&message),
            Body::Media {
                media_kind: "m.image".to_owned(),
                name: Some("photo.png".to_owned()),
            }
        );
    }

    #[test]
    fn the_account_own_message_is_outgoing() {
        let own = user_id!("@alice:example.org");
        assert_eq!(direction_of(Some(own), own), Direction::Outgoing);
    }

    #[test]
    fn somebody_else_message_is_incoming() {
        let own = user_id!("@alice:example.org");
        let other = user_id!("@bob:example.org");
        assert_eq!(direction_of(Some(own), other), Direction::Incoming);
    }

    #[test]
    fn without_a_known_account_a_message_is_incoming() {
        let other = user_id!("@alice:example.org");
        assert_eq!(direction_of(None, other), Direction::Incoming);
    }

    #[test]
    fn a_timestamp_becomes_milliseconds_since_the_epoch() {
        let timestamp = MilliSecondsSinceUnixEpoch(UInt::new(1_700_000_000_000).expect("in range"));
        assert_eq!(millis_of(timestamp), 1_700_000_000_000);
    }

    #[test]
    fn the_epoch_itself_converts_to_zero() {
        let timestamp = MilliSecondsSinceUnixEpoch(UInt::new(0).expect("in range"));
        assert_eq!(millis_of(timestamp), 0);
    }
}
