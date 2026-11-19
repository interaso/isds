//! Checks an exact databox ID and optionally searches by firm name.

use std::error::Error;
use std::{env, io};

use isds::{DataBox, DataBoxId};

mod common;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let raw_id = arguments.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: lookup <databox-id> [firm-name]",
        )
    })?;
    let firm_name = arguments.next();
    let client = common::client()?;

    let id = DataBoxId::new(raw_id)?;
    let state = client.check_data_box(&id).await?;
    println!(
        "databox {id}: state={}, active={}",
        state.code(),
        state.is_active()
    );

    match client.find_data_box(&id).await? {
        Some(data_box) => {
            if let Some(box_type) = &data_box.box_type {
                println!("type={box_type}");
            }
            println!("open_addressing={:?}", data_box.open_addressing);
        }
        None => println!("databox {id} was not found"),
    }

    // Optional criteria-based search using the same client.
    if let Some(firm_name) = firm_name {
        let result = client
            .find_data_boxes(&DataBox {
                firm_name: Some(firm_name),
                ..DataBox::default()
            })
            .await?;
        println!(
            "matches={}, truncated={}, postal_delivery_restricted={}",
            result.boxes.len(),
            result.truncated,
            result.postal_delivery_restricted
        );
        for data_box in result.boxes {
            if let Some(id) = data_box.id {
                println!("match: {id}");
            }
        }
    }

    Ok(())
}
