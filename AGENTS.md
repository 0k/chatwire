# chatwire

A library crate: one client interface over several instant messaging
services. Not a binary, not a daemon.

## What belongs here, and what does not

`chatwire` connects to a chat service as an ordinary account, reports
what happens in a service-independent vocabulary, and writes back.

It does NOT decide anything. No policy about what is worth reacting
to, no LLM, no scheduling, no auto-reply. Those belong to the programs
using it. A change that teaches this crate to *judge* an event is
almost certainly in the wrong repository.

## The vocabulary is the contract

`src/event.rs` defines what every service is translated into. A
backend that cannot express something reports `Event::Unsupported`
rather than inventing a shape, and an operation a service lacks
returns `Error::Unsupported` rather than pretending to succeed.

Before adding a variant, check whether an existing one already covers
the case. Adding a service must never rename or reshape an existing
event: consumers dispatch on `chat.<service>.<kind>`.

## Both directions are events

Messages from the connected account are delivered, marked
`Direction::Outgoing`. Never filter them out by default. Knowing that
the account already answered is what stops a consumer answering twice.

## Backends

One module per service under `src/`, implementing `Client`. Keep the
pure translation (service types to vocabulary) in its own file, as
`src/matrix/translate.rs` does, so it can be tested without a network.

Backends are feature-gated. The vocabulary must compile with
`--no-default-features`.

## Secrets

Passwords are never stored in configuration, never logged, and never
put in a command line. `Credentials` has a hand-written `Debug` that
hides the password; keep it that way.

Store directories are created `0700` and session files `0600`.

## Testing

Test names state the behaviour, and the body proves that one thing.
Translation and envelope tests need no network and must stay that way.

Run `cargo fmt`, `cargo clippy --all-targets --all-features`, and
`cargo test` before reporting completion. Live verification against a
real homeserver is a supplement to the test suite, not a replacement.
