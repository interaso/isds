use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use roxmltree::{Document, Node, ParsingOptions};

use crate::error::{Error, Result, ServiceError};

pub(crate) const ISDS_NS: &str = "http://isds.czechpoint.cz/v20";
const SOAP_11_NS: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const SOAP_12_NS: &str = "http://www.w3.org/2003/05/soap-envelope";
// These internal bounds leave ample room for protocol messages while preventing
// tiny XML inputs from causing disproportionate tree allocation or deep nesting.
const MAX_XML_NODES: u32 = 65_536;
const MAX_XML_DEPTH: u32 = 128;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SoapVersion {
    Soap11,
    Soap12,
}

impl SoapVersion {
    fn namespace(self) -> &'static str {
        match self {
            Self::Soap11 => SOAP_11_NS,
            Self::Soap12 => SOAP_12_NS,
        }
    }

    pub(crate) fn content_type(self) -> &'static str {
        match self {
            Self::Soap11 => "text/xml; charset=utf-8",
            Self::Soap12 => "application/soap+xml; charset=utf-8",
        }
    }
}

pub(crate) type XmlResult = std::result::Result<(), quick_xml::Error>;

pub(crate) fn request(
    operation: &str,
    write_operation: impl FnOnce(&mut Writer<Vec<u8>>) -> XmlResult,
) -> Result<Vec<u8>> {
    request_with_version(SoapVersion::Soap11, operation, write_operation)
}

pub(crate) fn request_with_version(
    version: SoapVersion,
    operation: &str,
    write_operation: impl FnOnce(&mut Writer<Vec<u8>>) -> XmlResult,
) -> Result<Vec<u8>> {
    let mut writer = Writer::new(Vec::new());
    write_envelope(&mut writer, version, operation, write_operation).map_err(|error| {
        Error::invalid_input("xml", format!("could not encode request: {error}"))
    })?;
    Ok(writer.into_inner())
}

fn write_envelope(
    writer: &mut Writer<Vec<u8>>,
    version: SoapVersion,
    operation: &str,
    write_operation: impl FnOnce(&mut Writer<Vec<u8>>) -> XmlResult,
) -> XmlResult {
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    let mut envelope = BytesStart::new("soap:Envelope");
    envelope.push_attribute(("xmlns:soap", version.namespace()));
    envelope.push_attribute(("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance"));
    writer.write_event(Event::Start(envelope))?;
    writer.write_event(Event::Start(BytesStart::new("soap:Body")))?;

    let mut operation_start = BytesStart::new(operation);
    operation_start.push_attribute(("xmlns", ISDS_NS));
    writer.write_event(Event::Start(operation_start))?;
    write_operation(writer)?;
    writer.write_event(Event::End(BytesEnd::new(operation)))?;

    writer.write_event(Event::End(BytesEnd::new("soap:Body")))?;
    writer.write_event(Event::End(BytesEnd::new("soap:Envelope")))?;
    Ok(())
}

pub(crate) fn text_element(writer: &mut Writer<Vec<u8>>, name: &str, value: &str) -> XmlResult {
    writer.write_event(Event::Start(BytesStart::new(name)))?;
    writer.write_event(Event::Text(BytesText::new(value)))?;
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

pub(crate) fn element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    write_contents: impl FnOnce(&mut Writer<Vec<u8>>) -> XmlResult,
) -> XmlResult {
    writer.write_event(Event::Start(BytesStart::new(name)))?;
    write_contents(writer)?;
    writer.write_event(Event::End(BytesEnd::new(name)))?;
    Ok(())
}

pub(crate) fn nillable_text_element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    value: Option<&str>,
) -> XmlResult {
    match value {
        Some(value) => text_element(writer, name, value),
        None => {
            let mut element = BytesStart::new(name);
            element.push_attribute(("xsi:nil", "true"));
            writer.write_event(Event::Empty(element))?;
            Ok(())
        }
    }
}

