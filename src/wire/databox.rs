use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{DataBoxId, DataBoxState};
use crate::wire::soap::{self, ISDS_NS};

pub(crate) fn check_data_box_request(id: &DataBoxId) -> Result<Vec<u8>> {
    soap::request("CheckDataBox", |writer| {
        soap::text_element(writer, "dbID", id.as_str())
    })
}

pub(crate) fn decode_check_data_box(response: Node<'_, '_>) -> Result<DataBoxState> {
    soap::status(response, "db")?.into_result()?;
    let state = soap::required_text(response, ISDS_NS, "dbState")?
        .parse::<i32>()
        .map_err(|_| Error::invalid_response("dbState is not a signed 32-bit integer"))?;
    Ok(DataBoxState::from_code(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_uses_the_isds_namespace() {
        let id = DataBoxId::new("egtk37z").unwrap();
        let xml = String::from_utf8(check_data_box_request(&id).unwrap()).unwrap();
        assert!(xml.contains("xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\""));
        assert!(xml.contains("xmlns=\"http://isds.czechpoint.cz/v20\""));
        assert!(xml.contains("<CheckDataBox "));
        assert!(xml.contains("<dbID>egtk37z</dbID>"));
    }

    #[test]
    fn response_accepts_different_namespace_prefixes() {
        let xml = br#"<?xml version="1.0"?>
            <s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
              <s:Body>
                <i:CheckDataBoxResponse xmlns:i="http://isds.czechpoint.cz/v20">
                  <i:dbState>1</i:dbState>
                  <i:dbStatus>
                    <i:dbStatusCode>0000</i:dbStatusCode>
                    <i:dbStatusMessage>OK</i:dbStatusMessage>
                  </i:dbStatus>
                </i:CheckDataBoxResponse>
              </s:Body>
            </s:Envelope>"#;
        let state = soap::decode(xml, "CheckDataBoxResponse", decode_check_data_box).unwrap();
        assert!(state.is_active());
    }
}
