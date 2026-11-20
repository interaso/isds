use jiff::civil::Date;
use quick_xml::Writer;
use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{DataBox, DataBoxId, DataBoxSearchResult, DataBoxState, DataBoxType};
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

pub(crate) fn owner_info_request() -> Result<Vec<u8>> {
    soap::request("GetOwnerInfoFromLogin2", |writer| {
        soap::text_element(writer, "dbDummy", "")
    })
}

pub(crate) fn decode_owner_info(response: Node<'_, '_>) -> Result<DataBox> {
    soap::status(response, "db")?.into_result()?;
    let owner = soap::child(response, ISDS_NS, "dbOwnerInfo")?
        .ok_or_else(|| Error::invalid_response("dbOwnerInfo is missing"))?;
    parse_data_box(owner)
}

pub(crate) fn find_data_boxes_request(query: &DataBox) -> Result<Vec<u8>> {
    if query
        .box_type
        .as_ref()
        .is_some_and(|value| value.as_str().is_empty())
    {
        return Err(Error::invalid_input(
            "box_type",
            "databox type must not be empty",
        ));
    }
    soap::request("FindDataBox2", |writer| {
        soap::element(writer, "dbOwnerInfo", |writer| {
            write_data_box(writer, query)
        })
    })
}

pub(crate) fn decode_find_data_boxes(response: Node<'_, '_>) -> Result<DataBoxSearchResult> {
    let status = soap::status(response, "db")?;
    let (truncated, postal_delivery_restricted) = match status.code.as_str() {
        "0000" => (false, false),
        "0002" => {
            return Ok(DataBoxSearchResult::default());
        }
        "0003" => (true, false),
        "0009" => (false, true),
        _ => return Err(status.into_error()),
    };

    let mut boxes = Vec::new();
    if let Some(results) = soap::child(response, ISDS_NS, "dbResults")? {
        for owner in soap::children(results, ISDS_NS, "dbOwnerInfo") {
            boxes.push(parse_data_box(owner)?);
        }
    }
    Ok(DataBoxSearchResult {
        boxes,
        truncated,
        postal_delivery_restricted,
    })
}

fn write_data_box(writer: &mut Writer<Vec<u8>>, data: &DataBox) -> soap::XmlResult {
    soap::nillable_text_element(writer, "dbID", data.id.as_ref().map(DataBoxId::as_str))?;
    if let Some(value) = data.identified_in_population_register {
        soap::text_element(writer, "aifoIsds", soap::xml_bool(value))?;
    }
    soap::nillable_text_element(
        writer,
        "dbType",
        data.box_type.as_ref().map(DataBoxType::as_str),
    )?;
    soap::nillable_text_element(writer, "ic", data.identification_number.as_deref())?;
    soap::nillable_text_element(writer, "pnGivenNames", data.given_names.as_deref())?;
    soap::nillable_text_element(writer, "pnLastName", data.last_name.as_deref())?;
    soap::nillable_text_element(writer, "firmName", data.firm_name.as_deref())?;
    soap::nillable_text_element(
        writer,
        "biDate",
        data.birth_date.map(|value| value.to_string()).as_deref(),
    )?;
    soap::nillable_text_element(writer, "biCity", data.birth_city.as_deref())?;
    soap::nillable_text_element(writer, "biCounty", data.birth_county.as_deref())?;
    soap::nillable_text_element(writer, "biState", data.birth_state.as_deref())?;
    soap::nillable_text_element(writer, "adCode", data.address_code.as_deref())?;
    soap::nillable_text_element(writer, "adCity", data.address_city.as_deref())?;
    soap::nillable_text_element(writer, "adDistrict", data.address_district.as_deref())?;
    soap::nillable_text_element(writer, "adStreet", data.address_street.as_deref())?;
    soap::nillable_text_element(
        writer,
        "adNumberInStreet",
        data.address_number_in_street.as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "adNumberInMunicipality",
        data.address_number_in_municipality.as_deref(),
    )?;
    soap::nillable_text_element(writer, "adZipCode", data.address_zip_code.as_deref())?;
    soap::nillable_text_element(writer, "adState", data.address_state.as_deref())?;
    soap::nillable_text_element(writer, "nationality", data.nationality.as_deref())?;
    soap::nillable_text_element(writer, "dbIdOVM", data.public_authority_id.as_deref())?;
    soap::nillable_text_element(
        writer,
        "dbState",
        data.state.map(|value| value.to_string()).as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dbOpenAddressing",
        data.open_addressing
            .map(|value| soap::xml_bool(value).to_owned())
            .as_deref(),
    )?;
    soap::nillable_text_element(
        writer,
        "dbUpperID",
        data.upper_id.as_ref().map(DataBoxId::as_str),
    )?;
    Ok(())
}

