//! Displays the authenticated databox's current credit and recent history.

use std::error::Error;
use std::io;

use isds::CreditQuery;
use jiff::{Span, ZonedDateTime};

mod common;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let client = common::client()?;
    let owner = client.owner_info().await?;
    let box_id = owner.id.ok_or_else(|| {
        io::Error::other("the authenticated owner response did not contain a databox ID")
    })?;

    let current = client
        .credit_info(&CreditQuery::current(box_id.clone()))
        .await?;
    println!("current credit: {:?} haléře", current.current_credit);

    // History dates are Czech-local civil dates. Tomorrow includes today's events.
    let today = ZonedDateTime::now().date();
    let from = today.checked_sub(Span::new().days(7))?;
    let to = today.checked_add(Span::new().days(1))?;
    let history = client
        .credit_info(&CreditQuery::history(box_id, from, to)?)
        .await?;

    for event in history.events {
        println!(
            "{}: type={}, change={} haléře, balance={} haléře",
            event.time, event.event_type, event.credit_change, event.credit_after
        );
    }

    Ok(())
}
