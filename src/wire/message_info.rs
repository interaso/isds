use roxmltree::Node;

use crate::error::{Error, Result};
use crate::model::{
    DeliveryEvent, DeliveryInfo, MessageEnvelopeDetails, MessageId, MessageListQuery, MessagePage,
    MessageRecord,
};
use crate::wire::message_common;
use crate::wire::soap::{self, ISDS_NS};

#[derive(Clone, Copy)]
pub(crate) enum MessageListKind {
    Sent,
    Received,
}

impl MessageListKind {
    fn operation(self) -> &'static str {
        match self {
            Self::Sent => "GetListOfSentMessages",
            Self::Received => "GetListOfReceivedMessages",
        }
    }

    fn organization_unit_element(self) -> &'static str {
        match self {
            Self::Sent => "dmSenderOrgUnitNum",
            Self::Received => "dmRecipientOrgUnitNum",
        }
    }
}

pub(crate) fn list_request(kind: MessageListKind, query: &MessageListQuery) -> Result<Vec<u8>> {
    validate_query(query)?;
    let status_filter = status_filter(query)?;
    soap::request(kind.operation(), |writer| {
        soap::nillable_text_element(
            writer,
            "dmFromTime",
            query.from.map(|value| value.to_string()).as_deref(),
        )?;
        soap::nillable_text_element(
            writer,
            "dmToTime",
            query.to.map(|value| value.to_string()).as_deref(),
        )?;
        soap::nillable_text_element(
            writer,
            kind.organization_unit_element(),
            query
                .organization_unit_number
                .map(|value| value.to_string())
                .as_deref(),
        )?;
        soap::text_element(writer, "dmStatusFilter", &status_filter)?;
        soap::text_element(writer, "dmOffset", &query.offset.to_string())?;
        soap::text_element(writer, "dmLimit", &query.limit.to_string())?;
        Ok(())
    })
}

pub(crate) fn decode_list(response: Node<'_, '_>, query: &MessageListQuery) -> Result<MessagePage> {
    soap::status(response, "dm")?.into_result()?;
    let mut records = Vec::new();
    if let Some(array) = soap::child(response, ISDS_NS, "dmRecords") {
        for record in soap::children(array, ISDS_NS, "dmRecord") {
            records.push(parse_record(record)?);
        }
    }
    Ok(MessagePage {
        records,
        offset: query.offset,
        limit: query.limit,
    })
}

pub(crate) fn delivery_info_request(id: &MessageId) -> Result<Vec<u8>> {
    soap::request("GetDeliveryInfo", |writer| {
        soap::text_element(writer, "dmID", id.as_str())
    })
}

pub(crate) fn decode_delivery_info(response: Node<'_, '_>) -> Result<DeliveryInfo> {
    if soap::child(response, ISDS_NS, "dmStatus").is_some() {
        soap::status(response, "dm")?.into_result()?;
    }
    let delivery = soap::child(response, ISDS_NS, "dmDelivery")
        .filter(|node| !soap::is_nil(*node))
        .ok_or_else(|| Error::invalid_response("dmDelivery is missing"))?;
    let contents = soap::child(delivery, ISDS_NS, "dmDm")
        .ok_or_else(|| Error::invalid_response("dmDelivery/dmDm is missing"))?;
    let envelope = message_common::parse_envelope(contents)?;
    let hash = message_common::parse_hash(delivery)?;
    let qualified_timestamp = message_common::decode_base64(
        soap::required_text_or_empty(delivery, ISDS_NS, "dmQTimestamp")?,
        "dmQTimestamp",
    )?;
    let delivery_time = message_common::optional_timestamp(delivery, "dmDeliveryTime")?;
    let acceptance_time = message_common::optional_timestamp(delivery, "dmAcceptanceTime")?;
    let state = message_common::required_state(delivery)?;
    let events_node = soap::child(delivery, ISDS_NS, "dmEvents")
        .ok_or_else(|| Error::invalid_response("dmEvents is missing"))?;
    let mut events = Vec::new();
    for event in soap::children(events_node, ISDS_NS, "dmEvent") {
        events.push(DeliveryEvent {
            time: message_common::optional_timestamp(event, "dmEventTime")?,
            description: soap::optional_text(event, ISDS_NS, "dmEventDescr").map(str::to_owned),
        });
    }
    if events.is_empty() {
        return Err(Error::invalid_response("dmEvents contains no events"));
    }

    Ok(DeliveryInfo {
        envelope,
        hash,
        qualified_timestamp,
        delivery_time,
        acceptance_time,
        state,
        events,
    })
}