fn parse_data_box(node: Node<'_, '_>) -> Result<DataBox> {
    Ok(DataBox {
        id: optional_id(node, "dbID")?,
        identified_in_population_register: optional_bool(node, "aifoIsds")?,
        box_type: soap::optional_text(node, ISDS_NS, "dbType")?
            .map(|value| {
                DataBoxType::new(value)
                    .map_err(|_| Error::invalid_response("dbType has an invalid value"))
            })
            .transpose()?,
        identification_number: optional_owned(node, "ic")?,
        given_names: optional_owned(node, "pnGivenNames")?,
        last_name: optional_owned(node, "pnLastName")?,
        firm_name: optional_owned(node, "firmName")?,
        birth_date: optional_parsed::<Date>(node, "biDate")?,
        birth_city: optional_owned(node, "biCity")?,
        birth_county: optional_owned(node, "biCounty")?,
        birth_state: optional_owned(node, "biState")?,
        address_code: optional_owned(node, "adCode")?,
        address_city: optional_owned(node, "adCity")?,
        address_district: optional_owned(node, "adDistrict")?,
        address_street: optional_owned(node, "adStreet")?,
        address_number_in_street: optional_owned(node, "adNumberInStreet")?,
        address_number_in_municipality: optional_owned(node, "adNumberInMunicipality")?,
        address_zip_code: optional_owned(node, "adZipCode")?,
        address_state: optional_owned(node, "adState")?,
        nationality: optional_owned(node, "nationality")?,
        public_authority_id: optional_owned(node, "dbIdOVM")?,
        state: optional_parsed::<i64>(node, "dbState")?,
        open_addressing: optional_bool(node, "dbOpenAddressing")?,
        upper_id: optional_id(node, "dbUpperID")?,
    })
}

fn optional_owned(node: Node<'_, '_>, name: &str) -> Result<Option<String>> {
    soap::optional_text(node, ISDS_NS, name)
}

fn optional_parsed<T: std::str::FromStr>(node: Node<'_, '_>, name: &str) -> Result<Option<T>> {
    soap::optional_text(node, ISDS_NS, name)?
        .map(|value| soap::parse(&value, name))
        .transpose()
}

fn optional_bool(node: Node<'_, '_>, name: &str) -> Result<Option<bool>> {
    soap::optional_text(node, ISDS_NS, name)?
        .map(|value| soap::parse_bool(&value, name))
        .transpose()
}

