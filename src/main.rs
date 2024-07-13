use argh::FromArgs;
use color_eyre::eyre::Result;
use dotenvy::dotenv;
use once_cell::unsync::OnceCell;
use rand::{thread_rng, Rng};
use reqwest::{blocking::Client, header::HeaderMap};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, thread, time::Duration};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/115.0";

#[derive(FromArgs)]
/// Manager for subscriptions of deleted accounts on BiliBili
struct Args {
    /// path to existing data for analysis
    #[argh(option)]
    data: Option<PathBuf>,

    /// unsubscribe deleted accounts and export a manifest
    #[argh(switch)]
    unsub: bool,
}

#[derive(Deserialize)]
struct Config {
    sessdata: String,
    vmid: String,
    csrf_token: String
}

#[derive(Debug, Deserialize, Serialize)]
struct Relation {
    mid: u64,
    mtime: u64,
    uname: String,
}

#[derive(Deserialize, Serialize)]
struct Data {
    list: Vec<Relation>,
    total: u16,
}

#[derive(Deserialize)]
struct FollowingsResponse {
    data: Data,
}

#[derive(Deserialize)]
struct UnsubResponse {
    code: i32,
    message: String
}

fn init_client(config: &Config) -> Result<Client> {
    let Config { sessdata, vmid , .. } = config;

    let mut headers = HeaderMap::new();

    headers.insert("Cookie", format!("SESSDATA={}", sessdata).parse()?);
    headers.insert(
        "Referer",
        format!("https://space.bilibili.com/{}/fans/follow", vmid).parse()?,
    );

    let client = Client::builder()
        .user_agent(UA)
        .default_headers(headers)
        .build()
        .unwrap();

    Ok(client)
}

fn collect_and_export(client_cell: &OnceCell<Client>, config: &Config) -> Result<Data> {
    let client = client_cell.get_or_try_init(|| init_client(config))?;

    let mut result: Vec<Vec<Relation>> = vec![];

    let mut collect_followings = |page: u8| -> Result<u16> {
        println!("Collecting page {}", page);
        let res = client
            .get("https://api.bilibili.com/x/relation/followings")
            .query(&[("vmid", &config.vmid), ("pn", &page.to_string())])
            .send()?;

        let FollowingsResponse { data } = res.json()?;

        result.push(data.list);

        Ok(data.total)
    };

    let total = collect_followings(1)?;

    for pn in 2..=total.div_ceil(50) {
        collect_followings(pn as u8)?;
        thread::sleep(Duration::from_millis(thread_rng().gen_range(800..=1500)))
    }

    let result: Vec<Relation> = result.into_iter().flatten().collect();

    let data = Data {
        total,
        list: result,
    };

    fs::write("data.json", serde_json::to_string_pretty(&data)?)?;
    Ok(data)
}

fn main() -> Result<()> {
    dotenv()?;

    let config = envy::from_env::<Config>()?;
    let args: Args = argh::from_env();

    let client_cell: OnceCell<Client> = OnceCell::new();
    let data: Data;

    if let Some(path) = args.data {
        data = serde_json::from_slice(fs::read(path.as_path())?.as_slice())?;
    } else {
        data = collect_and_export(&client_cell, &config)?;
    };

    let deleted: Vec<Relation> = data
        .list
        .into_iter()
        .filter(|relation| relation.uname == "账号已注销")
        .collect();

    println!("Subscribed accounts: {}", data.total);
    println!("Deleted accounts: {}", deleted.len());

    if !args.unsub {
        return Ok(());
    }

    let client = client_cell.get_or_try_init(|| init_client(&config))?;

    for mid in deleted.iter().map(|relation| relation.mid) {
        let res = client
            .post("https://api.bilibili.com/x/relation/modify")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!(
                "fid={}&act=2&re_src=11&csrf={}",
                mid, config.csrf_token
            ))
            .send()?;

        let UnsubResponse { code, message } = res.json()?;

        if code == 0 {
            println!("Unsub: {}", mid)
        } else {
            println!("Unsub failed: {}", message)
        }

        thread::sleep(Duration::from_millis(thread_rng().gen_range(800..=1500)))
    }

    fs::write("deleted.json", serde_json::to_string_pretty(&deleted)?)?;
    println!("Done");

    Ok(())
}