pub(crate) fn parse<T: std::str::FromStr>(value: &str, name: &str) -> Result<T> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::invalid_response(format!("{name} has an invalid value")))
}

pub(crate) fn parse_bool(value: &str, name: &str) -> Result<bool> {
    match value.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(Error::invalid_response(format!(
            "{name} is not an XML boolean"
        ))),
    }
}

pub(crate) fn xml_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

pub(crate) struct Status {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) reference_number: Option<String>,
}

impl Status {
    pub(crate) fn is_success(&self) -> bool {
        self.code == "0000"
    }

    pub(crate) fn into_result(self) -> Result<Self> {
        if self.is_success() {
            Ok(self)
        } else {
            Err(self.into_error())
        }
    }

    pub(crate) fn into_error(self) -> Error {
        ServiceError::new(self.code, self.message, self.reference_number).into()
    }
}

pub(crate) fn decode<T>(
    bytes: &[u8],
    operation: &str,
    decode_response: impl FnOnce(Node<'_, '_>) -> Result<T>,
) -> Result<T> {
    let xml = std::str::from_utf8(bytes)
        .map_err(|_| Error::invalid_response("response is not UTF-8 XML"))?;
    preflight(xml)?;
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_XML_NODES,
        ..ParsingOptions::default()
    };
    let document = Document::parse_with_options(xml, options)
        .map_err(|_| Error::invalid_response("malformed XML response"))?;

    let envelope = document.root_element();
    let version = match envelope.tag_name().namespace() {
        Some(SOAP_11_NS) => SoapVersion::Soap11,
        Some(SOAP_12_NS) => SoapVersion::Soap12,
        _ => {
            return Err(Error::invalid_response(
                "unsupported SOAP envelope namespace",
            ));
        }
    };
    require_tag(envelope, version.namespace(), "Envelope")?;
    let body = child(envelope, version.namespace(), "Body")?
        .ok_or_else(|| Error::invalid_response("SOAP Body is missing"))?;

    if let Some(fault) = child(body, version.namespace(), "Fault")? {
        let (code_name, message_name) = match version {
            SoapVersion::Soap11 => ("faultcode", "faultstring"),
            SoapVersion::Soap12 => ("Value", "Text"),
        };
        let code = descendant_text(fault, code_name)?.unwrap_or_else(|| "unknown".to_owned());
        let message = descendant_text(fault, message_name)?
            .unwrap_or_else(|| "unspecified SOAP fault".to_owned());
        return Err(Error::SoapFault { code, message });
    }

    let response = child(body, ISDS_NS, operation)?.ok_or_else(|| {
        Error::invalid_response(format!("SOAP Body does not contain {operation}"))
    })?;
    decode_response(response)
}

fn preflight(xml: &str) -> Result<()> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut nodes = 0_u32;
    let mut depth = 0_u32;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(element)) => {
                nodes = preflight_node(nodes)?;
                nodes = preflight_attributes(&element, nodes)?;
                depth = depth.checked_add(1).ok_or_else(|| {
                    Error::invalid_response("XML response exceeds the nesting limit")
                })?;
                if depth > MAX_XML_DEPTH {
                    return Err(Error::invalid_response(
                        "XML response exceeds the nesting limit",
                    ));
                }
            }
            Ok(Event::Empty(element)) => {
                if depth >= MAX_XML_DEPTH {
                    return Err(Error::invalid_response(
                        "XML response exceeds the nesting limit",
                    ));
                }
                nodes = preflight_node(nodes)?;
                nodes = preflight_attributes(&element, nodes)?;
            }
            Ok(Event::End(_)) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| Error::invalid_response("malformed XML response"))?;
            }
            Ok(Event::DocType(_)) => {
                return Err(Error::invalid_response(
                    "DTD is not allowed in XML responses",
                ));
            }
            Ok(
                Event::Text(_)
                | Event::CData(_)
                | Event::Comment(_)
                | Event::Decl(_)
                | Event::PI(_)
                | Event::GeneralRef(_),
            ) => {
                nodes = preflight_node(nodes)?;
            }
            Err(_) => return Err(Error::invalid_response("malformed XML response")),
        }
    }
    if depth != 0 {
        return Err(Error::invalid_response("malformed XML response"));
    }
    Ok(())
}