fn optional_id(node: Node<'_, '_>, name: &str) -> Result<Option<DataBoxId>> {
    soap::optional_text(node, ISDS_NS, name)?
        .map(|value| {
            DataBoxId::new(value)
                .map_err(|_| Error::invalid_response(format!("{name} is not a valid databox ID")))
        })
        .transpose()
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

    #[test]
    fn exact_lookup_emits_all_required_nillable_fields() {
        let query = DataBox::by_id(DataBoxId::new("egtk37z").unwrap());
        let xml = String::from_utf8(find_data_boxes_request(&query).unwrap()).unwrap();
        assert!(xml.contains("<dbID>egtk37z</dbID>"));
        assert!(xml.contains("<dbType xsi:nil=\"true\"/>"));
        assert!(xml.contains("<biDate xsi:nil=\"true\"/>"));
        assert!(xml.contains("<dbUpperID xsi:nil=\"true\"/>"));
        assert!(!xml.contains("<aifoIsds"));
        let db_type = xml.find("<dbType").unwrap();
        let firm_name = xml.find("<firmName").unwrap();
        let upper_id = xml.find("<dbUpperID").unwrap();
        assert!(db_type < firm_name && firm_name < upper_id);
    }

    #[test]
    fn empty_custom_databox_type_is_rejected_but_future_codes_are_preserved() {
        let empty = DataBox {
            box_type: Some(DataBoxType::Other(String::new())),
            ..DataBox::default()
        };
        assert!(matches!(
            find_data_boxes_request(&empty),
            Err(Error::InvalidInput {
                field: "box_type",
                ..
            })
        ));

        let future = DataBox {
            box_type: Some(DataBoxType::Other("FUTURE".to_owned())),
            ..DataBox::default()
        };
        let xml = String::from_utf8(find_data_boxes_request(&future).unwrap()).unwrap();
        assert!(xml.contains("<dbType>FUTURE</dbType>"));
    }

    #[test]
    fn owner_info_handles_nil_and_typed_values() {
        let xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><GetOwnerInfoFromLogin2Response xmlns="http://isds.czechpoint.cz/v20"
            xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
            <dbOwnerInfo>
              <dbID>egtk37z</dbID><dbType>PO</dbType><ic>12345678</ic>
              <pnGivenNames xsi:nil="true"/><pnLastName xsi:nil="true"/>
              <firmName>Synthetic &amp; Test</firmName><biDate xsi:nil="true"/>
              <dbState>1</dbState><dbOpenAddressing>false</dbOpenAddressing>
            </dbOwnerInfo>
            <dbStatus><dbStatusCode>0000</dbStatusCode><dbStatusMessage>OK</dbStatusMessage></dbStatus>
          </GetOwnerInfoFromLogin2Response></soap:Body></soap:Envelope>"#;
        let owner = soap::decode(xml, "GetOwnerInfoFromLogin2Response", decode_owner_info).unwrap();
        assert_eq!(owner.id.unwrap().as_str(), "egtk37z");
        assert_eq!(owner.box_type, Some(DataBoxType::LegalEntity));
        assert_eq!(owner.firm_name.as_deref(), Some("Synthetic & Test"));
        assert_eq!(owner.given_names, None);
        assert_eq!(owner.state, Some(1));
        assert_eq!(owner.open_addressing, Some(false));
    }

    #[test]
    fn lookup_distinguishes_empty_truncated_and_failed_results() {
        let empty = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><FindDataBox2Response xmlns="http://isds.czechpoint.cz/v20">
            <dbStatus><dbStatusCode>0002</dbStatusCode><dbStatusMessage>No result</dbStatusMessage></dbStatus>
          </FindDataBox2Response></soap:Body></soap:Envelope>"#;
        let result = soap::decode(empty, "FindDataBox2Response", decode_find_data_boxes).unwrap();
        assert!(result.boxes.is_empty());
        assert!(!result.truncated);

        let restricted = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><FindDataBox2Response xmlns="http://isds.czechpoint.cz/v20">
            <dbStatus><dbStatusCode>0009</dbStatusCode><dbStatusMessage>PDZ restricted</dbStatusMessage></dbStatus>
          </FindDataBox2Response></soap:Body></soap:Envelope>"#;
        let result =
            soap::decode(restricted, "FindDataBox2Response", decode_find_data_boxes).unwrap();
        assert!(result.boxes.is_empty());
        assert!(result.postal_delivery_restricted);

        let failed = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><FindDataBox2Response xmlns="http://isds.czechpoint.cz/v20">
            <dbStatus><dbStatusCode>1000</dbStatusCode><dbStatusMessage>Denied</dbStatusMessage></dbStatus>
          </FindDataBox2Response></soap:Body></soap:Envelope>"#;
        let error =
            soap::decode(failed, "FindDataBox2Response", decode_find_data_boxes).unwrap_err();
        assert!(matches!(error, Error::Service(ref service) if service.code() == "1000"));
    }
}