pub(crate) fn envelope_download_request(id: &MessageId) -> Result<Vec<u8>> {
    soap::request("MessageEnvelopeDownload", |writer| {
        soap::text_element(writer, "dmID", id.as_str())
    })
}

pub(crate) fn decode_envelope_download(response: Node<'_, '_>) -> Result<MessageEnvelopeDetails> {
    if soap::child(response, ISDS_NS, "dmStatus").is_some() {
        soap::status(response, "dm")?.into_result()?;
    }
    let returned = soap::child(response, ISDS_NS, "dmReturnedMessageEnvelope")
        .filter(|node| !soap::is_nil(*node))
        .ok_or_else(|| Error::invalid_response("dmReturnedMessageEnvelope is missing"))?;
    let contents = soap::child(returned, ISDS_NS, "dmDm")
        .ok_or_else(|| Error::invalid_response("dmReturnedMessageEnvelope/dmDm is missing"))?;
    let envelope = message_common::parse_envelope(contents)?;
    let hash = message_common::parse_hash(returned)?;
    let qualified_timestamp = message_common::decode_base64(
        soap::required_text_or_empty(returned, ISDS_NS, "dmQTimestamp")?,
        "dmQTimestamp",
    )?;
    let delivery_time = message_common::optional_timestamp(returned, "dmDeliveryTime")?;
    let acceptance_time = message_common::optional_timestamp(returned, "dmAcceptanceTime")?;
    let state = message_common::required_state(returned)?;
    let attachment_size = message_common::optional_i64(returned, "dmAttachmentSize")?;
    let message_type = returned.attribute("dmType").map(str::to_owned);
    let attachment_count = returned
        .attribute("attsNum")
        .map(|value| soap::parse(value, "attsNum"))
        .transpose()?;
    let special_message_flag = returned
        .attribute("specMessFlag")
        .map(|value| soap::parse(value, "specMessFlag"))
        .transpose()?;

    Ok(MessageEnvelopeDetails {
        envelope,
        hash,
        qualified_timestamp,
        delivery_time,
        acceptance_time,
        state,
        attachment_size,
        message_type,
        mode: message_common::optional_message_mode(returned)?,
        attachment_count,
        special_message_flag,
    })
}

pub(crate) fn mark_downloaded_request(id: &MessageId) -> Result<Vec<u8>> {
    soap::request("MarkMessageAsDownloaded", |writer| {
        soap::text_element(writer, "dmID", id.as_str())
    })
}

pub(crate) fn decode_mark_downloaded(response: Node<'_, '_>) -> Result<()> {
    soap::status(response, "dm")?.into_result()?;
    Ok(())
}

fn validate_query(query: &MessageListQuery) -> Result<()> {
    if query.offset == 0 {
        return Err(Error::invalid_input(
            "offset",
            "message-list offset is one-based and must be at least 1",
        ));
    }
    if query.limit == 0 {
        return Err(Error::invalid_input(
            "limit",
            "message-list limit must be at least 1",
        ));
    }
    if query.from.zip(query.to).is_some_and(|(from, to)| from > to) {
        return Err(Error::invalid_input(
            "time_range",
            "the delivery-time upper bound must not precede the lower bound",
        ));
    }
    Ok(())
}

fn status_filter(query: &MessageListQuery) -> Result<String> {
    let Some(states) = &query.states else {
        return Ok("-1".to_owned());
    };
    let mut mask = 0_u128;
    for state in states {
        let code = state.code();
        if code < 0 {
            return Err(Error::invalid_input(
                "states",
                "message state codes cannot be negative",
            ));
        }
        let bit = 1_u128.checked_shl(code as u32).ok_or_else(|| {
            Error::invalid_input("states", "message state code exceeds the supported mask")
        })?;
        mask |= bit;
    }
    Ok(mask.to_string())
}

