# isds

[![CI](https://github.com/interaso/isds/actions/workflows/ci.yml/badge.svg)](https://github.com/interaso/isds/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/isds.svg)](https://crates.io/crates/isds)
[![docs.rs](https://docs.rs/isds/badge.svg)](https://docs.rs/isds)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)

A high-level asynchronous Rust client for Czech **ISDS / datové schránky**.

The crate supports username/password authentication, ordinary and large-volume messages, databox lookup, and credit information. It uses the canonical `datovka.gov.cz` and `datovka-test.gov.cz` services and does not require `libdatovka`. The current `reqwest`/rustls backend uses `aws-lc-rs`/`aws-lc-sys`, which builds bundled native code and may require a supported C/CMake toolchain.

## Features

- Explicit test or production environment selection.
- Async, cheaply cloneable `Client` backed by `reqwest` and rustls.
- One `send_message` API that automatically selects ordinary `CreateMessage` or large-message operations (`UploadAttachment` followed by `CreateBigMessage`).
- Binary and XML attachments with strict base64 handling and verified large-message upload hashes.
- Sent/received message pages with explicit offsets, limits, time filters, and state masks.
- Standard and large-message download, standalone envelope download, delivery information, delivery events, and mark-read.
- Exact-ID and criteria-based databox lookup, with descriptive type variants and raw-code fallback for specialized or future types.
- Current credit and typed credit history in exact integer haléře.
- Structured transport, HTTP, SOAP, ISDS-status, and decoding errors.
- Redacted credentials and attachment content in debugging output.
- Optional `serde` feature for serializing `MessageId` and `DataBoxId` as strings and validating them during deserialization.
- Optional `tracing` feature for privacy-safe request spans and response metadata.

## Installation

```sh
cargo add isds
```

The crate is pre-1.0, so API changes may occur between minor releases. Optional ID serialization is enabled with `cargo add isds --features serde`.

## Quick start

The client requires a Tokio runtime. Construction performs no network request.

```rust,no_run
use bytes::Bytes;
use isds::{
    Attachment, Client, Credentials, DataBoxId, Environment, OutgoingMessage,
};

async fn example() -> isds::Result<()> {
    let client = Client::builder(Environment::Test)
        .credentials(Credentials::new(
            "username-from-your-secret-store",
            "password-from-your-secret-store",
        ))
        .build()?;

    let recipient = DataBoxId::new("n94k37v")?;
    let state = client.check_data_box(&recipient).await?;
    assert!(state.is_active());

    let message = OutgoingMessage::new(recipient.clone(), "Synthetic example")
        .with_attachment(Attachment::main(
            "document.txt",
            mime::TEXT_PLAIN,
            Bytes::from_static("Hello, ISDS — žluťoučký".as_bytes()),
        ));

    // Acceptance by ISDS is not the same as delivery to the recipient.
    let sent = client.send_message(&message, None).await?;
    let delivery = client.delivery_info(&sent.id).await?;
    println!("message {} has state {:?}", sent.id, delivery.state);
    Ok(())
}
```

`Environment::Production` must be selected explicitly. The client never follows redirects, never disables server certificate validation, and sends Basic credentials only to the fixed environment endpoints.

`MessageId` and `DataBoxId` support `new`, `FromStr`, and `TryFrom` for borrowed or owned strings. String input is always validated. An existing ID can be consumed into its raw string with `String::from(id)` or `id.into()`; use `as_str()` when borrowing is sufficient.

## Examples

The [`examples`](examples/) directory contains compile-checked consumer examples. Network-enabled examples use these environment variables; supply credentials through your shell or secret manager and never commit their values.

| Variable | Required | Purpose |
| --- | --- | --- |
| `ISDS_EXAMPLE_USERNAME` | For network examples | ISDS account username |
| `ISDS_EXAMPLE_PASSWORD` | For network examples | ISDS account password |
| `ISDS_EXAMPLE_ENVIRONMENT` | No | `test` by default; set to `production` only for intentional production use |

| Example | Demonstrates | Command |
| --- | --- | --- |
| [lookup.rs](examples/lookup.rs) | State checks, exact lookup, typed databox types, criteria search | `cargo run --example lookup -- <databox-id> [firm-name]` |
| [credit.rs](examples/credit.rs) | Owner information, current credit, seven-day history | `cargo run --example credit` |
| [send_message.rs](examples/send_message.rs) | Files, metadata, automatic ordinary/large-message sending | `cargo run --example send_message -- <recipient-id> <file> [mime-type] [sender-reference]` |
| [download_message.rs](examples/download_message.rs) | Automatic download, attachment metadata, delivery events | `cargo run --example download_message -- <message-id>` |
| [inbox_worker.rs](examples/inbox_worker.rs) | Delivery-by-login, durable idempotent persistence, mark-read | Integration template; does not make network requests when run |

Running a network example contacts the selected ISDS environment. Sending creates a real data message, and received-message listing can trigger legal delivery-by-login. Use test accounts and non-sensitive content. The download example deliberately leaves mark-read commented out until durable storage is in place.

## Optional serde support

Enable the `serde` feature when embedding IDs in serde-enabled application models:

```toml
[dependencies]
isds = { version = "0.1.0", features = ["serde"] }
```

`MessageId` and `DataBoxId` serialize as strings. Deserialization calls the same validators as their `new` constructors, including the databox Luhn32 check, so invalid IDs cannot be constructed through serde. The feature is disabled by default and does not add serde implementations to unrelated models.

## Optional tracing

Enable the `tracing` feature to instrument HTTP operations. The application configures its own subscriber and filters; the crate never installs a global subscriber.

```toml
[dependencies]
isds = { version = "0.1.0", features = ["tracing"] }
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

This example honors `RUST_LOG` and otherwise enables debug instrumentation only for `isds`:

```rust,no_run
use tracing_subscriber::EnvFilter;

fn main() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("isds=debug"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
```

Each HTTP request has a debug-level `isds.request` span containing the ISDS operation name, fixed service endpoint, and environment. After the response body is read, a debug event records its HTTP status and buffered byte count. Request-transport and response-reading failures also emit debug events. Use `RUST_LOG=isds=debug` to enable these records or a more restrictive filter to silence them.

Credentials, authorization headers, databox and message IDs, XML bodies, and attachment contents are never included at any level. Disabling the default-off feature removes this crate's instrumentation and direct `tracing` dependency. HTTP/TLS dependencies may still emit independent `log` or `tracing` records under their own targets; in particular, `reqwest` uses `log`.

## Supported operations

| Rust method | ISDS operation | Endpoint |
| --- | --- | --- |
| `owner_info` | `GetOwnerInfoFromLogin2` | `/DS/DsManage` |
| `check_data_box` | `CheckDataBox` | `/DS/df` |
| `find_data_boxes`, `find_data_box` | `FindDataBox2` | `/DS/df` |
| `credit_info` | `DataBoxCreditInfo` | `/DS/df` |
| `send_message` | `CreateMessage` or `UploadAttachment` + `CreateBigMessage` | `/DS/dz` or `/DS/vodz` |
| `list_sent_messages` | `GetListOfSentMessages` | `/DS/dx` |
| `list_received_messages` | `GetListOfReceivedMessages` | `/DS/dx` |
| `download_message` | `MessageDownload` or `BigMessageDownload`, with optional envelope detection | `/DS/dz` or `/DS/vodz` |
| `download_message_envelope` | `MessageEnvelopeDownload` | `/DS/dx` |
| `delivery_info` | `GetDeliveryInfo` | `/DS/dx` |
| `mark_message_downloaded` | `MarkMessageAsDownloaded` | `/DS/dx` |

`/DS/vodz` uses the selected environment's `ws2` host and SOAP 1.2. The other operations use `ws1` and SOAP 1.1.

`VerifyMessage` is intentionally not presented as pre-send validation: the current ISDS documentation describes it as obsolete hash retrieval.

Pure response models are `#[non_exhaustive]`, allowing new ISDS response fields without breaking downstream code. Their public fields remain directly readable. Shared request/response models such as `DataBox` and `MessageMetadata` remain constructible for queries and outgoing messages.

## Delivery and inbox safety

**`list_received_messages` can trigger legal delivery-by-login. It is not a harmless read-only probe.** Use it in production only when that legal effect is intended.

A safe consumer workflow is:

1. Discover the received message.
2. Download it. `Client` validates that the returned message ID matches the requested ID before returning the message.
3. Verify the message content as required by the application.
4. Durably persist it or hand it to a durable processor.
5. Only then call `mark_message_downloaded`, which marks the message as read and is separate from legal delivery.

Message IDs are opaque. `MessageId`'s `Ord` implementation provides deterministic lexicographic ordering for container use only; it never represents chronology or pagination. List offsets start at 1; the default explicit limit is 1000, and the crate performs no hidden auto-pagination.

Public date/time types use `jiff` 0.2 (currently 0.2.46), so downstream `jiff` 0.2 values can be supplied without conversion. Credit-history bounds are Czech-local civil dates with an exclusive upper bound; the credit example explicitly uses `Europe/Prague` and tomorrow to include today. Message-list time filters apply to `dmDeliveryTime`, not submission or legal-acceptance timestamps.

States 4 and 5 are represented by `MessageState::PlacedInRecipientBox` and `MessageState::DeliveredByFiction`, without requiring numeric comparisons. Messages in these states cannot yet be downloaded through `MessageDownload`; ISDS can return `1222`. Delivery processing can also return `3006` to a received-list call. Applications should implement bounded, operation-aware retry policies themselves.

## Message modes

`send_message` and `download_message` both take `Option<MessageMode>`:

| Mode | Sending | Downloading |
| --- | --- | --- |
| `None` | Select standard or large automatically from the decoded-attachment threshold | Read the envelope, then select the appropriate download |
| `Some(MessageMode::Standard)` | Force `CreateMessage` | Call `MessageDownload` directly |
| `Some(MessageMode::Large)` | Force sequential uploads and `CreateBigMessage` | Call `BigMessageDownload` directly |

Automatic download performs two service requests. Forced standard or large download performs one, which is useful when `MessageRecord::mode` is already known. `Message`, `MessageRecord`, and `MessageEnvelopeDetails` also expose `mode: Option<MessageMode>`; `None` means ISDS did not supply the mode.

## Sending and ambiguous outcomes

`send_message` and `mark_message_downloaded` are never automatically retried. A timeout or connection failure during sending has an **ambiguous outcome**: ISDS may have accepted the message. Reconcile using sent-message queries before submitting the same content again.

`send_message` sums decoded attachment bytes. Totals up to and including 20 MiB use `CreateMessage` by default; larger totals use `UploadAttachment` followed by `CreateBigMessage`. Configure the boundary with `ClientBuilder::large_message_threshold_bytes` and the default 100 MiB decoded-attachment maximum with `ClientBuilder::max_attachment_bytes`. Messages above the configured maximum are rejected before any network request.

The default total request timeout is 30 seconds. For large uploads on slow connections, set a longer timeout with `ClientBuilder::timeout`.

For large messages, the client uploads attachments sequentially and verifies returned SHA-256, SHA-512, SHA3-256, or SHA3-512 hashes locally before `CreateBigMessage`. Unknown future hash algorithms remain usable and are checked by ISDS during creation. Unused uploads expire after approximately two hours according to the ISDS guide.

All messages and downloads are fully buffered. Base64 increases their encoded size and memory usage. The default maximum response size is 256 MiB and is configurable with `ClientBuilder::max_response_bytes`. MTOM/XOP and streaming attachments are deferred.

Both standard and large downloads decode the inline `dmFile` attachments returned by ISDS.

Successful parsing, message download, and `GetDeliveryInfo` do **not** constitute local XMLDSig/CMS/CAdES verification. Signed ZFO operations are deferred.

## Errors

`Error` distinguishes invalid input, transport failures, authentication failures, unexpected HTTP statuses, SOAP faults, ISDS service statuses, oversized responses, and invalid responses.

`ServiceError::kind` classifies `1222` as `ServiceErrorKind::MessageNotDownloadable` and `3006` as `ServiceErrorKind::DeliveryInProgress`; other statuses return `ServiceErrorKind::Other`. This allows retry decisions without comparing raw strings. The exact code remains a string with leading zeroes through `ServiceError::code`, which also preserves unknown and future statuses alongside the server message and optional reference number.

Databox lookup `0002` (no result) and `0003` (truncated result) are represented as ordinary `DataBoxSearchResult` outcomes rather than transport failures. Informational status `0009` sets `DataBoxSearchResult::postal_delivery_restricted` while preserving returned records.

## Live tests against the official test environment

Default tests are offline. Live tests are `#[ignore]`d, and their endpoint is hardcoded to `Environment::Test`. Set all of these variables through your shell or secret manager; do not commit their values.

| Variable | Purpose |
| --- | --- |
| `ISDS_RUN_LIVE_TESTS` | Explicit opt-in; must be exactly `1` |
| `ISDS_TEST_SENDER_USERNAME` | Sender account username |
| `ISDS_TEST_SENDER_PASSWORD` | Sender account password |
| `ISDS_TEST_SENDER_BOX_ID` | Sender databox ID |
| `ISDS_TEST_RECIPIENT1_USERNAME` | First recipient account username |
| `ISDS_TEST_RECIPIENT1_PASSWORD` | First recipient account password |
| `ISDS_TEST_RECIPIENT1_BOX_ID` | First recipient databox ID |
| `ISDS_TEST_RECIPIENT2_USERNAME` | Second recipient account username |
| `ISDS_TEST_RECIPIENT2_PASSWORD` | Second recipient account password |
| `ISDS_TEST_RECIPIENT2_BOX_ID` | Second recipient databox ID |

Run the lifecycle serially:

```sh
ISDS_RUN_LIVE_TESTS=1 cargo test --test live -- --ignored --test-threads=1
```

The lifecycle sends fresh synthetic messages to both recipients and separately exercises automatic large-message selection with a 20 MiB + 1 byte attachment, including download and byte comparison. Use only test accounts and non-sensitive content. It never retries a send and never targets production.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
cargo test --all-features --locked
cargo check --examples --locked
RUSTDOCFLAGS="-D warnings" cargo doc --all-features --no-deps --locked
cargo package --locked --allow-dirty
```

Schema source URLs and SHA-256 hashes are recorded in [SCHEMAS.md](SCHEMAS.md). Runtime endpoints are selected independently because the official test-hosted WSDLs currently contain production `soap:address` values.

See [CONTRIBUTING.md](CONTRIBUTING.md#changelog-and-releases) for development, changelog, and release guidelines, and [CHANGELOG.md](CHANGELOG.md) for release history.

## Deferred capabilities

- Blocking API.
- Certificate, OTP, and Mobilní klíč authentication.
- Multi-recipient `CreateMultipleMessage` and partial per-recipient results.
- MTOM/XOP and streaming attachments; large-message operations currently use buffered base64 and SOAP 1.2.
- Signed ZFO/CMS/CAdES operations and local verification.
- Administration, password changes, notifications, and archival services.
- Durable inbox workers, persistence, automatic retries, and credit top-up/payment flows.

## Official references

- [Application interface](https://datovka.gov.cz/info/cs/74.html)
- [Operating Rules and technical appendices](https://datovka.gov.cz/info/cs/80.html)
- [Developer notices](https://datovka.gov.cz/info/cs/2052.html)
- [Test environment](https://datovka.gov.cz/info/cs/95.html)
- [Static WSDL/XSD files](https://www.datovka.gov.cz/static/wsdl/v20/dm_operations.wsdl)

## License

Licensed under the MIT License ([LICENSE](./LICENSE) or <https://opensource.org/licenses/MIT>).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate is licensed under the MIT License, without additional terms or conditions.