fn preflight_node(nodes: u32) -> Result<u32> {
    let nodes = nodes
        .checked_add(1)
        .ok_or_else(|| Error::invalid_response("XML response exceeds the node limit"))?;
    if nodes > MAX_XML_NODES {
        return Err(Error::invalid_response(
            "XML response exceeds the node limit",
        ));
    }
    Ok(nodes)
}

fn preflight_attributes(element: &BytesStart<'_>, mut nodes: u32) -> Result<u32> {
    for attribute in element.attributes() {
        if attribute.is_err() {
            return Err(Error::invalid_response("malformed XML response"));
        }
        nodes = preflight_node(nodes)?;
    }
    Ok(nodes)
}

pub(crate) fn require_tag(node: Node<'_, '_>, namespace: &str, name: &str) -> Result<()> {
    if node.tag_name().namespace() == Some(namespace) && node.tag_name().name() == name {
        Ok(())
    } else {
        Err(Error::invalid_response(format!(
            "expected XML element {name}"
        )))
    }
}

pub(crate) fn child<'a, 'input>(
    node: Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> Result<Option<Node<'a, 'input>>> {
    let mut matches = children(node, namespace, name);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(Error::invalid_response(format!(
            "XML element {name} occurs more than once"
        )));
    }
    Ok(first)
}

pub(crate) fn children<'a, 'input>(
    node: Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    node.children().filter(move |candidate| {
        candidate.is_element()
            && candidate.tag_name().namespace() == Some(namespace)
            && candidate.tag_name().name() == name
    })
}

pub(crate) fn is_nil(node: Node<'_, '_>) -> bool {
    node.attribute(("http://www.w3.org/2001/XMLSchema-instance", "nil"))
        .is_some_and(|value| value == "true" || value == "1")
}

pub(crate) fn optional_text(
    node: Node<'_, '_>,
    namespace: &str,
    name: &str,
) -> Result<Option<String>> {
    let Some(element) = child(node, namespace, name)? else {
        return Ok(None);
    };
    let content = simple_content(element, name)?;
    if is_nil(element) {
        if !content.is_empty() {
            return Err(Error::invalid_response(format!(
                "nil XML element {name} contains text"
            )));
        }
        return Ok(None);
    }
    Ok(Some(content))
}

pub(crate) fn required_text(node: Node<'_, '_>, namespace: &str, name: &str) -> Result<String> {
    optional_text(node, namespace, name)?
        .filter(|text| !text.is_empty())
        .ok_or_else(|| Error::invalid_response(format!("required XML element {name} is missing")))
}

pub(crate) fn required_text_or_empty(
    node: Node<'_, '_>,
    namespace: &str,
    name: &str,
) -> Result<String> {
    let element = child(node, namespace, name)?.ok_or_else(|| {
        Error::invalid_response(format!("required XML element {name} is missing"))
    })?;
    if is_nil(element) {
        return Err(Error::invalid_response(format!(
            "required XML element {name} is nil"
        )));
    }
    simple_content(element, name)
}

fn simple_content(node: Node<'_, '_>, name: &str) -> Result<String> {
    let mut content = String::new();
    for child in node.children() {
        if child.is_element() {
            return Err(Error::invalid_response(format!(
                "XML element {name} contains a nested element"
            )));
        }
        if child.is_text()
            && let Some(text) = child.text()
        {
            content.push_str(text);
        }
    }
    Ok(content)
}

