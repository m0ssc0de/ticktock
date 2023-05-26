use prometheus::{IntGaugeVec, Opts};

use reqwest::{Error, header};
use serde_json::{json, Value};
use std::{time::Duration, env};
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let client = reqwest::Client::new();
    let interval_secs = env::var("INTERVAL_SEC").expect("INTERVAL_SEC must be set").parse().unwrap();

    let bh_opts = Opts::new("last_block_height", "the last block height")
    .const_label("network", "polygon")
    .const_label("app", "dictionary")
    .const_label("component", "endpoint"); 
    let last_block_height_vec = IntGaugeVec::new(bh_opts, &[]).unwrap();
    prometheus::register(Box::new(last_block_height_vec.clone())).unwrap();
    prometheus_exporter::start("0.0.0.0:9184".parse().expect( "failed to parse binding",)).expect("failed to start prometheus exporter");

    let mut interval = time::interval(Duration::from_secs(interval_secs));

    loop {
        interval.tick().await;

        let graphql_query = r#"
        query MyQuery {
            _metadata {
              lastProcessedHeight
            }
          }
        "#;
        let body = json!({
            "query": graphql_query,
        });
        let response = client.post("https://gx.api.subquery.network/sq/subquery/polygon-dictionary-2")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CONTENT_LENGTH, body.to_string().len().to_string())
            .body(body.to_string()).send().await?;

        if response.status().is_success() {
            let current_response: Value = response.json().await?;
            let number = current_response["data"]["_metadata"]["lastProcessedHeight"].as_i64().unwrap(); // replace "number" with the actual field name in the JSON response

            last_block_height_vec.with_label_values(&[]).set(number);
            println!("block height {}", number);
        } else {
            eprintln!("Error: {}", response.status());
        }
    }
}
