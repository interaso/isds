use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, EXPECT};
use secrecy::{ExposeSecret, SecretString};

use crate::error::{Error, Result};
use crate::model::{
    CreditInfo, CreditQuery, DataBox, DataBoxId, DataBoxSearchResult, DataBoxState, DeliveryInfo,
    Message, MessageEnvelopeDetails, MessageId, MessageListQuery, MessageMode, MessagePage,
    OutgoingMessage, SentMessage,
};
use crate::wire::{credit, databox, large_message, message, message_download, message_info, soap};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_LARGE_MESSAGE_THRESHOLD_BYTES: u64 = 20 * 1024 * 1024;
const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// ISDS deployment to which a client sends requests.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Environment {
    /// The public test deployment at `datovka-test.gov.cz`.
    Test,
    /// The production deployment at `datovka.gov.cz`.
    Production,
}

impl Environment {
    fn basic_base_url(self) -> &'static str {
        match self {
            Self::Test => "https://ws1.datovka-test.gov.cz",
            Self::Production => "https://ws1.datovka.gov.cz",
        }
    }

    fn large_message_base_url(self) -> &'static str {
        match self {
            Self::Test => "https://ws2.datovka-test.gov.cz",
            Self::Production => "https://ws2.datovka.gov.cz",
        }
    }
}

/// Username and password used for ISDS HTTP Basic authentication.
///
/// The username and password are redacted from `Debug` output. Basic credentials
/// are sent on every request; this authentication method does not establish a cookie session.
#[derive(Clone)]
pub struct Credentials {
    username: String,
    password: SecretString,
}

