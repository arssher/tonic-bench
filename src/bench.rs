pub mod neon_broker {
    tonic::include_proto!("neon_broker");
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use neon_broker::neon_broker_client::NeonBrokerClient;
use neon_broker::{SafekeeperTimelineInfo, SubscribeSafekeeperInfoRequest};
use tokio::time::{self, sleep};
use tonic::transport::Channel;
use tonic::Request;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Number of publishers
    #[clap(short = 'p', long, value_parser, default_value_t = 1)]
    num_pubs: u32,
    /// Number of subscribers
    #[clap(short = 's', long, value_parser, default_value_t = 1)]
    num_subs: u32,
}

async fn progress_reporter(counter: Arc<AtomicU64>) {
    let mut interval = time::interval(Duration::from_millis(1000));
    let mut c_old = counter.load(Ordering::Relaxed);
    let mut started_at = None;
    let mut skipped: u64 = 0;
    loop {
        interval.tick().await;
        let c_new = counter.load(Ordering::Relaxed);
        if c_new > 0 && started_at.is_none() {
            started_at = Some(Instant::now());
            skipped = c_new;
        }
        let avg_rps = started_at.map(|s| {
            let dur = s.elapsed();
            let dur_secs = dur.as_secs() as f64 + (dur.subsec_millis() as f64) / 1000.0;
            let avg_rps = (c_new - skipped) as f64 / dur_secs;
            (dur, avg_rps)
        });
        println!(
            "rps {}, total {}, duration, avg rps {:?}",
            c_new - c_old,
            c_new,
            avg_rps
        );
        c_old = c_new;
    }
}

async fn subscribe(mut client: NeonBrokerClient<Channel>, counter: Arc<AtomicU64>, _i: u32) {
    let request = SubscribeSafekeeperInfoRequest {};
    let mut stream = client
        .subscribe_safekeeper_info(request)
        .await
        .unwrap()
        .into_inner();

    while let Some(_feature) = stream.message().await.unwrap() {
        counter.fetch_add(1, Ordering::Relaxed);
        // println!("info = {:?}, client {}", _feature, i);
    }
}

async fn publish(mut client: NeonBrokerClient<Channel>) {
    let mut counter: u64 = 0;

    // create stream producing new values
    let outbound = async_stream::stream! {
        loop {
            let info = SafekeeperTimelineInfo {
                last_log_term: 0,
                flush_lsn: counter,
                commit_lsn: 2,
                backup_lsn: 3,
                remote_consistent_lsn: 4,
                peer_horizon_lsn: 5,
                safekeeper_connstr: "zenith-1-sk-1.local:7676".to_owned(),
            };
            counter += 1;
            // println!("sending info = {:?}", info);
            yield info;
        }
    };
    let _response = client
        .publish_safekeeper_info(Request::new(outbound))
        .await
        .unwrap();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let counter = Arc::new(AtomicU64::new(0));
    let h = tokio::spawn(progress_reporter(counter.clone()));

    let client = NeonBrokerClient::connect("http://[::1]:50051").await?;

    for i in 0..args.num_subs {
        tokio::spawn(subscribe(client.clone(), counter.clone(), i));
    }
    // let subscribers get registred
    sleep(Duration::from_millis(1000)).await;
    for _ in 0..args.num_pubs {
        tokio::spawn(publish(client.clone()));
    }

    h.await?;
    Ok(())
}