fn parse_record(node: Node<'_, '_>) -> Result<MessageRecord> {
    let ordinal = soap::parse::<u32>(
        soap::required_text(node, ISDS_NS, "dmOrdinal")?,
        "dmOrdinal",
    )?;
    if ordinal == 0 {
        return Err(Error::invalid_response("dmOrdinal must be at least 1"));
    }
    let envelope = message_common::parse_envelope(node)?;
    let state = message_common::required_state(node)?;
    let attachment_size = message_common::optional_i64(node, "dmAttachmentSize")?;
    let delivery_time = message_common::optional_timestamp(node, "dmDeliveryTime")?;
    let acceptance_time = message_common::optional_timestamp(node, "dmAcceptanceTime")?;
    let message_type = node.attribute("dmType").map(str::to_owned);
    let special_message_flag = node
        .attribute("specMessFlag")
        .map(|value| soap::parse(value, "specMessFlag"))
        .transpose()?;

    Ok(MessageRecord {
        ordinal,
        envelope,
        state,
        attachment_size,
        delivery_time,
        acceptance_time,
        message_type,
        mode: message_common::optional_message_mode(node)?,
        special_message_flag,
    })
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use crate::model::{MessageMode, MessageState};

    use super::*;

    #[test]
    fn list_requests_use_explicit_paging_and_state_masks() {
        let query = MessageListQuery {
            from: Some("2026-11-01T00:00:00Z".parse::<Timestamp>().unwrap()),
            to: Some("2026-11-19T23:59:59Z".parse::<Timestamp>().unwrap()),
            organization_unit_number: Some(7),
            states: Some(vec![
                MessageState::DeliveredByFiction,
                MessageState::DeliveredByLogin,
                MessageState::Read,
            ]),
            offset: 21,
            limit: 50,
        };
        let sent = String::from_utf8(list_request(MessageListKind::Sent, &query).unwrap()).unwrap();
        assert!(sent.contains("<GetListOfSentMessages "));
        assert!(sent.contains("<dmSenderOrgUnitNum>7</dmSenderOrgUnitNum>"));
        assert!(sent.contains("<dmStatusFilter>224</dmStatusFilter>"));
        assert!(sent.contains("<dmOffset>21</dmOffset><dmLimit>50</dmLimit>"));
        assert!(sent.contains("<dmFromTime>2026-11-01T00:00:00Z</dmFromTime>"));

        let received =
            String::from_utf8(list_request(MessageListKind::Received, &query).unwrap()).unwrap();
        assert!(received.contains("<GetListOfReceivedMessages "));
        assert!(received.contains("<dmRecipientOrgUnitNum>7</dmRecipientOrgUnitNum>"));

        let all = String::from_utf8(
            list_request(MessageListKind::Sent, &MessageListQuery::default()).unwrap(),
        )
        .unwrap();
        assert!(all.contains("<dmStatusFilter>-1</dmStatusFilter>"));
        assert!(all.contains("<dmOffset>1</dmOffset><dmLimit>1000</dmLimit>"));
        assert!(all.contains("<dmFromTime xsi:nil=\"true\"/>"));
    }

    #[test]
    fn invalid_list_queries_are_rejected_locally() {
        let zero_offset = MessageListQuery {
            offset: 0,
            ..MessageListQuery::default()
        };
        assert!(list_request(MessageListKind::Sent, &zero_offset).is_err());

        let zero_limit = MessageListQuery {
            limit: 0,
            ..MessageListQuery::default()
        };
        assert!(list_request(MessageListKind::Sent, &zero_limit).is_err());

        let negative_state = MessageListQuery {
            states: Some(vec![MessageState::Unknown(-1)]),
            ..MessageListQuery::default()
        };
        assert!(list_request(MessageListKind::Sent, &negative_state).is_err());
    }

    #[test]
    fn parses_a_page_and_preserves_unknown_states() {
        let xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><GetListOfReceivedMessagesResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmRecords><dmRecord dmType="O" dmVODZ="false" specMessFlag="4">
              <dmOrdinal>1</dmOrdinal><dmID>00042</dmID><dbIDSender>egtk37z</dbIDSender>
              <dmSenderType>31</dmSenderType><dbIDRecipient>n94k37v</dbIDRecipient>
              <dmAnnotation>Synthetic</dmAnnotation><dmMessageStatus>42</dmMessageStatus>
              <dmAttachmentSize>12</dmAttachmentSize>
              <dmDeliveryTime>2026-11-19T12:00:00+01:00</dmDeliveryTime>
              <dmAcceptanceTime>2026-11-19T12:01:00+01:00</dmAcceptanceTime>
            </dmRecord></dmRecords>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </GetListOfReceivedMessagesResponse></soap:Body></soap:Envelope>"#;
        let query = MessageListQuery {
            offset: 11,
            limit: 20,
            ..MessageListQuery::default()
        };
        let page = soap::decode(xml, "GetListOfReceivedMessagesResponse", |response| {
            decode_list(response, &query)
        })
        .unwrap();
        assert_eq!(page.offset, 11);
        assert_eq!(page.limit, 20);
        assert_eq!(page.records.len(), 1);
        let record = &page.records[0];
        assert_eq!(record.state, MessageState::Unknown(42));
        assert_eq!(record.envelope.id.as_str(), "00042");
        assert_eq!(record.mode, Some(MessageMode::Standard));
        assert_eq!(record.special_message_flag, Some(4));
    }

    #[test]
    fn parses_standalone_envelope_metadata() {
        let xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MessageEnvelopeDownloadResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmReturnedMessageEnvelope dmType="E" dmVODZ="true" attsNum="7" specMessFlag="2">
              <dmDm><dmID>00042</dmID><dbIDSender>egtk37z</dbIDSender>
                <dmSenderType>31</dmSenderType><dmAnnotation>Envelope only</dmAnnotation></dmDm>
              <dmHash algorithm="SHA-256">AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmDeliveryTime>2026-11-19T12:00:00+01:00</dmDeliveryTime>
              <dmAcceptanceTime>2026-11-19T12:01:00+01:00</dmAcceptanceTime>
              <dmMessageStatus>6</dmMessageStatus><dmAttachmentSize>128</dmAttachmentSize>
            </dmReturnedMessageEnvelope>
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </MessageEnvelopeDownloadResponse></soap:Body></soap:Envelope>"#;
        let details = soap::decode(
            xml,
            "MessageEnvelopeDownloadResponse",
            decode_envelope_download,
        )
        .unwrap();
        assert_eq!(details.envelope.id.as_str(), "00042");
        assert_eq!(details.message_type.as_deref(), Some("E"));
        assert_eq!(details.mode, Some(MessageMode::Large));
        assert_eq!(details.attachment_count, Some(7));
        assert_eq!(details.special_message_flag, Some(2));
        assert_eq!(details.qualified_timestamp.as_ref(), &[4, 5]);
    }

    #[test]
    fn parses_delivery_events_and_mark_read_status() {
        let delivery = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><GetDeliveryInfoResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmDelivery><dmDm><dmID>00042</dmID><dmSenderType>31</dmSenderType>
              <dmAnnotation>Synthetic</dmAnnotation></dmDm>
              <dmHash algorithm="SHA-256">AAEC/f7/</dmHash><dmQTimestamp>BAU=</dmQTimestamp>
              <dmMessageStatus>7</dmMessageStatus><dmEvents>
                <dmEvent><dmEventTime>2026-11-19T12:01:00+01:00</dmEventTime>
                  <dmEventDescr>Delivered</dmEventDescr></dmEvent>
              </dmEvents></dmDelivery>
          </GetDeliveryInfoResponse></soap:Body></soap:Envelope>"#;
        let info = soap::decode(delivery, "GetDeliveryInfoResponse", decode_delivery_info).unwrap();
        assert_eq!(info.state, MessageState::Read);
        assert_eq!(info.qualified_timestamp.as_ref(), &[4, 5]);
        assert_eq!(info.events.len(), 1);
        assert_eq!(info.events[0].description.as_deref(), Some("Delivered"));

        let mark = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
          <soap:Body><MarkMessageAsDownloadedResponse xmlns="http://isds.czechpoint.cz/v20">
            <dmStatus><dmStatusCode>0000</dmStatusCode><dmStatusMessage>OK</dmStatusMessage></dmStatus>
          </MarkMessageAsDownloadedResponse></soap:Body></soap:Envelope>"#;
        soap::decode(
            mark,
            "MarkMessageAsDownloadedResponse",
            decode_mark_downloaded,
        )
        .unwrap();
    }
}
