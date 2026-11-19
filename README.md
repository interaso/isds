# isds

A high-level asynchronous Rust client for Czech **ISDS / datové schránky**.

The crate supports username/password authentication, ordinary and large-volume messages, databox lookup, and credit information. It uses the canonical `datovka.gov.cz` and `datovka-test.gov.cz` services and does not require `libdatovka` or another native library.

## Features

- Explicit test or production environment selection.
- Async, cheaply cloneable `Client` backed by `reqwest` and rustls.
- One `send_message` API that automatically selects ordinary `CreateMessage` or VoDZ (`UploadAttachment` followed by `CreateBigMessage`).
- Binary and XML attachments with strict base64 handling and verified VoDZ upload hashes.
- Sent/received message pages with explicit offsets, limits, time filters, and state masks.
- Ordinary and VoDZ message download, standalone envelope download, delivery information, delivery events, and mark-read.
- Exact-ID and criteria-based databox lookup.
- Current credit and typed credit history in exact integer haléře.
- Structured transport, HTTP, SOAP, ISDS-status, and decoding errors.
- Redacted credentials and attachment content in debugging output.

## Quick start

The client requires a Tokio runtime. Construction performs no network request.

```no_run
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
    let sent = client.send_message(&message).await?;
    let delivery = client.delivery_info(&sent.id).await?;
    println!("message {} has state {:?}", sent.id, delivery.state);
    Ok(())
}
```

`Environment::Production` must be selected explicitly. The client never follows redirects, never disables server certificate validation, and sends Basic credentials only to the fixed environment endpoints.

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
| `download_message` | `MessageEnvelopeDownload` + `MessageDownload` or `BigMessageDownload` | `/DS/dx` + `/DS/dz` or `/DS/vodz` |
| `download_received_message` | `MessageDownload` | `/DS/dz` |
| `download_big_message` | `BigMessageDownload` | `/DS/vodz` |
| `download_message_envelope` | `MessageEnvelopeDownload` | `/DS/dx` |
| `delivery_info` | `GetDeliveryInfo` | `/DS/dx` |
| `mark_message_downloaded` | `MarkMessageAsDownloaded` | `/DS/dx` |

`/DS/vodz` uses the selected environment's `ws2` host and SOAP 1.2. The other operations use `ws1` and SOAP 1.1.

`VerifyMessage` is intentionally not presented as pre-send validation: the current ISDS documentation describes it as obsolete hash retrieval.

## Delivery and inbox safety

**`list_received_messages` can trigger legal delivery-by-login. It is not a harmless read-only probe.** Use it in production only when that legal effect is intended.

A safe consumer workflow is:

1. Discover the received message.
2. Download it and verify the returned message ID and content.
3. Durably persist it or hand it to a durable processor.
4. Only then call `mark_message_downloaded`, which marks the message as read and is separate from legal delivery.

Message IDs are opaque and not chronologically ordered. List offsets start at 1; the default explicit limit is 1000. Time filters apply to `dmDeliveryTime`, not submission or legal-acceptance timestamps. The crate performs no hidden auto-pagination.

Messages in states 4 or 5 cannot yet be downloaded through `MessageDownload`; ISDS can return `1222`. Delivery processing can also return `3006` to a received-list call. Applications should implement bounded, operation-aware retry policies themselves.

## Sending and ambiguous outcomes

`send_message` and `mark_message_downloaded` are never automatically retried. A timeout or connection failure during sending has an **ambiguous outcome**: ISDS may have accepted the message. Reconcile using sent-message queries before submitting the same content again.

`send_message` sums decoded attachment bytes. Totals up to and including 20 MiB use ordinary DZ by default; larger totals use VoDZ. Configure the boundary with `ClientBuilder::big_message_threshold_bytes` and the default 100 MiB decoded-attachment maximum with `ClientBuilder::max_attachment_bytes`. Messages above the configured maximum are rejected before any network request.