impl Credentials {
    /// Creates credentials from an ISDS username and password.
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: SecretString::new(password.into().into_boxed_str()),
        }
    }

    /// Returns the ISDS username.
    pub fn username(&self) -> &str {
        &self.username
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

/// A concurrent, asynchronous ISDS client.
///
/// Cloning this value is cheap. All clones share the same HTTP connection pool,
/// credentials, endpoint selection, and limits.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http: reqwest::Client,
    credentials: Credentials,
    base_url: String,
    custom_base_url: bool,
    environment: Environment,
    max_response_bytes: usize,
    large_message_threshold_bytes: u64,
    max_attachment_bytes: u64,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("environment", &self.inner.environment)
            .field("base_url", &self.inner.base_url)
            .field("max_response_bytes", &self.inner.max_response_bytes)
            .field(
                "large_message_threshold_bytes",
                &self.inner.large_message_threshold_bytes,
            )
            .field("max_attachment_bytes", &self.inner.max_attachment_bytes)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

/// Builder for an asynchronous ISDS client.
pub struct ClientBuilder {
    environment: Environment,
    credentials: Option<Credentials>,
    connect_timeout: Duration,
    timeout: Duration,
    max_response_bytes: usize,
    large_message_threshold_bytes: u64,
    max_attachment_bytes: u64,
    #[cfg(test)]
    test_base_url: Option<String>,
}

impl Client {
    /// Starts building a client for an explicitly selected environment.
    pub fn builder(environment: Environment) -> ClientBuilder {
        ClientBuilder {
            environment,
            credentials: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            timeout: DEFAULT_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            large_message_threshold_bytes: DEFAULT_LARGE_MESSAGE_THRESHOLD_BYTES,
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            #[cfg(test)]
            test_base_url: None,
        }
    }

    /// Returns the environment selected during construction.
    pub fn environment(&self) -> Environment {
        self.inner.environment
    }

    /// Checks the state of a databox without retrieving its owner's personal details.
    pub async fn check_data_box(&self, id: &DataBoxId) -> Result<DataBoxState> {
        let request = databox::check_data_box_request(id)?;
        self.execute(
            Service::DataBoxSearch,
            request,
            "CheckDataBoxResponse",
            databox::decode_check_data_box,
        )
        .await
    }

    /// Returns information about the databox authenticated by this client.
    pub async fn owner_info(&self) -> Result<DataBox> {
        self.execute(
            Service::DataBoxAccess,
            databox::owner_info_request()?,
            "GetOwnerInfoFromLogin2Response",
            databox::decode_owner_info,
        )
        .await
    }

    /// Searches databoxes using exact-ID or owner/address filters.
    ///
    /// A no-result response is returned as an empty [`DataBoxSearchResult`].
    /// Truncated results have [`DataBoxSearchResult::truncated`] set to `true`.
    pub async fn find_data_boxes(&self, query: &DataBox) -> Result<DataBoxSearchResult> {
        self.execute(
            Service::DataBoxSearch,
            databox::find_data_boxes_request(query)?,
            "FindDataBox2Response",
            databox::decode_find_data_boxes,
        )
        .await
    }

    /// Looks up at most one databox by its exact identifier.
    ///
    /// Callers that need [`DataBoxSearchResult::postal_delivery_restricted`] should use
    /// [`Client::find_data_boxes`] instead.
    pub async fn find_data_box(&self, id: &DataBoxId) -> Result<Option<DataBox>> {
        let mut result = self.find_data_boxes(&DataBox::by_id(id.clone())).await?;
        if result.truncated {
            return Err(Error::invalid_response(
                "an exact databox ID lookup returned a truncated result",
            ));
        }
        if result.boxes.len() > 1 {
            return Err(Error::invalid_response(
                "an exact databox ID lookup returned multiple records",
            ));
        }
        Ok(result.boxes.pop())
    }

    /// Returns current credit or credit history for an accessible databox.
    ///
    /// Ordinary users may query only their own databox. Credit values are exact
    /// integer haléře. History intervals are limited to 12 months by ISDS.
    pub async fn credit_info(&self, query: &CreditQuery) -> Result<CreditInfo> {
        self.execute(
            Service::DataBoxSearch,
            credit::credit_info_request(query)?,
            "DataBoxCreditInfoResponse",
            credit::decode_credit_info,
        )
        .await
    }

    /// Sends one message using automatic, standard, or large-message mode.
    ///
    /// `None` selects large-message transport when decoded attachments exceed the
    /// configured threshold (20 MiB by default); equality remains standard.
    /// `Some(MessageMode::Standard)` forces `CreateMessage`, while
    /// `Some(MessageMode::Large)` forces sequential uploads followed by
    /// `CreateBigMessage`, regardless of the threshold.
    ///
    /// Success confirms acceptance by ISDS, not delivery. No upload or creation
    /// operation is retried automatically. A transport failure during final
    /// creation has an ambiguous outcome: reconcile through sent-message queries
    /// before submitting the same content again.
    pub async fn send_message(
        &self,
        message: &OutgoingMessage,
        mode: Option<MessageMode>,
    ) -> Result<SentMessage> {
        message::validate_outgoing(message)?;
        let attachment_bytes = attachment_bytes(message)?;
        if attachment_bytes > self.inner.max_attachment_bytes {
            return Err(Error::invalid_input(
                "attachments",
                format!(
                    "total attachment content exceeds the configured {}-byte maximum",
                    self.inner.max_attachment_bytes
                ),
            ));
        }
        if use_large_message(
            attachment_bytes,
            self.inner.large_message_threshold_bytes,
            mode,
        ) {
            self.send_large_message(message).await
        } else {
            self.execute(
                Service::MessageOperations,
                message::create_message_request(message)?,
                "CreateMessageResponse",
                message::decode_create_message,
            )
            .await
        }
    }

    async fn send_large_message(&self, message: &OutgoingMessage) -> Result<SentMessage> {
        let mut uploaded = Vec::with_capacity(message.attachments.len());
        for attachment in &message.attachments {
            let result = self
                .execute(
                    Service::LargeMessageUpload,
                    large_message::upload_attachment_request(attachment)?,
                    "UploadAttachmentResponse",
                    |response| large_message::decode_upload_attachment(response, attachment),
                )
                .await?;
            uploaded.push(result);
        }

        self.execute(
            Service::LargeMessage,
            large_message::create_large_message_request(message, &uploaded)?,
            "CreateBigMessageResponse",
            large_message::decode_create_large_message,
        )
        .await
    }

    /// Lists one explicitly requested page of sent messages.
    ///
    /// Time bounds filter `dmDeliveryTime`. Offsets are one-based; no automatic
    /// pagination or hidden additional requests are performed.
    pub async fn list_sent_messages(&self, query: &MessageListQuery) -> Result<MessagePage> {
        self.execute(
            Service::MessageInfo,
            message_info::list_request(message_info::MessageListKind::Sent, query)?,
            "GetListOfSentMessagesResponse",
            |response| message_info::decode_list(response, query),
        )
        .await
    }

    /// Lists one explicitly requested page of received messages.
    ///
    /// # Legal side effect
    ///
    /// ISDS documents that this operation can trigger **legal delivery-by-login**
    /// for messages the authenticated user may read. It is not a harmless probe.
    /// Do not use it as a background production poll without intending that effect.
    pub async fn list_received_messages(&self, query: &MessageListQuery) -> Result<MessagePage> {
        self.execute(
            Service::MessageInfo,
            message_info::list_request(message_info::MessageListKind::Received, query)?,
            "GetListOfReceivedMessagesResponse",
            |response| message_info::decode_list(response, query),
        )
        .await
    }

    /// Downloads a received message using automatic, standard, or large-message mode.
    ///
    /// `None` performs `MessageEnvelopeDownload` followed by either
    /// `MessageDownload` or `BigMessageDownload`, making two service requests.
    /// `Some(MessageMode::Standard)` or `Some(MessageMode::Large)` forces the
    /// corresponding download directly and avoids the envelope request.
    ///
    /// Messages in states 4 or 5 are not yet downloadable; ISDS can return status
    /// `1222`. A received-message list performs the separate delivery-by-login
    /// step. No local cryptographic signature verification is performed.
    pub async fn download_message(
        &self,
        id: &MessageId,
        mode: Option<MessageMode>,
    ) -> Result<Message> {
        match mode {
            Some(MessageMode::Standard) => self.download_standard_message(id).await,
            Some(MessageMode::Large) => self.download_large_message(id).await,
            None => {
                let envelope = self.download_message_envelope(id).await?;
                if envelope.mode == Some(MessageMode::Large) {
                    self.download_large_message(id).await
                } else {
                    self.download_standard_message(id).await
                }
            }
        }
    }

    async fn download_standard_message(&self, id: &MessageId) -> Result<Message> {
        let message = self
            .execute(
                Service::MessageOperations,
                message_download::download_request(id)?,
                "MessageDownloadResponse",
                message_download::decode_download,
            )
            .await?;
        ensure_message_id(id, &message.envelope.id)?;
        Ok(message)
    }

    async fn download_large_message(&self, id: &MessageId) -> Result<Message> {
        let message = self
            .execute(
                Service::LargeMessage,
                large_message::large_message_download_request(id)?,
                "BigMessageDownloadResponse",
                large_message::decode_large_message_download,
            )
            .await?;
        ensure_message_id(id, &message.envelope.id)?;
        Ok(message)
    }

    /// Downloads a received-message envelope without attachment content.
    ///
    /// The result identifies ordinary versus large-volume messages and includes the
    /// attachment count when supplied by ISDS. It also works for erased messages.
    pub async fn download_message_envelope(
        &self,
        id: &MessageId,
    ) -> Result<MessageEnvelopeDetails> {
        let envelope = self
            .execute(
                Service::MessageInfo,
                message_info::envelope_download_request(id)?,
                "MessageEnvelopeDownloadResponse",
                message_info::decode_envelope_download,
            )
            .await?;
        ensure_message_id(id, &envelope.envelope.id)?;
        Ok(envelope)
    }

    /// Retrieves unsigned delivery information and ordered delivery events.
    ///
    /// This operation does not perform local signature or legal-validity verification.
    pub async fn delivery_info(&self, id: &MessageId) -> Result<DeliveryInfo> {
        let delivery = self
            .execute(
                Service::MessageInfo,
                message_info::delivery_info_request(id)?,
                "GetDeliveryInfoResponse",
                message_info::decode_delivery_info,
            )
            .await?;
        ensure_message_id(id, &delivery.envelope.id)?;
        Ok(delivery)
    }

    /// Marks a received message as read (state 7) using `MarkMessageAsDownloaded`.
    ///
    /// This does not cause legal delivery. Download and durably persist or hand off
    /// the message before calling this method. The operation is never retried
    /// automatically.
    pub async fn mark_message_downloaded(&self, id: &MessageId) -> Result<()> {
        self.execute(
            Service::MessageInfo,
            message_info::mark_downloaded_request(id)?,
            "MarkMessageAsDownloadedResponse",
            message_info::decode_mark_downloaded,
        )
        .await
    }

    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(
            name = "isds.request",
            level = "debug",
            skip_all,
            fields(
                operation = response_operation
                    .strip_suffix("Response")
                    .unwrap_or(response_operation),
                endpoint = service.path(),
                environment = ?self.inner.environment,
            )
        )
    )]
    pub(crate) async fn execute<T>(
        &self,
        service: Service,
        request: Vec<u8>,
        response_operation: &str,
        decode_response: impl FnOnce(roxmltree::Node<'_, '_>) -> Result<T>,
    ) -> Result<T> {
        let base_url = if matches!(service, Service::LargeMessage | Service::LargeMessageUpload)
            && !self.inner.custom_base_url
        {
            self.inner.environment.large_message_base_url()
        } else {
            self.inner.base_url.as_str()
        };
        let url = format!("{}{}", base_url, service.path());
        let version = service.soap_version();
        let mut http_request = self
            .inner
            .http
            .post(url)
            .basic_auth(
                &self.inner.credentials.username,
                Some(self.inner.credentials.password.expose_secret()),
            )
            .header(CONTENT_TYPE, version.content_type())
            .header(ACCEPT, version.content_type())
            .body(request);
        if version == soap::SoapVersion::Soap11 {
            http_request = http_request.header("SOAPAction", "\"\"");
        }
        if service.expect_continue() {
            http_request = http_request.header(EXPECT, "100-continue");
        }
        let response = match http_request.send().await {
            Ok(response) => response,
            Err(error) => {
                #[cfg(feature = "tracing")]
                tracing::debug!("ISDS transport request failed");
                return Err(error.into());
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            #[cfg(feature = "tracing")]
            tracing::debug!(http_status = status.as_u16(), "ISDS authentication failed");
            return Err(Error::Authentication {
                status: status.as_u16(),
            });
        }

        let body = match read_bounded(response, self.inner.max_response_bytes).await {
            Ok(body) => body,
            Err(error) => {
                #[cfg(feature = "tracing")]
                tracing::debug!("ISDS response body could not be read");
                return Err(error);
            }
        };
        #[cfg(feature = "tracing")]
        tracing::debug!(
            http_status = status.as_u16(),
            response_bytes = body.len(),
            "ISDS response received"
        );

        let decoded = soap::decode(&body, response_operation, decode_response);
        let result = if status.is_success() {
            decoded
        } else {
            match decoded {
                Err(error @ (Error::SoapFault { .. } | Error::Service(_))) => Err(error),
                _ => Err(Error::Http {
                    status: status.as_u16(),
                }),
            }
        };
        // Let hyper finish releasing the connection before callers immediately
        // chain another request on tokio's current-thread runtime.
        tokio::task::yield_now().await;
        result
    }

    #[cfg(test)]
    pub(crate) fn test_client(base_url: String) -> Self {
        ClientBuilder::test_at(Environment::Test, base_url)
            .build()
            .unwrap()
    }
}

