//! Matrix, reached through the shared [`Client`] interface.
//!
//! The client logs into a homeserver the way any other Matrix client
//! does: it is an ordinary account with an ordinary device. Nothing is
//! installed on the server, and nothing requires administrator rights.

mod session;
mod translate;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::ruma::api::client::filter::FilterDefinition;
use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use matrix_sdk::ruma::events::reaction::ReactionEventContent;
use matrix_sdk::ruma::events::receipt::ReceiptThread;
use matrix_sdk::ruma::events::relation::Annotation;
use matrix_sdk::ruma::events::room::encrypted::OriginalSyncRoomEncryptedEvent;
use matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent;
use matrix_sdk::ruma::events::room::message::{
    OriginalRoomMessageEvent, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::ruma::events::room::redaction::OriginalSyncRoomRedactionEvent;
use matrix_sdk::ruma::events::typing::SyncTypingEvent;
use matrix_sdk::ruma::serde::Raw;
use matrix_sdk::ruma::{EventId, OwnedUserId, RoomId};
use matrix_sdk::{Client as SdkClient, LoopCtrl, Room, RoomState};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

pub use session::{has_saved_session, Credentials};

use crate::client::{Client, Events, Outgoing};
use crate::config::Config;
use crate::dedup::SeenEvents;
use crate::error::{Error, Result};
use crate::event::{
    Body, Connection, Conversation, Direction, Event, EventKind, Membership, Message, Participant,
    Service, Subscription,
};

/// How many event identifiers the deduplication window remembers.
const DEDUP_WINDOW: usize = 4096;
/// How many events may wait before the slowest subscriber loses the
/// oldest ones.
const CHANNEL_CAPACITY: usize = 512;

/// A live Matrix connection.
pub struct MatrixClient {
    inner: SdkClient,
    account: String,
    events: broadcast::Sender<Event>,
    sync_task: JoinHandle<()>,
}

/// Connect, using the saved session when there is one.
///
/// `credentials` is needed only when the store holds no session for
/// this account.
pub async fn connect(config: &Config, credentials: Option<Credentials>) -> Result<MatrixClient> {
    if config.service != Service::Matrix {
        return Err(Error::Config(format!(
            "this is the matrix backend, but the configuration asks for {}",
            config.service
        )));
    }

    let store_path = session::prepare_store(&config.account_store_path())?;
    session::check_stored_account(&store_path, &config.account)?;

    let inner = SdkClient::builder()
        .homeserver_url(&config.server)
        .sqlite_store(&store_path, None)
        .build()
        .await
        .map_err(|source| {
            Error::Login(format!(
                "cannot reach the server {}: {source}",
                config.server
            ))
        })?;

    session::establish(&inner, config, credentials, &store_path).await?;
    session::verify_account(&inner, config)?;

    let (events, _) = broadcast::channel(CHANNEL_CAPACITY);
    let start_token = baseline_token(&inner, &store_path).await?;
    register_handlers(&inner, events.clone());

    let account = inner
        .user_id()
        .ok_or_else(|| Error::Login("the client is not logged in".to_owned()))?
        .to_string();
    let device = inner
        .device_id()
        .map(|id| id.to_string())
        .unwrap_or_default();
    let _ = events.send(Event::Connection(Connection::Connected {
        account: account.clone(),
        device,
    }));

    let sync_task = spawn_sync_loop(inner.clone(), store_path, start_token, events.clone());

    Ok(MatrixClient {
        inner,
        account,
        events,
        sync_task,
    })
}

impl MatrixClient {
    /// Find a room the account is in, or say plainly that it is not.
    fn room(&self, conversation: &str) -> Result<Room> {
        let room_id = RoomId::parse(conversation).map_err(|source| {
            Error::Send(format!("{conversation:?} is not a room id: {source}"))
        })?;
        self.inner
            .get_room(&room_id)
            .ok_or_else(|| Error::NoSuchConversation(conversation.to_owned()))
    }
}

#[async_trait]
impl Client for MatrixClient {
    fn service(&self) -> Service {
        Service::Matrix
    }

    fn account(&self) -> String {
        self.account.clone()
    }

    fn device(&self) -> Option<String> {
        self.inner.device_id().map(|id| id.to_string())
    }

    fn subscribe(&self) -> Events {
        Events::new(self.events.subscribe())
    }

    fn subscribe_to(&self, subscription: Subscription) -> Events {
        let (filtered, receiver) = broadcast::channel(CHANNEL_CAPACITY);
        let mut source = self.events.subscribe();
        tokio::spawn(async move {
            while let Ok(event) = source.recv().await {
                if subscription.wants(event.kind()) && filtered.send(event).is_err() {
                    return;
                }
            }
        });
        Events::new(receiver)
    }

    async fn conversations(&self) -> Result<Vec<Conversation>> {
        let mut conversations = Vec::new();
        for room in self.inner.joined_rooms() {
            conversations.push(translate::conversation_of(&room).await);
        }
        Ok(conversations)
    }

    async fn send(&self, conversation: &str, message: Outgoing) -> Result<String> {
        let room = self.room(conversation)?;
        let mut content = if message.is_notice() {
            RoomMessageEventContent::notice_plain(message.body())
        } else {
            RoomMessageEventContent::text_plain(message.body())
        };
        if let Some(replied_to) = message.in_reply_to() {
            // A rich reply quotes the original, so the original has to
            // be fetched: its sender and thread are part of the reply.
            let original = original_message(&room, replied_to).await?;
            content = content.make_reply_to(
                &original,
                matrix_sdk::ruma::events::room::message::ForwardThread::Yes,
                matrix_sdk::ruma::events::room::message::AddMentions::No,
            );
        }
        let response = room
            .send(content)
            .await
            .map_err(|source| Error::Send(format!("cannot send to {conversation}: {source}")))?;
        Ok(response.response.event_id.to_string())
    }

    async fn edit(&self, conversation: &str, message_id: &str, text: &str) -> Result<()> {
        let room = self.room(conversation)?;
        let original = original_message(&room, message_id).await?;
        if Some(original.sender.as_ref()) != self.inner.user_id() {
            return Err(Error::Send(format!(
                "{message_id} was sent by {}, and only its author can edit it",
                original.sender
            )));
        }
        let content = RoomMessageEventContent::text_plain(text).make_replacement(&original);
        room.send(content)
            .await
            .map_err(|source| Error::Send(format!("cannot send the edit: {source}")))?;
        Ok(())
    }

    async fn remove(&self, conversation: &str, message_id: &str) -> Result<()> {
        let room = self.room(conversation)?;
        let event_id = EventId::parse(message_id).map_err(|source| {
            Error::Send(format!("{message_id:?} is not a message id: {source}"))
        })?;
        room.redact(&event_id, None, None)
            .await
            .map_err(|source| Error::Send(format!("cannot remove the message: {source}")))?;
        Ok(())
    }

    async fn react(&self, conversation: &str, message_id: &str, reaction: &str) -> Result<()> {
        let room = self.room(conversation)?;
        let event_id = EventId::parse(message_id).map_err(|source| {
            Error::Send(format!("{message_id:?} is not a message id: {source}"))
        })?;
        let content = ReactionEventContent::new(Annotation::new(event_id, reaction.to_owned()));
        room.send(content)
            .await
            .map_err(|source| Error::Send(format!("cannot react: {source}")))?;
        Ok(())
    }

    async fn typing(&self, conversation: &str, typing: bool) -> Result<()> {
        let room = self.room(conversation)?;
        room.typing_notice(typing)
            .await
            .map_err(|source| Error::Send(format!("cannot send the typing notice: {source}")))?;
        Ok(())
    }

    async fn mark_read(&self, conversation: &str, message_id: &str) -> Result<()> {
        let room = self.room(conversation)?;
        let event_id = EventId::parse(message_id).map_err(|source| {
            Error::Send(format!("{message_id:?} is not a message id: {source}"))
        })?;
        room.send_single_receipt(ReceiptType::Read, ReceiptThread::Unthreaded, event_id)
            .await
            .map_err(|source| Error::Send(format!("cannot mark as read: {source}")))?;
        Ok(())
    }

    async fn join(&self, conversation: &str) -> Result<()> {
        let room_id = RoomId::parse(conversation).map_err(|source| {
            Error::Send(format!("{conversation:?} is not a room id: {source}"))
        })?;
        self.inner
            .join_room_by_id(&room_id)
            .await
            .map_err(|source| Error::Send(format!("cannot join {conversation}: {source}")))?;
        Ok(())
    }

    async fn leave(&self, conversation: &str) -> Result<()> {
        let room = self.room(conversation)?;
        room.leave()
            .await
            .map_err(|source| Error::Send(format!("cannot leave {conversation}: {source}")))?;
        Ok(())
    }

    async fn shutdown(self: Box<Self>) {
        self.sync_task.abort();
        let _ = self.sync_task.await;
    }
}

/// Decide where the sync loop starts.
///
/// With a saved position, resume there, so what arrived while the
/// client was down is delivered. Without one, take a silent baseline
/// sync first, so the history already in the rooms is not mistaken for
/// new traffic.
async fn baseline_token(client: &SdkClient, store_path: &std::path::Path) -> Result<String> {
    let token_file = store_path.join(session::SYNC_TOKEN_FILE);
    if let Ok(saved) = std::fs::read_to_string(&token_file) {
        let saved = saved.trim().to_owned();
        if !saved.is_empty() {
            return Ok(saved);
        }
    }

    let settings = SyncSettings::default().filter(FilterDefinition::with_lazy_loading().into());
    let response = client
        .sync_once(settings)
        .await
        .map_err(|source| Error::Sync(format!("the first sync failed: {source}")))?;
    session::write_private_file(&token_file, &response.next_batch)?;
    Ok(response.next_batch)
}

/// Record an event id, and say whether it was new.
fn remember(seen: &Arc<Mutex<SeenEvents>>, id: &str) -> bool {
    match seen.lock() {
        Ok(mut seen) => seen.insert(id),
        Err(poisoned) => {
            // A duplicate is less harmful than a loss, so let it through.
            eprintln!("warning: the deduplication window was left inconsistent");
            poisoned.into_inner().insert(id)
        }
    }
}

fn register_handlers(client: &SdkClient, events: broadcast::Sender<Event>) {
    let seen = Arc::new(Mutex::new(SeenEvents::new(DEDUP_WINDOW)));

    register_messages(client, events.clone(), Arc::clone(&seen));
    register_undecryptable(client, events.clone(), Arc::clone(&seen));
    register_redactions(client, events.clone(), seen);
    register_invites(client, events.clone());
    register_typing(client, events);
}

fn register_messages(
    client: &SdkClient,
    events: broadcast::Sender<Event>,
    seen: Arc<Mutex<SeenEvents>>,
) {
    client.add_event_handler(
        move |event: OriginalSyncRoomMessageEvent, room: Room, client: SdkClient| {
            let events = events.clone();
            let seen = Arc::clone(&seen);
            async move {
                if room.state() != RoomState::Joined {
                    return;
                }
                let id = event.event_id.to_string();
                if !remember(&seen, &id) {
                    return;
                }
                let own: Option<OwnedUserId> = client.user_id().map(ToOwned::to_owned);
                let message = Message {
                    id,
                    conversation: translate::conversation_of(&room).await,
                    sender: translate::participant_of(&room, &event.sender).await,
                    direction: translate::direction_of(own.as_deref(), &event.sender),
                    body: translate::body_of(&event.content.msgtype),
                    replies_to: event
                        .content
                        .relates_to
                        .as_ref()
                        .and_then(reply_target)
                        .map(|id| id.to_string()),
                    timestamp_ms: translate::millis_of(event.origin_server_ts),
                };
                let _ = events.send(Event::Message(message));
            }
        },
    );
}

/// The message a relation replies to, when it is a reply.
fn reply_target(
    relation: &matrix_sdk::ruma::events::room::message::Relation<
        matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation,
    >,
) -> Option<&matrix_sdk::ruma::EventId> {
    match relation {
        matrix_sdk::ruma::events::room::message::Relation::Reply(in_reply_to) => {
            Some(&in_reply_to.in_reply_to.event_id)
        }
        _ => None,
    }
}

/// Fetch a message from the server, to reply to it or to edit it.
///
/// Both operations need more than the identifier the caller has: a
/// reply quotes the original's sender and thread, and an edit must
/// preserve its relation.
async fn original_message(room: &Room, message_id: &str) -> Result<OriginalRoomMessageEvent> {
    let event_id = EventId::parse(message_id)
        .map_err(|source| Error::Send(format!("{message_id:?} is not a message id: {source}")))?;
    let timeline_event = room
        .event(&event_id, None)
        .await
        .map_err(|source| Error::Send(format!("cannot read {message_id}: {source}")))?;
    timeline_event
        .raw()
        .deserialize_as_unchecked::<OriginalRoomMessageEvent>()
        .map_err(|source| Error::Send(format!("{message_id} is not a text message: {source}")))
}

fn register_undecryptable(
    client: &SdkClient,
    events: broadcast::Sender<Event>,
    seen: Arc<Mutex<SeenEvents>>,
) {
    client.add_event_handler(
        move |raw: Raw<OriginalSyncRoomEncryptedEvent>, room: Room| {
            let events = events.clone();
            let seen = Arc::clone(&seen);
            async move {
                if room.state() != RoomState::Joined {
                    return;
                }
                let Ok(event) = raw.deserialize() else {
                    eprintln!("warning: unreadable encrypted event");
                    return;
                };
                let id = event.event_id.to_string();
                if !remember(&seen, &id) {
                    return;
                }
                let message = Message {
                    id,
                    conversation: translate::conversation_of(&room).await,
                    sender: translate::participant_of(&room, &event.sender).await,
                    direction: Direction::Incoming,
                    body: Body::Undecryptable {
                        reason: "this device holds no key for that message".to_owned(),
                    },
                    replies_to: None,
                    timestamp_ms: translate::millis_of(event.origin_server_ts),
                };
                let _ = events.send(Event::Message(message));
            }
        },
    );
}

fn register_redactions(
    client: &SdkClient,
    events: broadcast::Sender<Event>,
    seen: Arc<Mutex<SeenEvents>>,
) {
    client.add_event_handler(move |event: OriginalSyncRoomRedactionEvent, room: Room| {
        let events = events.clone();
        let seen = Arc::clone(&seen);
        async move {
            if room.state() != RoomState::Joined {
                return;
            }
            let id = event.event_id.to_string();
            if !remember(&seen, &id) {
                return;
            }
            let Some(removed) = event.redacts.clone() else {
                return;
            };
            let _ = events.send(Event::MessageRemoved {
                id: removed.to_string(),
                conversation: translate::conversation_of(&room).await,
                removed_by: translate::participant_of(&room, &event.sender).await,
                timestamp_ms: translate::millis_of(event.origin_server_ts),
            });
        }
    });
}

fn register_invites(client: &SdkClient, events: broadcast::Sender<Event>) {
    client.add_event_handler(
        move |event: StrippedRoomMemberEvent, room: Room, client: SdkClient| {
            let events = events.clone();
            async move {
                let own = client.user_id().map(ToOwned::to_owned);
                let Some(own) = own else { return };
                if event.state_key != own {
                    return;
                }
                if room.state() != RoomState::Invited {
                    return;
                }
                let _ = events.send(Event::MembershipChanged {
                    conversation: translate::conversation_of(&room).await,
                    participant: Participant {
                        id: own.to_string(),
                        name: None,
                    },
                    membership: Membership::Invited,
                    timestamp_ms: now_millis(),
                });
            }
        },
    );
}

fn register_typing(client: &SdkClient, events: broadcast::Sender<Event>) {
    client.add_event_handler(move |event: SyncTypingEvent, room: Room| {
        let events = events.clone();
        async move {
            if room.state() != RoomState::Joined {
                return;
            }
            let mut participants = Vec::new();
            for user_id in &event.content.user_ids {
                participants.push(translate::participant_of(&room, user_id).await);
            }
            let _ = events.send(Event::Typing {
                conversation: translate::conversation_of(&room).await,
                participants,
            });
        }
    });
}

/// Milliseconds since the Unix epoch, for events the service does not
/// date itself.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Run the sync loop, saving the position after every batch and coming
/// back after a failure.
fn spawn_sync_loop(
    client: SdkClient,
    store_path: PathBuf,
    start_token: String,
    events: broadcast::Sender<Event>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let token_file = store_path.join(session::SYNC_TOKEN_FILE);
        let mut token = start_token;
        let mut backoff_seconds = 1;

        loop {
            let settings = SyncSettings::default().token(token.clone());
            let token_file_for_callback = token_file.clone();
            let outcome = client
                .sync_with_result_callback(settings, |result| {
                    let token_file = token_file_for_callback.clone();
                    async move {
                        let response = result?;
                        if let Err(error) =
                            session::write_private_file(&token_file, &response.next_batch)
                        {
                            eprintln!("warning: cannot save the sync position: {error}");
                        }
                        Ok(LoopCtrl::Continue)
                    }
                })
                .await;

            match outcome {
                Ok(()) => return,
                Err(error) => {
                    let _ = events.send(Event::Connection(Connection::Lost {
                        reason: error.to_string(),
                    }));
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_seconds)).await;
                    backoff_seconds = (backoff_seconds * 2).min(60);
                    if let Ok(saved) = std::fs::read_to_string(&token_file) {
                        let saved = saved.trim().to_owned();
                        if !saved.is_empty() {
                            token = saved;
                        }
                    }
                    let _ = events.send(Event::Connection(Connection::Restored));
                }
            }
        }
    })
}

/// Kinds this backend can currently report.
///
/// Stated explicitly so a caller can tell an event that will never
/// arrive from one that simply has not happened yet.
pub fn reported_kinds() -> Vec<EventKind> {
    vec![
        EventKind::Message,
        EventKind::MessageRemoved,
        EventKind::MembershipChanged,
        EventKind::Typing,
        EventKind::Connection,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[tokio::test]
    async fn connecting_with_a_rocket_chat_configuration_is_refused() {
        let config = Config::from_yaml(indoc! {"
            service: rocket-chat
            server: https://chat.example.org
            account: someone
        "})
        .expect("the configuration is valid");
        let error = connect(&config, None)
            .await
            .err()
            .expect("the matrix backend must refuse another service");
        assert!(
            error.to_string().contains("rocketchat"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn the_backend_says_which_kinds_it_reports() {
        let kinds = reported_kinds();
        assert!(kinds.contains(&EventKind::Message));
        assert!(kinds.contains(&EventKind::Connection));
    }

    #[test]
    fn reported_kinds_are_listed_once_each() {
        let kinds = reported_kinds();
        let mut unique = kinds.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), kinds.len());
    }
}
