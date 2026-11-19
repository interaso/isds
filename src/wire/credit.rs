use jiff::Timestamp;
use jiff::civil::Date;
use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{
    CreditEvent, CreditEventDetails, CreditInfo, CreditQuery, DataBoxId, DataVaultChange,
};
use crate::wire::soap::{self, ISDS_NS};

pub(crate) fn credit_info_request(query: &CreditQuery) -> Result<Vec<u8>> {
    soap::request("DataBoxCreditInfo", |writer| {
        soap::text_element(writer, "dbID", query.id().as_str())?;
        soap::nillable_text_element(
            writer,
            "ciFromDate",
            query.from().map(|value| value.to_string()).as_deref(),
        )?;
        // The lowercase `d` is intentional and required by the official XSD.
        soap::nillable_text_element(
            writer,
            "ciTodate",
            query.to().map(|value| value.to_string()).as_deref(),
        )?;
        Ok(())
    })
}

pub(crate) fn decode_credit_info(response: Node<'_, '_>) -> Result<CreditInfo> {
    soap::status(response, "db")?.into_result()?;
    let current_credit = soap::optional_text(response, ISDS_NS, "currentCredit")
        .map(|value| soap::parse(value, "currentCredit"))
        .transpose()?;
    let notification_email =
        soap::optional_text(response, ISDS_NS, "notifEmail").map(str::to_owned);
    let mut events = Vec::new();

    if let Some(records) = soap::child(response, ISDS_NS, "ciRecords") {
        for record in soap::children(records, ISDS_NS, "ciRecord") {
            if !soap::is_nil(record) {
                events.push(parse_credit_event(record)?);
            }
        }
    }

    Ok(CreditInfo {
        current_credit,
        notification_email,
        events,
    })
}

fn parse_credit_event(node: Node<'_, '_>) -> Result<CreditEvent> {
    let time = soap::parse::<Timestamp>(
        soap::required_text(node, ISDS_NS, "ciEventTime")?,
        "ciEventTime",
    )?;
    let event_type = required_parsed::<i64>(node, "ciEventType")?;
    let credit_change = required_parsed::<i64>(node, "ciCreditChange")?;
    let credit_after = required_parsed::<i64>(node, "ciCreditAfter")?;
    let details = parse_event_details(node)?;

    Ok(CreditEvent {
        time,
        event_type,
        credit_change,
        credit_after,
        details,
    })
}

fn parse_event_details(node: Node<'_, '_>) -> Result<CreditEventDetails> {
    if let Some(transaction_id) = soap::optional_text(node, ISDS_NS, "ciTransID") {
        return Ok(CreditEventDetails::Transaction {
            transaction_id: transaction_id.to_owned(),
        });
    }

    if soap::child(node, ISDS_NS, "ciRecipientID").is_some()
        || soap::child(node, ISDS_NS, "ciPDZID").is_some()
    {
        let recipient = soap::required_text(node, ISDS_NS, "ciRecipientID")?;
        let recipient_id = DataBoxId::new(recipient)
            .map_err(|_| Error::invalid_response("ciRecipientID is not a valid databox ID"))?;
        let postal_message_id = soap::required_text(node, ISDS_NS, "ciPDZID")?.to_owned();
        return Ok(CreditEventDetails::PostalMessage {
            recipient_id,
            postal_message_id,
        });
    }

    if soap::child(node, ISDS_NS, "ciNewCapacity").is_some() {
        return Ok(CreditEventDetails::DataVault(DataVaultChange {
            new_capacity: required_parsed(node, "ciNewCapacity")?,
            new_from: required_parsed::<Date>(node, "ciNewFrom")?,
            new_to: required_parsed::<Date>(node, "ciNewTo")?,
            old_capacity: optional_parsed(node, "ciOldCapacity")?,
            old_from: optional_parsed(node, "ciOldFrom")?,
            old_to: optional_parsed(node, "ciOldTo")?,
            done_by: soap::optional_text(node, ISDS_NS, "ciDoneBy").map(str::to_owned),
        }));
    }

    if let Some(done_by) = soap::optional_text(node, ISDS_NS, "ciDoneBy") {
        return Ok(CreditEventDetails::Administrative {
            done_by: done_by.to_owned(),
        });
    }

    Ok(CreditEventDetails::Unknown)
}

fn required_parsed<T: std::str::FromStr>(node: Node<'_, '_>, name: &str) -> Result<T> {
    soap::parse(soap::required_text(node, ISDS_NS, name)?, name)
}