impl ClientBuilder {
    /// Sets the username/password credentials used on every request.
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Sets the connection timeout. The default is 10 seconds.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Sets the total request timeout. The default is 30 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the maximum buffered response size in bytes.
    ///
    /// The default is 256 MiB. Ordinary and large-message attachments can be base64 encoded,
    /// so downloading them requires substantially more memory than their decoded size.
    pub fn max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    /// Sets the nominal decoded-attachment threshold for automatic large-message selection.
    ///
    /// The default is 20 MiB. ISDS documents an overlapping, non-fixed boundary
    /// around 20 MB, so this setting allows target-environment tuning. Equality
    /// uses ordinary `CreateMessage`; only a larger total uses large-volume transport.
    pub fn large_message_threshold_bytes(mut self, threshold: u64) -> Self {
        self.large_message_threshold_bytes = threshold;
        self
    }

    /// Sets the maximum total decoded attachment content accepted for sending.
    ///
    /// The default is 100 MiB. ISDS applies this limit to attachment content, not
    /// to the larger base64-encoded SOAP representation.
    pub fn max_attachment_bytes(mut self, maximum: u64) -> Self {
        self.max_attachment_bytes = maximum;
        self
    }

    /// Builds the client without performing any network request.
    pub fn build(self) -> Result<Client> {
        let credentials = self
            .credentials
            .ok_or_else(|| Error::invalid_input("credentials", "credentials are required"))?;
        if self.connect_timeout.is_zero() {
            return Err(Error::invalid_input(
                "connect_timeout",
                "timeout must be greater than zero",
            ));
        }
        if self.timeout.is_zero() {
            return Err(Error::invalid_input(
                "timeout",
                "timeout must be greater than zero",
            ));
        }
        if self.max_response_bytes == 0 {
            return Err(Error::invalid_input(
                "max_response_bytes",
                "limit must be greater than zero",
            ));
        }
        if self.large_message_threshold_bytes == 0 {
            return Err(Error::invalid_input(
                "large_message_threshold_bytes",
                "threshold must be greater than zero",
            ));
        }
        if self.max_attachment_bytes == 0 {
            return Err(Error::invalid_input(
                "max_attachment_bytes",
                "maximum must be greater than zero",
            ));
        }
        if self.large_message_threshold_bytes >= self.max_attachment_bytes {
            return Err(Error::invalid_input(
                "large_message_threshold_bytes",
                "threshold must be smaller than the maximum attachment size",
            ));
        }

        #[cfg(test)]
        let (base_url, https_only, custom_base_url) = match self.test_base_url {
            Some(url) => (url, false, true),
            None => (self.environment.basic_base_url().to_owned(), true, false),
        };
        #[cfg(not(test))]
        let (base_url, https_only, custom_base_url) =
            (self.environment.basic_base_url().to_owned(), true, false);

        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .https_only(https_only)
            .connect_timeout(self.connect_timeout)
            .timeout(self.timeout)
            .user_agent(concat!("isds/", env!("CARGO_PKG_VERSION")))
            .build()?;

        Ok(Client {
            inner: Arc::new(ClientInner {
                http,
                credentials,
                base_url: base_url.trim_end_matches('/').to_owned(),
                custom_base_url,
                environment: self.environment,
                max_response_bytes: self.max_response_bytes,
                large_message_threshold_bytes: self.large_message_threshold_bytes,
                max_attachment_bytes: self.max_attachment_bytes,
            }),
        })
    }

