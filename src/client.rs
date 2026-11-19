use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use secrecy::{ExposeSecret, SecretString};

use crate::error::{Error, Result};
use crate::model::{
    CreditInfo, CreditQuery, DataBox, DataBoxId, DataBoxSearchResult, DataBoxState,
    OutgoingMessage, SentMessage,
};
use crate::wire::{credit, databox, message, soap};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 128 * 1024 * 1024;

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
}

/// Username and password used for ISDS HTTP Basic authentication.
///
/// The password is redacted from `Debug` output. Basic credentials are sent on
/// every request; this authentication method does not establish a cookie session.
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
            .field("username", &self.username)
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
    environment: Environment,
    max_response_bytes: usize,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("environment", &self.inner.environment)
            .field("base_url", &self.inner.base_url)
            .field("max_response_bytes", &self.inner.max_response_bytes)
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

    /// Sends one ordinary message using `CreateMessage`.
    ///
    /// Success confirms acceptance by ISDS, not delivery. This method never
    /// retries automatically. A transport failure has an ambiguous outcome:
    /// reconcile through sent-message queries before submitting the same content
    /// again.
    pub async fn send_message(&self, message: &OutgoingMessage) -> Result<SentMessage> {
        self.execute(
            Service::MessageOperations,
            message::create_message_request(message)?,
            "CreateMessageResponse",
            message::decode_create_message,
        )
        .await
    }

    pub(crate) async fn execute<T>(
        &self,
        service: Service,
        request: Vec<u8>,
        response_operation: &str,
        decode_response: impl FnOnce(roxmltree::Node<'_, '_>) -> Result<T>,
    ) -> Result<T> {
        let url = format!("{}{}", self.inner.base_url, service.path());
        let response = self
            .inner
            .http
            .post(url)
            .basic_auth(
                &self.inner.credentials.username,
                Some(self.inner.credentials.password.expose_secret()),
            )
            .header(CONTENT_TYPE, "text/xml; charset=utf-8")
            .header(ACCEPT, "text/xml")
            .header("SOAPAction", "\"\"")
            .body(request)
            .send()
            .await?;

        let status = response.status();
        let body = read_bounded(response, self.inner.max_response_bytes).await?;

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(Error::Authentication {
                status: status.as_u16(),
            });
        }

        let decoded = soap::decode(&body, response_operation, decode_response);
        if status.is_success() {
            decoded
        } else {
            match decoded {
                Err(error @ (Error::SoapFault { .. } | Error::Service(_))) => Err(error),
                _ => Err(Error::Http {
                    status: status.as_u16(),
                }),
            }
        }
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
    /// The default is 128 MiB. Ordinary SOAP attachments are base64 encoded, so
    /// downloading them requires substantially more memory than their decoded size.
    pub fn max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
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

        #[cfg(test)]
        let (base_url, https_only) = match self.test_base_url {
            Some(url) => (url, false),
            None => (self.environment.basic_base_url().to_owned(), true),
        };
        #[cfg(not(test))]
        let (base_url, https_only) = (self.environment.basic_base_url().to_owned(), true);

        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
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
                environment: self.environment,
                max_response_bytes: self.max_response_bytes,
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
    DataBoxSearch,
    DataBoxAccess,
}

impl Service {
    fn path(self) -> &'static str {
        match self {
            Self::MessageOperations => "/DS/dz",
            Self::DataBoxSearch => "/DS/df",
            Self::DataBoxAccess => "/DS/DsManage",
        }
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
            .expect(1)
            .mount(&server)
            .await;

        let sent = Client::test_client(server.uri())
            .send_message(&outgoing)
            .await
            .unwrap();
        assert_eq!(sent.id.as_str(), "00042");
        server.verify().await;
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
            .send_message(&outgoing)
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
        let error = client.send_message(&outgoing()).await.unwrap_err();
        assert!(matches!(error, Error::Transport(_)));
        server.verify().await;
    }

    #[test]
    fn credentials_and_client_debug_output_are_redacted() {
        let credentials = Credentials::new("test-user", "sentinel-password");
        let rendered = format!("{credentials:?}");
        assert!(!rendered.contains("sentinel-password"));

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