fn optional_parsed<T: std::str::FromStr>(node: Node<'_, '_>, name: &str) -> Result<Option<T>> {
    soap::optional_text(node, ISDS_NS, name)
        .map(|value| soap::parse(value, name))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> CreditQuery {
        CreditQuery::current(DataBoxId::new("egtk37z").unwrap())
    }

    #[test]
    fn current_credit_request_uses_nil_dates_and_exact_todate_spelling() {
        let xml = String::from_utf8(credit_info_request(&query()).unwrap()).unwrap();
        assert!(xml.contains("<dbID>egtk37z</dbID>"));
        assert!(xml.contains("<ciFromDate xsi:nil=\"true\"/>"));
        assert!(xml.contains("<ciTodate xsi:nil=\"true\"/>"));
        assert!(!xml.contains("ciToDate"));
    }

    #[test]
    fn parses_current_credit_without_inventing_history() {
        let xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><DataBoxCreditInfoResponse xmlns="http://isds.czechpoint.cz/v20">
            <currentCredit>12345</currentCredit><notifEmail>credit@example.test</notifEmail>
            <dbStatus><dbStatusCode>0000</dbStatusCode><dbStatusMessage>OK</dbStatusMessage></dbStatus>
          </DataBoxCreditInfoResponse></soap:Body></soap:Envelope>"#;
        let info = soap::decode(xml, "DataBoxCreditInfoResponse", decode_credit_info).unwrap();
        assert_eq!(info.current_credit, Some(12_345));
        assert_eq!(
            info.notification_email.as_deref(),
            Some("credit@example.test")
        );
        assert!(info.events.is_empty());
    }

    #[test]
    fn parses_transaction_and_data_vault_history_branches() {
        let xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><DataBoxCreditInfoResponse xmlns="http://isds.czechpoint.cz/v20"
            xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
            <currentCredit>9000</currentCredit><ciRecords>
              <ciRecord><ciEventTime>2026-11-19T12:30:00+01:00</ciEventTime>
                <ciEventType>1</ciEventType><ciCreditChange>10000</ciCreditChange>
                <ciCreditAfter>10000</ciCreditAfter><ciTransID>transaction-1</ciTransID></ciRecord>
              <ciRecord><ciEventTime>2026-11-19T13:30:00+01:00</ciEventTime>
                <ciEventType>4</ciEventType><ciCreditChange>-1000</ciCreditChange>
                <ciCreditAfter>9000</ciCreditAfter><ciNewCapacity>20</ciNewCapacity>
                <ciNewFrom>2026-12-01</ciNewFrom><ciNewTo>2027-12-01</ciNewTo>
                <ciOldCapacity>10</ciOldCapacity><ciOldFrom xsi:nil="true"/>
                <ciOldTo xsi:nil="true"/><ciDoneBy>synthetic-admin</ciDoneBy></ciRecord>
            </ciRecords>
            <dbStatus><dbStatusCode>0000</dbStatusCode><dbStatusMessage>OK</dbStatusMessage></dbStatus>
          </DataBoxCreditInfoResponse></soap:Body></soap:Envelope>"#;
        let info = soap::decode(xml, "DataBoxCreditInfoResponse", decode_credit_info).unwrap();
        assert_eq!(info.events.len(), 2);
        assert!(matches!(
            &info.events[0].details,
            CreditEventDetails::Transaction { transaction_id } if transaction_id == "transaction-1"
        ));
        assert_eq!(info.events[0].time.to_string(), "2026-11-19T11:30:00Z");
        let CreditEventDetails::DataVault(change) = &info.events[1].details else {
            panic!("expected data vault change");
        };
        assert_eq!(change.new_capacity, 20);
        assert_eq!(change.old_capacity, Some(10));
        assert_eq!(change.old_from, None);
    }

    #[test]
    fn history_query_rejects_reversed_and_overlong_intervals() {
        let id = DataBoxId::new("egtk37z").unwrap();
        let from = Date::constant(2026, 1, 1);
        assert!(CreditQuery::history(id.clone(), from, Date::constant(2025, 12, 31)).is_err());
        assert!(CreditQuery::history(id.clone(), from, Date::constant(2027, 1, 1)).is_ok());
        assert!(CreditQuery::history(id, from, Date::constant(2027, 1, 2)).is_err());
    }
}