    #[cfg(test)]
    fn test_at(environment: Environment, base_url: String) -> Self {
        Self {
            test_base_url: Some(base_url),
            ..Client::builder(environment)
        }
        .credentials(Credentials::new("test-user", "sentinel-password"))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Service {
    MessageOperations,
    MessageInfo,
    LargeMessage,
    LargeMessageUpload,
    DataBoxSearch,
    DataBoxAccess,
}

impl Service {
    fn soap_version(self) -> soap::SoapVersion {
        match self {
            Self::MessageOperations
            | Self::MessageInfo
            | Self::DataBoxSearch
            | Self::DataBoxAccess => soap::SoapVersion::Soap11,
            Self::LargeMessage | Self::LargeMessageUpload => soap::SoapVersion::Soap12,
        }
    }

    fn expect_continue(self) -> bool {
        matches!(self, Self::LargeMessageUpload)
    }

    fn path(self) -> &'static str {
        match self {
            Self::MessageOperations => "/DS/dz",
            Self::MessageInfo => "/DS/dx",
            Self::LargeMessage | Self::LargeMessageUpload => "/DS/vodz",
            Self::DataBoxSearch => "/DS/df",
            Self::DataBoxAccess => "/DS/DsManage",
        }
    }
}

fn attachment_bytes(message: &OutgoingMessage) -> Result<u64> {
    message
        .attachments
        .iter()
        .try_fold(0_u64, |total, attachment| {
            total
                .checked_add(attachment.content().len() as u64)
                .ok_or_else(|| {
                    Error::invalid_input("attachments", "total attachment size overflows u64")
                })
        })
}

fn use_large_message(
    total_attachment_bytes: u64,
    threshold: u64,
    mode: Option<MessageMode>,
) -> bool {
    match mode {
        Some(MessageMode::Standard) => false,
        Some(MessageMode::Large) => true,
        None => total_attachment_bytes > threshold,
    }
}

fn ensure_message_id(expected: &MessageId, actual: &MessageId) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(Error::invalid_response(
            "response message ID does not match the requested message ID",
        ))
    }
}