pub(crate) fn status(node: Node<'_, '_>, prefix: &str) -> Result<Status> {
    let status_name = format!("{prefix}Status");
    let code_name = format!("{prefix}StatusCode");
    let message_name = format!("{prefix}StatusMessage");
    let reference_name = format!("{prefix}StatusRefNumber");
    let status = child(node, ISDS_NS, &status_name)?.ok_or_else(|| {
        Error::invalid_response(format!("required XML element {status_name} is missing"))
    })?;

    Ok(Status {
        code: required_text(status, ISDS_NS, &code_name)?,
        message: optional_text(status, ISDS_NS, &message_name)?.unwrap_or_default(),
        reference_number: optional_text(status, ISDS_NS, &reference_name)?,
    })
}

fn descendant_text(node: Node<'_, '_>, name: &str) -> Result<Option<String>> {
    node.descendants()
        .find(|candidate| candidate.is_element() && candidate.tag_name().name() == name)
        .map(|candidate| simple_content(candidate, name))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation_response(contents: &str) -> String {
        format!(
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
              <soap:Body><CheckDataBoxResponse xmlns="http://isds.czechpoint.cz/v20"
                xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">{contents}</CheckDataBoxResponse>
              </soap:Body></soap:Envelope>"#
        )
    }

    #[test]
    fn text_is_escaped_without_string_interpolation() {
        let request = request("CheckDataBox", |writer| {
            text_element(writer, "dbID", "Příliš & <žluťoučký>")
        })
        .unwrap();
        let xml = String::from_utf8(request).unwrap();
        assert!(xml.contains("Příliš &amp; &lt;žluťoučký&gt;"));
    }

    #[test]
    fn simple_content_concatenates_text_and_preserves_nil_and_empty_semantics() {
        let xml = operation_response(
            r#"<value>first<!-- ignored --><![CDATA[+second]]><?test ignored?>+third</value>
              <nil xsi:nil="true"/><empty/>"#,
        );
        let (value, nil, missing, empty) =
            decode(xml.as_bytes(), "CheckDataBoxResponse", |response| {
                Ok((
                    required_text(response, ISDS_NS, "value")?,
                    optional_text(response, ISDS_NS, "nil")?,
                    optional_text(response, ISDS_NS, "missing")?,
                    required_text_or_empty(response, ISDS_NS, "empty")?,
                ))
            })
            .unwrap();
        assert_eq!(value, "first+second+third");
        assert_eq!(nil, None);
        assert_eq!(missing, None);
        assert_eq!(empty, "");
    }

    #[test]
    fn simple_content_rejects_nested_elements_and_duplicate_scalars() {
        let nested = operation_response("<value>before<nested/>after</value>");
        let error = decode(nested.as_bytes(), "CheckDataBoxResponse", |response| {
            required_text(response, ISDS_NS, "value").map(|_| ())
        })
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));

        let duplicate = operation_response("<value>first</value><value>second</value>");
        let error = decode(duplicate.as_bytes(), "CheckDataBoxResponse", |response| {
            optional_text(response, ISDS_NS, "value").map(|_| ())
        })
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));

        for invalid_nil in [
            r#"<value xsi:nil="true">text</value>"#,
            r#"<value xsi:nil="true"><nested/></value>"#,
        ] {
            let xml = operation_response(invalid_nil);
            let error = decode(xml.as_bytes(), "CheckDataBoxResponse", |response| {
                optional_text(response, ISDS_NS, "value").map(|_| ())
            })
            .unwrap_err();
            assert!(matches!(error, Error::InvalidResponse { .. }));
        }
    }

    #[test]
    fn duplicate_bodies_responses_and_conflicting_statuses_are_rejected() {
        let duplicate_body =
            br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><CheckDataBoxResponse xmlns="http://isds.czechpoint.cz/v20"/></soap:Body>
          <soap:Body><CheckDataBoxResponse xmlns="http://isds.czechpoint.cz/v20"/></soap:Body>
        </soap:Envelope>"#;
        let error = decode(duplicate_body, "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));

        let duplicate_response = operation_response("").replace(
            "</CheckDataBoxResponse>",
            "</CheckDataBoxResponse><CheckDataBoxResponse xmlns=\"http://isds.czechpoint.cz/v20\"/>",
        );
        let error = decode(
            duplicate_response.as_bytes(),
            "CheckDataBoxResponse",
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));

        let conflicting_status = operation_response(
            r#"<dbStatus><dbStatusCode>0000</dbStatusCode></dbStatus>
              <dbStatus><dbStatusCode>1000</dbStatusCode></dbStatus>"#,
        );
        let error = decode(
            conflicting_status.as_bytes(),
            "CheckDataBoxResponse",
            |response| {
                status(response, "db")?.into_result()?;
                Ok(())
            },
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }

    #[test]
    fn document_type_definitions_are_rejected() {
        let xml = br#"<!DOCTYPE soap:Envelope [<!ENTITY secret SYSTEM "file:///etc/passwd">]>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
              <soap:Body />
            </soap:Envelope>"#;
        let error = decode(xml, "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }

    #[test]
    fn parser_limits_reject_excessive_depth_and_node_counts() {
        let mut deep = String::from(
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body>"#,
        );
        deep.push_str(&"<x>".repeat(MAX_XML_DEPTH as usize));
        let error = decode(deep.as_bytes(), "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidResponse { ref message } if message == "XML response exceeds the nesting limit"
        ));

        let mut wide = String::from(
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body>"#,
        );
        wide.push_str(&"<x/>".repeat(MAX_XML_NODES as usize));
        let error = decode(wide.as_bytes(), "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidResponse { ref message } if message == "XML response exceeds the node limit"
        ));

        let mut attributes = String::from(
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"><soap:Body>"#,
        );
        attributes.push_str("<x");
        for index in 0..MAX_XML_NODES {
            attributes.push_str(&format!(" a{index}=\"\""));
        }
        attributes.push_str("/></soap:Body></soap:Envelope>");
        let error = decode(attributes.as_bytes(), "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidResponse { ref message } if message == "XML response exceeds the node limit"
        ));
    }

    #[test]
    fn parser_errors_do_not_expose_response_tokens() {
        let mismatched = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><SENSITIVE_RESPONSE_TOKEN></soap:Body></soap:Envelope>"#;
        let unbound_prefix = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><secret:CheckDataBoxResponse>SENSITIVE_RESPONSE_TOKEN</secret:CheckDataBoxResponse>
          </soap:Body></soap:Envelope>"#;
        for xml in [mismatched.as_slice(), unbound_prefix.as_slice()] {
            let error = decode(xml, "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
            let Error::InvalidResponse { message } = error else {
                panic!("expected invalid response");
            };
            assert_eq!(message, "malformed XML response");
            assert!(!message.contains("SENSITIVE_RESPONSE_TOKEN"));
        }
    }

    #[test]
    fn soap_12_uses_its_own_envelope_and_fault_shape() {
        let request =
            request_with_version(SoapVersion::Soap12, "CreateBigMessage", |_| Ok(())).unwrap();
        let xml = String::from_utf8(request).unwrap();
        assert!(xml.contains("xmlns:soap=\"http://www.w3.org/2003/05/soap-envelope\""));

        let fault = br#"<env:Envelope xmlns:env="http://www.w3.org/2003/05/soap-envelope">
          <env:Body><env:Fault><env:Code><env:Value>env:<!-- split -->Receiver</env:Value></env:Code>
            <env:Reason><env:Text xml:lang="en">Temporary<!-- split --> <![CDATA[failure]]></env:Text></env:Reason>
          </env:Fault></env:Body></env:Envelope>"#;
        let error = decode(fault, "CreateBigMessageResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            Error::SoapFault { ref code, ref message }
                if code == "env:Receiver" && message == "Temporary failure"
        ));
    }
}
