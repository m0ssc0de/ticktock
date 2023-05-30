use prometheus::{IntGaugeVec, Opts};

use core::num;
use futures::stream::{FuturesUnordered, PollNext};
use futures::StreamExt;
use reqwest::{header, Client, Url};
use serde_json::{json, Value};
use std::{env, process::Command, time::Duration};
use structopt::StructOpt;
use tokio::time;

#[derive(Debug)]
enum MError {
    REQWEST(reqwest::Error),
    PARSEJSON(String, String),
    PARSEHEX(num::ParseIntError),
}
impl From<reqwest::Error> for MError {
    fn from(error: reqwest::Error) -> Self {
        MError::REQWEST(error)
    }
}
impl From<num::ParseIntError> for MError {
    fn from(error: num::ParseIntError) -> Self {
        MError::PARSEHEX(error)
    }
}

#[derive(Debug, StructOpt)]
struct Opt {
    /// List of URL-label pairs
    #[structopt(short, long, parse(try_from_str = parse_gq_target))]
    gq_target: Vec<(String, Url)>,
    #[structopt(short, long, parse(try_from_str = parse_node_target))]
    node_target: Vec<(String, String, i64, Url)>,
    job_target: String,
}

fn parse_gq_target(s: &str) -> Result<(String, Url), &'static str> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        Err("must be a pair {TARGET LABEL},{URL}")
    } else {
        Ok((parts[0].into(), Url::parse(parts[1].into()).unwrap()))
    }
}

fn parse_node_target(s: &str) -> Result<(String, String, i64, Url), &'static str> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        Err("No enough info for node rpc target {TARGET LABEL},{BLOCK LABEL},{BLOCK OFFSET},{URL}")
    } else {
        Ok((
            parts[0].into(),
            parts[1].into(),
            parts[2].parse::<i64>().unwrap(),
            Url::parse(parts[3].into()).unwrap(),
        ))
    }
}

#[tokio::main]
async fn main() -> Result<(), MError> {
    let opt = Opt::from_args();

    let client = reqwest::Client::new();
    let interval_secs = env::var("INTERVAL_SEC")
        .expect("INTERVAL_SEC must be set")
        .parse()
        .unwrap();

    let bh_opts =
        Opts::new("last_block_height", "the last block height").const_label("network", "polygon");
    // .const_label("app", "dictionary");
    // .const_label("component", "endpoint");
    let last_block_height_vec =
        IntGaugeVec::new(bh_opts, &["app", "component", "source_label"]).unwrap();
    prometheus::register(Box::new(last_block_height_vec.clone())).unwrap();
    prometheus_exporter::start("0.0.0.0:9184".parse().expect("failed to parse binding"))
        .expect("failed to start prometheus exporter");

    let mut interval = time::interval(Duration::from_secs(interval_secs));
    loop {
        let mut node_futures = FuturesUnordered::new();
        let mut gq_futures = FuturesUnordered::new();
        for (node_label, node_block_lable, node_offset, node_url) in &opt.node_target {
            node_futures.push(node_rpc(
                node_label.to_owned(),
                &node_block_lable,
                node_offset.to_owned(),
                node_url,
                &client,
            ));
        }
        for (gq_label, gq_url) in &opt.gq_target {
            gq_futures.push(graphql_rpc(gq_label.to_owned(), gq_url, &client))
        }
        if !opt.job_target.is_empty() {
            println!("{}", opt.job_target);
            last_block_height_vec
                .with_label_values(&["datasource", "fetching-job", "last-uploaded"])
                .set(uploaded(&opt.job_target))
        }
        while let Some(result) = gq_futures.next().await {
            match result {
                Ok((label, number)) => {
                    last_block_height_vec
                        .with_label_values(&["dictionary", "query-endpoint", &label])
                        .set(number);
                }
                Err(e) => {
                    eprintln!("Failed get graphql response , Error {:#?}", e)
                }
            }
        }
        while let Some(result) = node_futures.next().await {
            match result {
                Ok((label, number)) => {
                    last_block_height_vec
                        .with_label_values(&["node", "rpc-endpoint", &label])
                        .set(number);
                }
                Err(e) => {
                    eprintln!("Failed get node rpc response , Error {:#?}", e)
                }
            }
        }
        interval.tick().await;
    }
}

fn uploaded(cfg_name: &str) -> i64 {
    let output = Command::new("/bin/sh").arg(format!("kubectl -n `cat /var/run/secrets/kubernetes.io/serviceaccount/namespace` get configmap {} -o yaml | grep ED: | cut -d '\"' -f 2", cfg_name)).output().unwrap();
    let errstr = String::from_utf8_lossy(&output.stderr);
    if !errstr.is_empty() {
        println!("Command failed to execute: {}", errstr);
    }
    let outstr = String::from_utf8_lossy(&output.stdout);
    if !outstr.is_empty() {
        println!("Command output: {}", outstr);
    }
    // let number: i64 = outstr.parse().unwrap();
    if let Ok(number) = outstr.parse::<i64>() {
        return number;
    } else {
        return 0;
    }
}

async fn graphql_rpc(
    label: String,
    url: &Url,
    http_client: &Client,
) -> Result<(String, i64), MError> {
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
    let response = http_client
        .post(url.to_owned())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, body.to_string().len().to_string())
        .body(body.to_string())
        .send()
        .await?;

    match response.error_for_status() {
        Err(e) => Err(MError::REQWEST(e)),
        Ok(response) => {
            let current_response: Value = response.json().await?;
            match current_response["data"]["_metadata"]["lastProcessedHeight"].as_i64() {
                Some(n) => Ok((label, n)),
                None => Err(MError::PARSEJSON(
                    ".data._metadata.lastProcessedHeight".to_string(),
                    current_response.to_string(),
                )),
            }
        }
    }
}

async fn node_rpc(
    label: String,
    tag: &str,
    offset: i64,
    url: &Url,
    http_client: &Client,
) -> Result<(String, i64), MError> {
    let body = json!({
        "jsonrpc":"2.0",
        "method":"eth_getBlockByNumber",
        "params":[tag, false],
        "id": 1
    });
    let response = http_client
        .post(url.to_owned())
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, body.to_string().len().to_string())
        .body(body.to_string())
        .send()
        .await?;

    match response.error_for_status() {
        Ok(r) => {
            let current_response: Value = r.json().await?;
            match current_response["result"]["number"].as_str() {
                Some(hex) => {
                    return Ok((
                        label,
                        i64::from_str_radix(&hex[2..], 16).map_err(|e| {
                            println!("HEX {}", hex);
                            e
                        })? - offset,
                    ));
                }
                None => {
                    return Err(MError::PARSEJSON(
                        ".result.number".to_string(),
                        current_response.to_string(),
                    ))
                }
            }
        }
        Err(e) => Err(MError::REQWEST(e)),
    }
}