async fn read_bounded(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::ResponseTooLarge { limit });
    }

    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let new_length = body.len().saturating_add(chunk.len());
        if new_length > limit {
            return Err(Error::ResponseTooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[cfg(feature = "tracing")]
    #[derive(Clone, Default)]
    struct TraceBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    #[cfg(feature = "tracing")]
    impl std::io::Write for TraceBuffer {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(feature = "tracing")]
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TraceBuffer {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn response_body(state: &str, code: &str, message: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
              <soap:Body>
                <CheckDataBoxResponse xmlns="http://isds.czechpoint.cz/v20">
                  {state}
                  <dbStatus>
                    <dbStatusCode>{code}</dbStatusCode>
                    <dbStatusMessage>{message}</dbStatusMessage>
                    <dbStatusRefNumber>ref-123</dbStatusRefNumber>
                  </dbStatus>
                </CheckDataBoxResponse>
              </soap:Body>
            </soap:Envelope>"#,
            state = if state.is_empty() {
                String::new()
            } else {
                format!("<dbState>{state}</dbState>")
            }
        )
    }

    fn id() -> DataBoxId {
        DataBoxId::new("egtk37z").unwrap()
    }

    fn outgoing() -> OutgoingMessage {
        OutgoingMessage::new(DataBoxId::new("n94k37v").unwrap(), "Synthetic test").with_attachment(
            crate::model::Attachment::main(
                "test.bin",
                mime::APPLICATION_OCTET_STREAM,
                bytes::Bytes::from_static(&[0, 1, 2, 255]),
            ),
        )
    }

    #[tokio::test]
    async fn check_data_box_uses_the_selected_service_and_basic_auth() {
        let server = MockServer::start().await;
        let expected_auth = format!("Basic {}", BASE64.encode("test-user:sentinel-password"));
        Mock::given(method("POST"))
            .and(path("/DS/df"))
            .and(wiremock::matchers::header("authorization", expected_auth))
            .and(wiremock::matchers::header(
                "content-type",
                "text/xml; charset=utf-8",
            ))
            .and(wiremock::matchers::header("soapaction", "\"\""))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body("1", "0000", "OK")),
            )
            .expect(1)
            .mount(&server)
            .await;

        let state = Client::test_client(server.uri())
            .check_data_box(&id())
            .await
            .unwrap();
        assert!(state.is_active());
        server.verify().await;
    }

    #[cfg(feature = "tracing")]
    #[tokio::test(flavor = "current_thread")]
    async fn tracing_emits_request_metadata_without_sensitive_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/DS/df"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body("1", "0000", "OK")),
            )
            .mount(&server)
            .await;

        let buffer = TraceBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(buffer.clone())
            .with_env_filter("isds=debug")
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
        tracing::callsite::rebuild_interest_cache();

        Client::test_client(server.uri())
            .check_data_box(&id())
            .await
            .unwrap();

        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert!(output.contains("isds.request"), "{output}");
        assert!(output.contains("CheckDataBox"), "{output}");
        assert!(output.contains("/DS/df"), "{output}");
        assert!(output.contains("http_status=200"), "{output}");
        assert!(output.contains("response_bytes="), "{output}");
        for sensitive in [
            "test-user",
            "sentinel-password",
            "egtk37z",
            "<soap:Envelope",
        ] {
            assert!(!output.contains(sensitive), "{output}");
        }
    }

    #[tokio::test]
    async fn service_status_is_an_error_even_on_http_200() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(response_body(
                "",
                "1010",
                "Missing databox",
            )))
            .mount(&server)
            .await;

        let error = Client::test_client(server.uri())
            .check_data_box(&id())
            .await
            .unwrap_err();
        let Error::Service(service) = error else {
            panic!("expected service error");
        };
        assert_eq!(service.code(), "1010");
        assert_eq!(service.message(), "Missing databox");
        assert_eq!(service.reference_number(), Some("ref-123"));
    }

    #[tokio::test]
    async fn html_authentication_failure_does_not_leak_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(401)
                    .set_body_string("<html>sentinel-password and private body</html>"),
            )
            .mount(&server)
            .await;

        let error = Client::test_client(server.uri())
            .check_data_box(&id())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Authentication { status: 401 }));
        let rendered = error.to_string();
        assert!(!rendered.contains("sentinel-password"));
        assert!(!rendered.contains("private body"));
    }

    #[tokio::test]
    async fn authentication_precedes_response_body_limits() {
        for status in [401, 403] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_string("x".repeat(1024)))
                .expect(1)
                .mount(&server)
                .await;
            let client = ClientBuilder::test_at(Environment::Test, server.uri())
                .max_response_bytes(64)
                .build()
                .unwrap();

            let error = client.check_data_box(&id()).await.unwrap_err();
            assert!(matches!(
                error,
                Error::Authentication { status: actual } if actual == status
            ));
            server.verify().await;
        }
    }

    #[tokio::test]
    async fn soap_fault_is_preserved_on_http_500() {
        let server = MockServer::start().await;
        let fault = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body><soap:Fault><faultcode>soap:Server</faultcode>
            <faultstring>Internal error</faultstring></soap:Fault></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string(fault))
            .mount(&server)
            .await;

        let error = Client::test_client(server.uri())
            .check_data_box(&id())
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::SoapFault { ref code, ref message }
                if code == "soap:Server" && message == "Internal error"
        ));
    }

    #[tokio::test]
    async fn unexpected_http_status_and_malformed_success_are_distinct() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_string("maintenance"))
            .mount(&server)
            .await;
        let error = Client::test_client(server.uri())
            .check_data_box(&id())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http { status: 503 }));

        let malformed_server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<not-xml"))
            .mount(&malformed_server)
            .await;
        let error = Client::test_client(malformed_server.uri())
            .check_data_box(&id())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }

    #[tokio::test]
    async fn response_limit_is_enforced() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(1024)))
            .mount(&server)
            .await;
        let client = ClientBuilder::test_at(Environment::Test, server.uri())
            .max_response_bytes(64)
            .build()
            .unwrap();
        let error = client.check_data_box(&id()).await.unwrap_err();
        assert!(matches!(error, Error::ResponseTooLarge { limit: 64 }));
    }

    #[tokio::test]
    async fn timeout_does_not_retry_the_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(100))
                    .set_body_string(response_body("1", "0000", "OK")),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = ClientBuilder::test_at(Environment::Test, server.uri())
            .timeout(Duration::from_millis(10))
            .build()
            .unwrap();
        let error = client.check_data_box(&id()).await.unwrap_err();
        assert!(matches!(error, Error::Transport(_)));
        server.verify().await;
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let source = MockServer::start().await;
        let destination = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", destination.uri()))
            .expect(1)
            .mount(&source)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&destination)
            .await;

        let error = Client::test_client(source.uri())
            .check_data_box(&id())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Http { status: 302 }));
        source.verify().await;
        destination.verify().await;
    }

    #[tokio::test]
    async fn account_lookup_and_credit_operations_use_their_contract_endpoints() {
        let server = MockServer::start().await;
        let owner_body = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><GetOwnerInfoFromLogin2Response xmlns="http://isds.czechpoint.cz/v20">
            <dbOwnerInfo><dbID>egtk37z</dbID><dbType>PO</dbType></dbOwnerInfo>
            <dbStatus><dbStatusCode>0000</dbStatusCode><dbStatusMessage>OK</dbStatusMessage></dbStatus>
          </GetOwnerInfoFromLogin2Response></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/DsManage"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(databox::owner_info_request().unwrap()).unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(owner_body))
            .expect(1)
            .mount(&server)
            .await;

        let query = DataBox::by_id(id());
        let find_body = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><FindDataBox2Response xmlns="http://isds.czechpoint.cz/v20">
            <dbResults><dbOwnerInfo><dbID>egtk37z</dbID><dbType>PO</dbType></dbOwnerInfo></dbResults>
            <dbStatus><dbStatusCode>0003</dbStatusCode><dbStatusMessage>Truncated</dbStatusMessage></dbStatus>
          </FindDataBox2Response></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/df"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(databox::find_data_boxes_request(&query).unwrap()).unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(find_body))
            .expect(1)
            .mount(&server)
            .await;

        let credit_query = CreditQuery::current(id());
        let credit_body = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><DataBoxCreditInfoResponse xmlns="http://isds.czechpoint.cz/v20">
            <currentCredit>500</currentCredit>
            <dbStatus><dbStatusCode>0000</dbStatusCode><dbStatusMessage>OK</dbStatusMessage></dbStatus>
          </DataBoxCreditInfoResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/df"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(credit::credit_info_request(&credit_query).unwrap()).unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(credit_body))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::test_client(server.uri());
        assert_eq!(client.owner_info().await.unwrap().id, Some(id()));
        let found = client.find_data_boxes(&query).await.unwrap();
        assert_eq!(found.boxes.len(), 1);
        assert!(found.truncated);
        assert_eq!(
            client
                .credit_info(&credit_query)
                .await
                .unwrap()
                .current_credit,
            Some(500)
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn send_message_uses_message_operations_and_preserves_the_id() {
        let server = MockServer::start().await;
        let outgoing = outgoing();
        let response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><CreateMessageResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmID>00042</dmID><dmStatus><dmStatusCode>0000</dmStatusCode>
            <dmStatusMessage>Accepted</dmStatusMessage></dmStatus>
          </CreateMessageResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dz"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message::create_message_request(&outgoing).unwrap()).unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(response))
            .expect(2)
            .mount(&server)
            .await;

        let sent = Client::test_client(server.uri())
            .send_message(&outgoing, None)
            .await
            .unwrap();
        assert_eq!(sent.id.as_str(), "00042");

        let forced = ClientBuilder::test_at(Environment::Test, server.uri())
            .large_message_threshold_bytes(1)
            .max_attachment_bytes(1024)
            .build()
            .unwrap();
        let forced_sent = forced
            .send_message(&outgoing, Some(MessageMode::Standard))
            .await
            .unwrap();
        assert_eq!(forced_sent.id.as_str(), "00042");
        server.verify().await;
    }

    #[tokio::test]
    async fn send_message_automatically_uploads_and_creates_a_large_message() {
        let server = MockServer::start().await;
        let outgoing = outgoing();
        let upload_response = r#"<UploadAttachmentResponse xmlns="http://isds.czechpoint.cz/v20">
          <dmAttID>upload-1</dmAttID>
          <dmAttHash1 AttHashAlg="SHA-256">3d1f57c984978ef98a18378c8166c1cb8ede02c03eeb6aee7e2f121dfeee3e56</dmAttHash1>
          <dmAttHash2 AttHashAlg="SHA3-256">9611923b9b1fe6ba837b2246b1e1a26ac51ecc2af6a2c662acb705fd2e8c4d4b</dmAttHash2>
          <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
        </UploadAttachmentResponse>"#;
        let uploaded = large_message::decode_upload_attachment(
            roxmltree::Document::parse(upload_response)
                .unwrap()
                .root_element(),
            &outgoing.attachments[0],
        )
        .unwrap();
        let upload_response = format!(
            r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope"><env:Body>{upload_response}</env:Body></env:Envelope>"#
        );

        Mock::given(method("POST"))
            .and(path("/DS/vodz"))
            .and(wiremock::matchers::header(
                "content-type",
                "application/soap+xml; charset=utf-8",
            ))
            .and(wiremock::matchers::header("expect", "100-continue"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(
                    large_message::upload_attachment_request(&outgoing.attachments[0]).unwrap(),
                )
                .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(upload_response))
            .expect(2)
            .mount(&server)
            .await;

        let create_response = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
          <env:Body><CreateBigMessageResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmID>00099</dmID><dmStatus><dmStatusCode>0000</dmStatusCode>
            <dmStatusMessage>Accepted</dmStatusMessage></dmStatus>
          </CreateBigMessageResponse></env:Body></env:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/vodz"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(
                    large_message::create_large_message_request(&outgoing, &[uploaded]).unwrap(),
                )
                .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(create_response))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/DS/dz"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let client = ClientBuilder::test_at(Environment::Test, server.uri())
            .large_message_threshold_bytes(1)
            .max_attachment_bytes(1024)
            .build()
            .unwrap();
        let sent = client.send_message(&outgoing, None).await.unwrap();
        assert_eq!(sent.id.as_str(), "00099");

        let forced = ClientBuilder::test_at(Environment::Test, server.uri())
            .large_message_threshold_bytes(1024)
            .max_attachment_bytes(2048)
            .build()
            .unwrap();
        let forced_sent = forced
            .send_message(&outgoing, Some(MessageMode::Large))
            .await
            .unwrap();
        assert_eq!(forced_sent.id.as_str(), "00099");
        server.verify().await;
    }

    #[tokio::test]
    async fn oversized_message_is_rejected_before_any_upload() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let client = ClientBuilder::test_at(Environment::Test, server.uri())
            .large_message_threshold_bytes(1)
            .max_attachment_bytes(2)
            .build()
            .unwrap();
        let error = client.send_message(&outgoing(), None).await.unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidInput {
                field: "attachments",
                ..
            }
        ));
        server.verify().await;
    }

    #[test]
    fn large_message_threshold_uses_strictly_greater_than() {
        let threshold = 20 * 1024 * 1024;
        assert!(!use_large_message(threshold - 1, threshold, None));
        assert!(!use_large_message(threshold, threshold, None));
        assert!(use_large_message(threshold + 1, threshold, None));
    }

    #[test]
    fn forced_message_mode_overrides_the_threshold() {
        let threshold = 20 * 1024 * 1024;
        assert!(!use_large_message(
            threshold + 1,
            threshold,
            Some(MessageMode::Standard)
        ));
        assert!(use_large_message(1, threshold, Some(MessageMode::Large)));
    }

    #[tokio::test]
    async fn invalid_outgoing_message_is_not_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let mut outgoing = outgoing();
        outgoing.attachments.clear();
        let error = Client::test_client(server.uri())
            .send_message(&outgoing, None)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidInput {
                field: "attachments",
                ..
            }
        ));
        server.verify().await;
    }

    #[tokio::test]
    async fn send_timeout_makes_exactly_one_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
            .expect(1)
            .mount(&server)
            .await;
        let client = ClientBuilder::test_at(Environment::Test, server.uri())
            .timeout(Duration::from_millis(10))
            .build()
            .unwrap();
        let error = client.send_message(&outgoing(), None).await.unwrap_err();
        assert!(matches!(error, Error::Transport(_)));
        server.verify().await;
    }

    #[tokio::test]
    async fn receiving_and_delivery_operations_use_their_contract_endpoints() {
        let server = MockServer::start().await;
        let client = Client::test_client(server.uri());
        let query = MessageListQuery {
            offset: 1,
            limit: 10,
            ..MessageListQuery::default()
        };
        let message_id = MessageId::new("00042").unwrap();

        for (kind, operation) in [
            (message_info::MessageListKind::Sent, "GetListOfSentMessages"),
            (
                message_info::MessageListKind::Received,
                "GetListOfReceivedMessages",
            ),
        ] {
            let response = format!(
                r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
                  <soap:Body><{operation}Response xmlns="http://isds.czechpoint.cz/v20">
                    <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
                  </{operation}Response></soap:Body></soap:Envelope>"#
            );
            Mock::given(method("POST"))
                .and(path("/DS/dx"))
                .and(wiremock::matchers::body_string(
                    String::from_utf8(message_info::list_request(kind, &query).unwrap()).unwrap(),
                ))
                .respond_with(ResponseTemplate::new(200).set_body_string(response))
                .expect(1)
                .mount(&server)
                .await;
        }

        let download_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MessageDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessage><dmDm><dmID>00042</dmID><dmSenderType>31</dmSenderType>
              <dmFiles><dmFile dmMimeType="application/octet-stream" dmFileMetaType="main"
                dmFileDescr="data.bin"><dmEncodedContent>AAEC/f7/</dmEncodedContent></dmFile></dmFiles>
            </dmDm><dmHash>AAEC/f7/</dmHash><dmQTimestamp></dmQTimestamp>
            <dmMessageStatus>6</dmMessageStatus></dmReturnedMessage>
          </MessageDownloadResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dz"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_download::download_request(&message_id).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(download_response))
            .expect(2)
            .mount(&server)
            .await;

        let delivery_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><GetDeliveryInfoResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmDelivery><dmDm><dmID>00042</dmID><dmSenderType>31</dmSenderType></dmDm>
              <dmHash>AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus><dmEvents><dmEvent>
                <dmEventDescr>Delivered</dmEventDescr></dmEvent></dmEvents>
            </dmDelivery>
          </GetDeliveryInfoResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dx"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_info::delivery_info_request(&message_id).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(delivery_response))
            .expect(1)
            .mount(&server)
            .await;

        let mark_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MarkMessageAsDownloadedResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </MarkMessageAsDownloadedResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dx"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_info::mark_downloaded_request(&message_id).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(mark_response))
            .expect(1)
            .mount(&server)
            .await;

        let envelope_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MessageEnvelopeDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessageEnvelope dmVODZ="false"><dmDm>
              <dmID>00042</dmID><dmSenderType>31</dmSenderType></dmDm>
              <dmHash>AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus></dmReturnedMessageEnvelope>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </MessageEnvelopeDownloadResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dx"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_info::envelope_download_request(&message_id).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(envelope_response))
            .expect(1)
            .mount(&server)
            .await;

        assert!(
            client
                .list_sent_messages(&query)
                .await
                .unwrap()
                .records
                .is_empty()
        );
        assert!(
            client
                .list_received_messages(&query)
                .await
                .unwrap()
                .records
                .is_empty()
        );
        let downloaded = client.download_message(&message_id, None).await.unwrap();
        assert_eq!(
            downloaded.attachments[0].content().as_ref(),
            &[0, 1, 2, 253, 254, 255]
        );
        let forced = client
            .download_message(&message_id, Some(MessageMode::Standard))
            .await
            .unwrap();
        assert_eq!(forced.envelope.id, message_id);
        assert_eq!(
            client
                .delivery_info(&message_id)
                .await
                .unwrap()
                .events
                .len(),
            1
        );
        client.mark_message_downloaded(&message_id).await.unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn automatic_large_download_and_envelope_download_use_their_documented_services() {
        let server = MockServer::start().await;
        let client = Client::test_client(server.uri());
        let id = MessageId::new("00042").unwrap();

        let large_response = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
          <env:Body><BigMessageDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessage dmVODZ="true" attsNum="1"><dmDm><dmID>00042</dmID>
              <dmSenderType>31</dmSenderType><dmFiles><dmFile dmMimeType="application/octet-stream"
                dmFileMetaType="main" dmFileDescr="large.bin">
                <dmEncodedContent>AAEC/f7/</dmEncodedContent></dmFile></dmFiles>
            </dmDm><dmHash>AAEC/f7/</dmHash><dmQTimestamp></dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus></dmReturnedMessage>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </BigMessageDownloadResponse></env:Body></env:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/vodz"))
            .and(wiremock::matchers::header(
                "content-type",
                "application/soap+xml; charset=utf-8",
            ))
            .and(wiremock::matchers::body_string(
                String::from_utf8(large_message::large_message_download_request(&id).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(large_response))
            .expect(2)
            .mount(&server)
            .await;

        let envelope_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MessageEnvelopeDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessageEnvelope dmVODZ="true" attsNum="1"><dmDm>
              <dmID>00042</dmID><dmSenderType>31</dmSenderType></dmDm>
              <dmHash>AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus></dmReturnedMessageEnvelope>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </MessageEnvelopeDownloadResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dx"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_info::envelope_download_request(&id).unwrap()).unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(envelope_response))
            .expect(2)
            .mount(&server)
            .await;

        let message = client.download_message(&id, None).await.unwrap();
        assert_eq!(message.mode, Some(MessageMode::Large));
        assert_eq!(message.attachments.len(), 1);
        let forced = client
            .download_message(&id, Some(MessageMode::Large))
            .await
            .unwrap();
        assert_eq!(forced.envelope.id, id);
        let envelope = client.download_message_envelope(&id).await.unwrap();
        assert_eq!(envelope.mode, Some(MessageMode::Large));
        assert_eq!(envelope.attachment_count, Some(1));
        server.verify().await;
    }

    #[tokio::test]
    async fn downloads_and_delivery_info_reject_mismatched_message_ids() {
        fn assert_mismatch(error: Error) {
            match error {
                Error::InvalidResponse { message } => assert_eq!(
                    message,
                    "response message ID does not match the requested message ID"
                ),
                other => panic!("expected invalid response, got {other:?}"),
            }
        }

        let server = MockServer::start().await;
        let client = Client::test_client(server.uri());
        let requested = MessageId::new("00042").unwrap();

        let standard_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MessageDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessage><dmDm><dmID>00043</dmID><dmSenderType>31</dmSenderType>
              <dmFiles><dmFile dmMimeType="application/octet-stream" dmFileMetaType="main"
                dmFileDescr="data.bin"><dmEncodedContent>AAEC/f7/</dmEncodedContent></dmFile></dmFiles>
            </dmDm><dmHash>AAEC/f7/</dmHash><dmQTimestamp></dmQTimestamp>
            <dmMessageStatus>6</dmMessageStatus></dmReturnedMessage>
          </MessageDownloadResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dz"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_download::download_request(&requested).unwrap()).unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(standard_response))
            .expect(1)
            .mount(&server)
            .await;

        let large_response = r#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
          <env:Body><BigMessageDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessage dmVODZ="true" attsNum="1"><dmDm><dmID>00043</dmID>
              <dmSenderType>31</dmSenderType><dmFiles><dmFile dmMimeType="application/octet-stream"
                dmFileMetaType="main" dmFileDescr="large.bin">
                <dmEncodedContent>AAEC/f7/</dmEncodedContent></dmFile></dmFiles>
            </dmDm><dmHash>AAEC/f7/</dmHash><dmQTimestamp></dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus></dmReturnedMessage>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </BigMessageDownloadResponse></env:Body></env:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/vodz"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(
                    large_message::large_message_download_request(&requested).unwrap(),
                )
                .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(large_response))
            .expect(1)
            .mount(&server)
            .await;

        let envelope_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MessageEnvelopeDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessageEnvelope dmVODZ="false"><dmDm>
              <dmID>00043</dmID><dmSenderType>31</dmSenderType></dmDm>
              <dmHash>AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus></dmReturnedMessageEnvelope>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </MessageEnvelopeDownloadResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dx"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_info::envelope_download_request(&requested).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(envelope_response))
            .expect(1)
            .mount(&server)
            .await;

        let delivery_response = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><GetDeliveryInfoResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmDelivery><dmDm><dmID>00043</dmID><dmSenderType>31</dmSenderType></dmDm>
              <dmHash>AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmMessageStatus>6</dmMessageStatus><dmEvents><dmEvent>
                <dmEventDescr>Delivered</dmEventDescr></dmEvent></dmEvents>
            </dmDelivery>
          </GetDeliveryInfoResponse></soap:Body></soap:Envelope>"#;
        Mock::given(method("POST"))
            .and(path("/DS/dx"))
            .and(wiremock::matchers::body_string(
                String::from_utf8(message_info::delivery_info_request(&requested).unwrap())
                    .unwrap(),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(delivery_response))
            .expect(1)
            .mount(&server)
            .await;

        assert_mismatch(
            client
                .download_message(&requested, Some(MessageMode::Standard))
                .await
                .unwrap_err(),
        );
        assert_mismatch(
            client
                .download_message(&requested, Some(MessageMode::Large))
                .await
                .unwrap_err(),
        );
        assert_mismatch(
            client
                .download_message_envelope(&requested)
                .await
                .unwrap_err(),
        );
        assert_mismatch(client.delivery_info(&requested).await.unwrap_err());
        server.verify().await;
    }

    #[tokio::test]
    async fn mark_downloaded_timeout_makes_exactly_one_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(100)))
            .expect(1)
            .mount(&server)
            .await;
        let client = ClientBuilder::test_at(Environment::Test, server.uri())
            .timeout(Duration::from_millis(10))
            .build()
            .unwrap();
        let error = client
            .mark_message_downloaded(&MessageId::new("00042").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::Transport(_)));
        server.verify().await;
    }

    #[test]
    fn credentials_and_client_debug_output_are_redacted() {
        let credentials = Credentials::new("test-user", "sentinel-password");
        let rendered = format!("{credentials:?}");
        assert_eq!(
            rendered,
            r#"Credentials { username: "[REDACTED]", password: "[REDACTED]" }"#
        );
        assert!(!rendered.contains("sentinel-password"));
        assert!(!rendered.contains("test-user"));

        let client = Client::builder(Environment::Test)
            .credentials(credentials)
            .build()
            .unwrap();
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("sentinel-password"));
        assert!(!rendered.contains("test-user"));
        assert!(rendered.contains("ws1.datovka-test.gov.cz"));
    }

    #[test]
    fn builder_requires_credentials_and_nonzero_limits() {
        let missing = Client::builder(Environment::Test).build().unwrap_err();
        assert!(matches!(
            missing,
            Error::InvalidInput {
                field: "credentials",
                ..
            }
        ));

        let zero = Client::builder(Environment::Production)
            .credentials(Credentials::new("user", "password"))
            .max_response_bytes(0)
            .build()
            .unwrap_err();
        assert!(matches!(
            zero,
            Error::InvalidInput {
                field: "max_response_bytes",
                ..
            }
        ));
    }
}
