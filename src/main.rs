use prometheus::{IntGaugeVec, Opts};

use core::num;
use futures::stream::{FuturesUnordered, PollNext};
use futures::StreamExt;
use reqwest::{header, Client, Url};
// use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::{env, fs::File, path::PathBuf, process::Command, time::Duration};
use structopt::StructOpt;
use tokio::time::{self, Interval};

#[derive(Debug, Clone)]
enum MError {
    REQWEST(String),
    PARSEJSON(String, String),
    PARSEHEX(num::ParseIntError),
}
impl From<reqwest::Error> for MError {
    fn from(error: reqwest::Error) -> Self {
        MError::REQWEST(error.to_string())
    }
}
impl From<num::ParseIntError> for MError {
    fn from(error: num::ParseIntError) -> Self {
        MError::PARSEHEX(error)
    }
}

use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
// #[serde(rename_all = "lowercase")]
pub enum RPCType {
    ChainNode,
    DictEntry,
    RedisKey,
    CFGMAPKey,
    // ... add other variants here ...
}

#[derive(Debug, PartialEq, Serialize, Deserialize, Clone)]
struct Target {
    #[serde(rename = "type")]
    type_: RPCType,
    uri: String,
    label_network: String,
    label_app: String,
    label_component: String,
    label_source_label: String,
    block_label: Option<String>,
    block_label_offset: Option<i64>,
    result: Option<i64>,
    redis_host: Option<String>,
    redis_passwd: Option<String>,
    redis_key: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Config {
    targets: Vec<Target>,
    interval: u64,
}

#[derive(Debug, StructOpt)]
struct Opt {
    // /// List of URL-label pairs
    // #[structopt(short, long, parse(try_from_str = parse_gq_target))]
    // gq_target: Vec<(String, Url)>,
    // #[structopt(short, long, parse(try_from_str = parse_node_target))]
    // node_target: Vec<(String, String, i64, Url)>,
    // #[structopt(short, long)]
    // job_target: String,
    // #[structopt(short, long)]
    // redis_target: bool,
    #[structopt(parse(from_os_str))]
    config: PathBuf,
}

#[warn(dead_code)]
fn parse_gq_target(s: &str) -> Result<(String, Url), &'static str> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        Err("must be a pair {TARGET LABEL},{URL}")
    } else {
        Ok((parts[0].into(), Url::parse(parts[1].into()).unwrap()))
    }
}

#[warn(dead_code)]
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

    let mut file = File::open(opt.config).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();

    let cfg: Config = serde_yaml::from_str(&contents).unwrap();

    let client = reqwest::Client::new();
    let interval_secs = cfg.interval;

    let bh_opts = Opts::new("last_block_height", "the last block height");
    let last_block_height_vec =
        IntGaugeVec::new(bh_opts, &["network", "app", "component", "source_label"]).unwrap();
    prometheus::register(Box::new(last_block_height_vec.clone())).unwrap();
    prometheus_exporter::start("0.0.0.0:9184".parse().expect("failed to parse binding"))
        .expect("failed to start prometheus exporter");

    let mut interval = time::interval(Duration::from_secs(interval_secs));
    loop {
        let mut all_futures = FuturesUnordered::new();
        for t in &cfg.targets {
            all_futures.push(emit_target(t.clone(), &client))
        }
        while let Some(result) = all_futures.next().await {
            match result {
                Ok(target) => {
                    if let Some(n) = target.result {
                        last_block_height_vec
                            .with_label_values(&[
                                &target.label_network,
                                &target.label_app,
                                &target.label_component,
                                &target.label_source_label,
                            ])
                            .set(n);
                    }
                }
                Err(e) => {
                    eprintln!("Failed get response , Error {:#?}", e)
                }
            }
        }
        interval.tick().await;
    }
}

fn uploaded(cfg_name: &str) -> i64 {
    let output = Command::new("/bin/sh").arg("-c").arg(format!("kubectl -n `cat /var/run/secrets/kubernetes.io/serviceaccount/namespace` get configmap {} -o yaml | grep ED: | cut -d '\"' -f 2", cfg_name)).output().unwrap();
    let errstr = String::from_utf8_lossy(&output.stderr);
    if !errstr.is_empty() {
        println!("Command failed to execute: {}", errstr);
    }
    let outstr = String::from_utf8_lossy(&output.stdout);
    if !outstr.is_empty() {
        println!("Command output: |{}|", outstr);
    }
    // let number: i64 = outstr.parse().unwrap();
    if let Ok(number) = outstr.trim().parse::<i64>() {
        return number;
    } else {
        return 0;
    }
}

fn uploaded_in_redis(redis_host: &str, redis_passwd: &str, redis_key: &str) -> i64 {
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            "redis-cli -h {} -a {} get {}",
            redis_host, redis_passwd, redis_key
        ))
        .output()
        .unwrap();
    let errstr = String::from_utf8_lossy(&output.stderr);
    if !errstr.is_empty() {
        println!("Command failed to execute: {}", errstr);
    }
    let outstr = String::from_utf8_lossy(&output.stdout);
    if !outstr.is_empty() {
        println!("Command output: |{}|", outstr);
    }
    // let number: i64 = outstr.parse().unwrap();
    if let Ok(number) = outstr.trim().replace("\"", "").parse::<i64>() {
        return number;
    } else {
        return 0;
    }
}

async fn emit_target(mut target: Target, http_client: &Client) -> Result<Target, MError> {
    match target.type_ {
        RPCType::ChainNode => {
            let (_l, n) = node_rpc(
                "sss".to_owned(),
                &target.block_label.to_owned().unwrap(),
                target.block_label_offset.unwrap(),
                &Url::parse(&target.uri).unwrap(),
                http_client,
            )
            .await?;
            target.result = Some(n);
            Ok(target)
        }
        RPCType::DictEntry => {
            let (_l, n) = graphql_rpc(
                "label".to_owned(),
                &Url::parse(&target.uri).unwrap(),
                http_client,
            )
            .await?;
            target.result = Some(n);
            Ok(target)
        }
        RPCType::CFGMAPKey => {
            target.result = Some(uploaded(&target.uri));
            Ok(target)
        }
        RPCType::RedisKey => {
            target.result = Some(uploaded_in_redis(
                &target.redis_host.to_owned().unwrap(),
                &target.redis_passwd.to_owned().unwrap(),
                &target.redis_key.to_owned().unwrap(),
            ));
            Ok(target)
        } // _ => Ok(target),
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
        Err(e) => Err(MError::REQWEST(e.to_string())),
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
        Err(e) => Err(MError::REQWEST(e.to_string())),
    }
}