For VoDZ, the client uploads attachments sequentially and verifies returned SHA-256, SHA-512, SHA3-256, or SHA3-512 hashes locally before `CreateBigMessage`. Unknown future hash algorithms remain usable and are checked by ISDS during creation. Unused uploads expire after approximately two hours according to the ISDS guide.

All messages and downloads are fully buffered. Base64 increases their encoded size and memory usage. The default maximum response size is 256 MiB and is configurable with `ClientBuilder::max_response_bytes`. MTOM/XOP and streaming attachments are deferred.

`download_message` automatically checks the standalone envelope and selects ordinary or VoDZ download. It performs two service requests. When a message-list record already supplies the `vodz` flag, call `download_received_message` or `download_big_message` directly to avoid the envelope request. Both download paths decode inline `dmFile` attachments returned by ISDS.

Successful parsing, message download, and `GetDeliveryInfo` do **not** constitute local XMLDSig/CMS/CAdES verification. Signed ZFO operations are deferred.

## Errors

`Error` distinguishes invalid input, transport failures, authentication failures, unexpected HTTP statuses, SOAP faults, ISDS service statuses, oversized responses, and invalid responses.

ISDS status codes remain strings with leading zeroes. `ServiceError` exposes the code, message, and optional reference number. Databox lookup `0002` (no result) and `0003` (truncated result) are represented as ordinary `DataBoxSearchResult` outcomes rather than transport failures. Informational status `0009` sets `DataBoxSearchResult::postal_delivery_restricted` while preserving returned records.

## Live tests against the official test environment

Default tests are offline. Live tests are both `#[ignore]`d and gated by `ISDS_RUN_LIVE_TESTS=1`. Their endpoint is hardcoded to `Environment::Test`.

Set these variables through your shell or secret manager; do not commit them:

- `ISDS_TEST_SENDER_USERNAME`, `ISDS_TEST_SENDER_PASSWORD`, `ISDS_TEST_SENDER_BOX_ID`
- `ISDS_TEST_RECIPIENT1_USERNAME`, `ISDS_TEST_RECIPIENT1_PASSWORD`, `ISDS_TEST_RECIPIENT1_BOX_ID`
- `ISDS_TEST_RECIPIENT2_USERNAME`, `ISDS_TEST_RECIPIENT2_PASSWORD`, `ISDS_TEST_RECIPIENT2_BOX_ID`

Run the lifecycle serially:

```text
ISDS_RUN_LIVE_TESTS=1 cargo test --test live -- --ignored --test-threads=1
```

The lifecycle sends fresh synthetic messages to both recipients and separately exercises automatic VoDZ selection with a 20 MiB + 1 byte attachment, including download and byte comparison. Use only test accounts and non-sensitive content. It never retries a send and never targets production.

## Development checks

```text
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo doc --no-deps
```

Schema source URLs and SHA-256 hashes are recorded in [SCHEMAS.md](SCHEMAS.md). Runtime endpoints are selected independently because the official test-hosted WSDLs currently contain production `soap:address` values.

## Deferred capabilities

- Blocking API.
- Certificate, OTP, and Mobilní klíč authentication.
- Multi-recipient `CreateMultipleMessage` and partial per-recipient results.
- MTOM/XOP and streaming attachments; VoDZ currently uses buffered base64 and SOAP 1.2.
- Signed ZFO/CMS/CAdES operations and local verification.
- Administration, password changes, notifications, and archival services.
- Durable inbox workers, persistence, automatic retries, and credit top-up/payment flows.

## Official references

- [Application interface](https://datovka.gov.cz/info/cs/74.html)
- [Operating Rules and technical appendices](https://datovka.gov.cz/info/cs/80.html)
- [Developer notices](https://datovka.gov.cz/info/cs/2052.html)
- [Test environment](https://datovka.gov.cz/info/cs/95.html)
- [Static WSDL/XSD files](https://www.datovka.gov.cz/static/wsdl/v20/dm_operations.wsdl)
