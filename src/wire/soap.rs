use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use roxmltree::{Document, Node, ParsingOptions};

use crate::error::{Error, Result, ServiceError};

pub(crate) const ISDS_NS: &str = "http://isds.czechpoint.cz/v20";
const SOAP_NS: &str = "http://schemas.xmlsoap.org/soap/envelope/";

pub(crate) type XmlResult = std::result::Result<(), quick_xml::Error>;

pub(crate) fn request(
    operation: &str,
    write_operation: impl FnOnce(&mut Writer<Vec<u8>>) -> XmlResult,
) -> Result<Vec<u8>> {
    let mut writer = Writer::new(Vec::new());
    write_envelope(&mut writer, operation, write_operation).map_err(|error| {
        Error::invalid_input("xml", format!("could not encode request: {error}"))
    })?;
    Ok(writer.into_inner())
}

fn write_envelope(
    writer: &mut Writer<Vec<u8>>,
    operation: &str,
    write_operation: impl FnOnce(&mut Writer<Vec<u8>>) -> XmlResult,
) -> XmlResult {
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;

    let mut envelope = BytesStart::new("soap:Envelope");
    envelope.push_attribute(("xmlns:soap", SOAP_NS));
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
    let options = ParsingOptions {
        allow_dtd: false,
        ..ParsingOptions::default()
    };
    let document = Document::parse_with_options(xml, options)
        .map_err(|error| Error::invalid_response(format!("malformed XML: {error}")))?;

    let envelope = document.root_element();
    require_tag(envelope, SOAP_NS, "Envelope")?;
    let body = child(envelope, SOAP_NS, "Body")
        .ok_or_else(|| Error::invalid_response("SOAP Body is missing"))?;

    if let Some(fault) = child(body, SOAP_NS, "Fault") {
        let code = descendant_text(fault, "faultcode").unwrap_or("unknown");
        let message = descendant_text(fault, "faultstring").unwrap_or("unspecified SOAP fault");
        return Err(Error::SoapFault {
            code: code.to_owned(),
            message: message.to_owned(),
        });
    }

    let response = child(body, ISDS_NS, operation).ok_or_else(|| {
        Error::invalid_response(format!("SOAP Body does not contain {operation}"))
    })?;
    decode_response(response)
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
) -> Option<Node<'a, 'input>> {
    node.children().find(|candidate| {
        candidate.is_element()
            && candidate.tag_name().namespace() == Some(namespace)
            && candidate.tag_name().name() == name
    })
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

pub(crate) fn optional_text<'a, 'input>(
    node: Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> Option<&'a str> {
    child(node, namespace, name).and_then(|child| if is_nil(child) { None } else { child.text() })
}

pub(crate) fn required_text<'a, 'input>(
    node: Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> Result<&'a str> {
    optional_text(node, namespace, name)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| Error::invalid_response(format!("required XML element {name} is missing")))
}

pub(crate) fn required_text_or_empty<'a, 'input>(
    node: Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> Result<&'a str> {
    let element = child(node, namespace, name).ok_or_else(|| {
        Error::invalid_response(format!("required XML element {name} is missing"))
    })?;
    if is_nil(element) {
        return Err(Error::invalid_response(format!(
            "required XML element {name} is nil"
        )));
    }
    Ok(element.text().unwrap_or(""))
}

pub(crate) fn status(node: Node<'_, '_>, prefix: &str) -> Result<Status> {
    let status_name = format!("{prefix}Status");
    let code_name = format!("{prefix}StatusCode");
    let message_name = format!("{prefix}StatusMessage");
    let reference_name = format!("{prefix}StatusRefNumber");
    let status = child(node, ISDS_NS, &status_name).ok_or_else(|| {
        Error::invalid_response(format!("required XML element {status_name} is missing"))
    })?;

    Ok(Status {
        code: required_text(status, ISDS_NS, &code_name)?.to_owned(),
        message: optional_text(status, ISDS_NS, &message_name)
            .unwrap_or_default()
            .to_owned(),
        reference_number: optional_text(status, ISDS_NS, &reference_name).map(str::to_owned),
    })
}

fn descendant_text<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    node.descendants()
        .find(|candidate| candidate.is_element() && candidate.tag_name().name() == name)
        .and_then(|candidate| candidate.text())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn document_type_definitions_are_rejected() {
        let xml = br#"<!DOCTYPE soap:Envelope [<!ENTITY secret SYSTEM "file:///etc/passwd">]>
            <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
              <soap:Body />
            </soap:Envelope>"#;
        let error = decode(xml, "CheckDataBoxResponse", |_| Ok(())).unwrap_err();
        assert!(matches!(error, Error::InvalidResponse { .. }));
    }
}
